use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use uuid::Uuid;

const DEFAULT_DB_PATH: &str = "docs/doc-pointer-db.json";
const DOC_POINTER_NAMESPACE: Uuid = uuid::uuid!("64e9408c-37a7-5f92-8893-f149cbde01c0");
const TOKEN_RANGES: [(u32, u32); 4] = [
    (0x10980, 0x1099F), // Meroitic Hieroglyphs
    (0x13000, 0x1342F), // Egyptian Hieroglyphs
    (0x13460, 0x143FF), // Egyptian Hieroglyphs Extended-A; skips U+13430..U+1345F controls
    (0x14400, 0x1467F), // Anatolian Hieroglyphs
];
const TOKEN_SIZE: u128 = token_alphabet_size();
const TOKEN_LENGTH: usize = 4;

const fn token_alphabet_size() -> u128 {
    let mut total = 0u128;
    let mut index = 0usize;
    while index < TOKEN_RANGES.len() {
        let (start, end) = TOKEN_RANGES[index];
        total += (end - start + 1) as u128;
        index += 1;
    }
    total
}

#[derive(Debug, Clone)]
struct Pointer {
    uuid: Option<Uuid>,
    kind: Option<String>,
    code: String,
    path: String,
    line: usize,
    name: String,
    description: String,
    locations: Vec<Location>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Location {
    file_path: String,
    line: usize,
    end_line: Option<usize>,
}

#[derive(Debug)]
struct ScanOptions {
    root: PathBuf,
    db: String,
    write: bool,
    check: bool,
    install_hook: bool,
    filter: ScanFilter,
}

/// Root-relative subtree scoping for scans. Empty `include` means the whole root.
/// A directory is entered when it could contain an included path; a file matches
/// when it sits under an included prefix and under no excluded prefix.
/// `--exclude` accepts plain prefixes (`utilities/vendored`) or globs relative
/// to the root (`**/generated/**`, `src/*.gen.ex`). Default excludes always
/// apply: worktree and staging duplicates hold no stores of their own and
/// previously produced thousands of phantom records in the monorepo.
#[derive(Debug, Default, Clone)]
struct ScanFilter {
    include: Vec<PathBuf>,
    exclude: Vec<PathBuf>,
}

/// Default-excluded path segments: `.claude/worktrees` and the legacy worktree
/// placements (`.worktrees/`, `<repo>.worktrees/`) plus nested `staging/` trees.
fn default_excluded(rel: &Path) -> bool {
    let names: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    names.iter().enumerate().any(|(index, name)| {
        name == "staging"
            || name == ".worktrees"
            || name.ends_with(".worktrees")
            || (name == "worktrees" && index > 0 && names[index - 1] == ".claude")
    })
}

/// A pattern without glob metacharacters keeps the historical prefix semantics;
/// patterns with `*`/`?` are matched as root-relative globs.
fn pattern_matches(pattern: &str, rel: &str) -> bool {
    if !pattern.contains('*') && !pattern.contains('?') {
        return rel == pattern || rel.starts_with(&format!("{pattern}/"));
    }
    glob_match(pattern, rel)
}

fn glob_match(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    glob_segments(&pat, &seg)
}

fn glob_segments(pat: &[&str], seg: &[&str]) -> bool {
    match pat.split_first() {
        None => seg.is_empty(),
        // `**` spans zero or more whole path segments.
        Some((&"**", rest)) => (0..=seg.len()).any(|skip| glob_segments(rest, &seg[skip..])),
        Some((head, rest)) => match seg.split_first() {
            Some((path_head, path_rest)) => {
                segment_match(head, path_head) && glob_segments(rest, path_rest)
            }
            None => false,
        },
    }
}

fn segment_match(pat: &str, text: &str) -> bool {
    let pat: Vec<char> = pat.chars().collect();
    let text: Vec<char> = text.chars().collect();
    segment_match_chars(&pat, &text)
}

fn segment_match_chars(pat: &[char], text: &[char]) -> bool {
    match pat.split_first() {
        None => text.is_empty(),
        Some(('*', rest)) => (0..=text.len()).any(|skip| segment_match_chars(rest, &text[skip..])),
        Some((head, rest)) => match text.split_first() {
            Some((text_head, text_rest)) if head == text_head || *head == '?' => {
                segment_match_chars(rest, text_rest)
            }
            _ => false,
        },
    }
}

impl ScanFilter {
    fn excluded(&self, rel: &Path) -> bool {
        if default_excluded(rel) {
            return true;
        }
        let rel_str = rel.to_string_lossy();
        self.exclude
            .iter()
            .any(|e| pattern_matches(&e.to_string_lossy(), &rel_str))
    }

    fn allows_dir(&self, rel: &Path) -> bool {
        if self.excluded(rel) {
            return false;
        }
        if self.include.is_empty() {
            return true;
        }
        self.include
            .iter()
            .any(|i| rel.starts_with(i) || i.starts_with(rel))
    }

    fn allows_file(&self, rel: &Path) -> bool {
        if self.excluded(rel) {
            return false;
        }
        if self.include.is_empty() {
            return true;
        }
        self.include.iter().any(|i| rel.starts_with(i))
    }
}

#[derive(Debug)]
struct AnnotateOptions {
    root: PathBuf,
    db: String,
    write: bool,
    include_exs: bool,
    force_remint: bool,
    filter: ScanFilter,
}

#[derive(Debug)]
struct Uuid5Options {
    name: Option<String>,
    root: PathBuf,
    db: String,
    namespace: String,
    salt: String,
    format: PointerFormat,
    kind: String,
    description: String,
    no_clipboard: bool,
}

#[derive(Debug, Clone, Copy)]
enum PointerFormat {
    Marker,
    Code,
    Declaration,
    Deeplink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerKind {
    File,
    Module,
    Contract,
    Component,
    Function,
    Logic,
    Diagram,
}

impl MarkerKind {
    fn emoji(self) -> &'static str {
        match self {
            Self::File => "📁",
            Self::Module => "📦",
            Self::Contract => "🔌",
            Self::Component => "🧩",
            Self::Function => "🔧",
            Self::Logic => "🔀",
            Self::Diagram => "📐",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "file" | "📁" => Some(Self::File),
            "module" | "class" | "struct" | "📦" => Some(Self::Module),
            "contract" | "interface" | "protocol" | "behavior" | "behaviour" | "🔌" => {
                Some(Self::Contract)
            }
            "component" | "🧩" => Some(Self::Component),
            "function" | "🔧" => Some(Self::Function),
            "logic" | "🔀" => Some(Self::Logic),
            "diagram" | "mermaid" | "plantuml" | "📐" => Some(Self::Diagram),
            _ => None,
        }
    }

    fn closable(self) -> bool {
        matches!(self, Self::Component | Self::Logic | Self::Diagram)
    }

    /// Canonical string kind stored in .meta/pointers.yaml. Emoji markers map
    /// to their canonical default; fine-grained siblings (class, struct,
    /// protocol, behaviour) are only reachable through explicit string kinds.
    fn canonical(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Module => "module",
            Self::Contract => "interface",
            Self::Component => "component",
            Self::Function => "function",
            Self::Logic => "logic",
            Self::Diagram => "diagram",
        }
    }
}

/// Canonical string kinds stored in .meta/pointers.yaml. Accepts the
/// fine-grained string space plus legacy scope emoji (and historical CLI
/// aliases), normalizing everything to a canonical string.
fn parse_kind_string(value: &str) -> Option<String> {
    let canonical = match value {
        "file" | "📁" => "file",
        "module" | "📦" => "module",
        "class" => "class",
        "struct" => "struct",
        "interface" | "contract" | "🔌" => "interface",
        "protocol" => "protocol",
        "behaviour" | "behavior" => "behaviour",
        "function" | "🔧" => "function",
        "logic" | "🔀" => "logic",
        "component" | "🧩" => "component",
        "diagram" | "mermaid" | "plantuml" | "📐" => "diagram",
        _ => return None,
    };
    Some(canonical.to_string())
}

fn emoji_for_kind(kind: &str) -> &'static str {
    match kind {
        "file" => "📁",
        "module" | "class" | "struct" => "📦",
        "interface" | "protocol" | "behaviour" | "contract" => "🔌",
        "component" => "🧩",
        "function" => "🔧",
        "logic" => "🔀",
        "diagram" | "mermaid" | "plantuml" => "📐",
        _ => "🔧",
    }
}

fn kind_is_closable(kind: &str) -> bool {
    matches!(kind, "component" | "logic" | "diagram")
}

/// A stored string kind matches a marker kind when they share the same emoji
/// scope (so a stored "class" matches a 📦 marker).
fn stored_kind_matches(stored: &Option<String>, kind: MarkerKind) -> bool {
    stored.as_deref().and_then(MarkerKind::parse) == Some(kind)
}

/// A canonical marker payload. New markers embed the 4-glyph token
/// (`MarkerPayload::Token`); the full v5 UUID (`MarkerPayload::Uuid`) stays
/// accepted everywhere markers are parsed or looked up. Tokens are derived
/// only — resolution token -> UUID goes through the store index.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MarkerPayload {
    Uuid(Uuid),
    Token(String),
}

impl MarkerPayload {
    fn as_str(&self) -> String {
        match self {
            Self::Uuid(uuid) => uuid.to_string(),
            Self::Token(code) => code.clone(),
        }
    }
}

/// Exactly four glyphs, each from the hieroglyph alphabet — the shape a token
/// payload must have before it is worth resolving against the store.
fn valid_token_glyphs(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    chars.len() == TOKEN_LENGTH
        && chars.iter().all(|&c| {
            TOKEN_RANGES
                .iter()
                .any(|&(start, end)| (start..=end).contains(&(c as u32)))
        })
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    // No arguments at all -> print help and exit 0. Previously this silently ran a
    // read-only scan, which surprised users who just wanted to know what the tool does.
    if args.is_empty() {
        print_help();
        std::process::exit(0);
    }

    // First positional token selects the subcommand (build / uuid5 / hook / help).
    // A leading option (-h/--help, or a legacy build flag like --root/--write/--check)
    // is treated as the build command so existing scripts keep working.
    let result = match args.first().map(String::as_str) {
        Some("build") => scan_command(&args[1..]),
        Some("annotate") => annotate_command(&args[1..]),
        Some("uuid5" | "new") => uuid5_command(&args[1..]),
        Some("lookup") => lookup_command(&args[1..]),
        Some("hook") => install_command(&args[1..], true),
        Some("-h" | "--help" | "help") => {
            print_help();
            return;
        }
        Some("--root" | "--db" | "--write" | "--check" | "--install-hook") => {
            scan_or_install(&args)
        }
        Some(value) => {
            eprintln!("ERROR: unknown subcommand or option: {value}\n");
            print_help();
            std::process::exit(1);
        }
        None => {
            print_help();
            return;
        }
    };

    if let Err(error) = result {
        eprintln!("ERROR: {error}");
        std::process::exit(1);
    }
}

// Legacy dispatch: the build/install-hook commands used to share one option namespace at the
// top level. `--install-hook` installs the pre-commit hook; everything else is a build scan.
fn scan_or_install(args: &[String]) -> Result<(), String> {
    if args.iter().any(|arg| arg == "--install-hook") {
        install_command(args, false)
    } else {
        scan_command(args)
    }
}

fn install_command(args: &[String], explicit: bool) -> Result<(), String> {
    let options = parse_scan_options(args)?;
    // `explicit` = reached via the `hook` subcommand (always install). The legacy
    // `--install-hook` flag also sets options.install_hook; reject ambiguous calls.
    if !explicit && !options.install_hook {
        eprintln!("ERROR: --install-hook requires the hook subcommand or the --install-hook flag");
        std::process::exit(1);
    }
    let root = absolute_path(&options.root)?;
    install_hook(&root)
}

fn scan_command(args: &[String]) -> Result<(), String> {
    let options = parse_scan_options(args)?;
    let root = absolute_path(&options.root)?;
    let db_path = legacy_db_path(&root, &options.db)?;

    let token_index = store_token_index(&root, &db_path)?;
    let (mut pointers, mut errors) =
        collect_pointers(&root, &db_path, &options.filter, &token_index)?;
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    hydrate_pointer_ids(&root, &db_path, &mut pointers)?;
    let records = reconcile_records(&root, &db_path, &pointers, &options.filter)?;
    let db_changed = if options.write || options.check {
        backend_reconcile(&root, &records, options.write)?
    } else {
        false
    };
    let expansion = expansion_index(&root, &db_path, &pointers)?;
    let (changed_links, link_errors) =
        expand_markdown_links(&root, &expansion, options.write, &options.filter)?;
    errors.extend(link_errors);

    for error in &errors {
        eprintln!("ERROR: {error}");
    }

    if options.check {
        if db_changed {
            eprintln!("ERROR: pointer stores are stale; run doc-pointers build --write.");
        }
        if !changed_links.is_empty() {
            eprintln!(
                "ERROR: markdown deeplinks need expansion in: {}",
                changed_links.join(", ")
            );
        }
        if !errors.is_empty() || db_changed || !changed_links.is_empty() {
            std::process::exit(1);
        }
    }

    if options.write {
        if db_changed {
            println!("updated .meta/pointers.yaml stores");
        }
        if !changed_links.is_empty() {
            println!(
                "expanded markdown deeplinks in: {}",
                changed_links.join(", ")
            );
        }
        if !db_changed && changed_links.is_empty() {
            println!("doc pointers already up to date");
        }
        if !errors.is_empty() {
            std::process::exit(1);
        }
    } else {
        println!("found {} doc pointer declarations", pointers.len());
    }

    if errors.is_empty() {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

fn uuid5_command(args: &[String]) -> Result<(), String> {
    let options = parse_uuid5_options(args)?;
    let root = absolute_path(&options.root)?;
    let db_path = legacy_db_path(&root, &options.db)?;
    let namespace = parse_namespace(&options.namespace)?;
    let token_index = store_token_index(&root, &db_path)?;
    let (mut pointers, errors) =
        collect_pointers(&root, &db_path, &ScanFilter::default(), &token_index)?;
    for pointer in backend_status(&root)? {
        pointers.entry(pointer.code.clone()).or_insert(pointer);
    }
    for pointer in legacy_records(&db_path)?.into_values() {
        pointers.entry(pointer.code.clone()).or_insert(pointer);
    }

    for error in errors {
        eprintln!("WARNING: {error}");
    }

    let seed = options
        .name
        .clone()
        .unwrap_or_else(|| format!("auto:{}", Uuid::new_v4()));
    let (code, uuid, uuid_name, attempt) =
        generate_uuid5_code(&seed, namespace, &options.salt, &pointers)?;
    let payload = format_pointer(
        options.format,
        &options.kind,
        &code,
        options.name.as_deref(),
        &options.description,
    )?;

    println!("uuid5: {uuid}");
    println!("uuid5-name: {uuid_name}");
    if attempt > 0 {
        println!("collision-attempt: {attempt}");
    }
    println!("code: {code}");
    println!("marker: 〚{}:{code}〛", emoji_for_kind(&options.kind));
    if kind_is_closable(&options.kind) {
        println!("closing: 〚/{}:{code}〛", emoji_for_kind(&options.kind));
    }
    println!("clipboard: {payload}");

    if !options.no_clipboard {
        copy_to_clipboard(&payload)?;
        println!("copied to clipboard");
    }

    Ok(())
}

fn lookup_command(args: &[String]) -> Result<(), String> {
    let mut root = PathBuf::from(".");
    let mut context = 2usize;
    let mut query = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--root" => {
                index += 1;
                root = PathBuf::from(expect_value(args, index, "--root")?);
            }
            "--context" => {
                index += 1;
                context = expect_value(args, index, "--context")?
                    .parse::<usize>()
                    .map_err(|_| "--context must be a nonnegative integer".to_string())?;
                if context > 20 {
                    return Err("--context must be at most 20".to_string());
                }
            }
            "--help" | "-h" => {
                println!("usage: doc-pointers lookup UUID|token|emoji:UUID|emoji:token|〚emoji:UUID〛|〚emoji:token〛 [--root ROOT] [--context N]");
                return Ok(());
            }
            value if value.starts_with('-') => return Err(format!("unknown option: {value}")),
            value => {
                if query.replace(value.to_string()).is_some() {
                    return Err("lookup accepts exactly one pointer".to_string());
                }
            }
        }
        index += 1;
    }
    let (expected_kind, payload) = parse_lookup_key(
        query
            .as_deref()
            .ok_or_else(|| "lookup requires a UUID, token, or typed marker".to_string())?,
    )?;
    let root = absolute_path(&root)?;
    let response = backend_request(&root, &json!({"op": "status"}))?;
    let records = response
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(|| "backend status omitted records".to_string())?;
    let record = match &payload {
        MarkerPayload::Uuid(uuid) => records.iter().find(|record| {
            record.get("uuid").and_then(Value::as_str) == Some(uuid.to_string().as_str())
        }),
        MarkerPayload::Token(code) => records
            .iter()
            .find(|record| record.get("token").and_then(Value::as_str) == Some(code.as_str())),
    }
    .ok_or_else(|| format!("pointer {} not found", payload.as_str()))?;
    let kind = record
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("function");
    if expected_kind.is_some_and(|expected| MarkerKind::parse(kind) != Some(expected)) {
        return Err(format!(
            "pointer {} has kind {kind}, not the requested kind",
            payload.as_str()
        ));
    }
    let token = record
        .get("token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| payload.as_str());
    println!("marker: 〚{}:{token}〛", emoji_for_kind(kind));
    for (label, key) in [
        ("token", "token"),
        ("name", "function"),
        ("description", "description"),
        ("created_at", "created_at"),
        ("updated_at", "updated_at"),
    ] {
        if let Some(value) = record.get(key).and_then(Value::as_str) {
            println!("{label}: {value}");
        }
    }
    let locations = record.get("locations").and_then(Value::as_array);
    if let Some(locations) = locations.filter(|locations| !locations.is_empty()) {
        for (number, location) in locations.iter().enumerate() {
            print_location(&root, location, number + 1, context);
        }
    } else {
        print_location(&root, record, 1, context);
    }
    Ok(())
}

fn parse_lookup_key(raw: &str) -> Result<(Option<MarkerKind>, MarkerPayload), String> {
    let text = raw
        .strip_prefix('〚')
        .and_then(|text| text.strip_suffix('〛'))
        .unwrap_or(raw);
    let (kind, payload_text) = match text.split_once(':') {
        Some((kind, payload)) => (
            Some(MarkerKind::parse(kind).ok_or_else(|| format!("unknown pointer kind: {kind}"))?),
            payload,
        ),
        None => (None, text),
    };
    let payload = if let Ok(uuid) = Uuid::parse_str(payload_text) {
        if uuid.get_version_num() != 5 || uuid.get_variant() != uuid::Variant::RFC4122 {
            return Err("pointer UUID must be version 5".to_string());
        }
        MarkerPayload::Uuid(uuid)
    } else if valid_token_glyphs(payload_text) {
        MarkerPayload::Token(payload_text.to_string())
    } else {
        return Err(format!("invalid pointer payload: {payload_text}"));
    };
    Ok((kind, payload))
}

fn print_location(root: &Path, location: &Value, number: usize, context: usize) {
    let Some(path) = location.get("file_path").and_then(Value::as_str) else {
        return;
    };
    let line = location.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
    let end_line = location
        .get("end_line")
        .and_then(Value::as_u64)
        .map(|value| value as usize);
    match end_line {
        Some(end) => println!("location {number}: {path}:{line}-{end}"),
        None => println!("location {number}: {path}:{line}"),
    }
    let source = root.join(path);
    let safe = fs::canonicalize(&source)
        .ok()
        .and_then(|path| {
            fs::canonicalize(root)
                .ok()
                .map(|base| path.starts_with(base))
        })
        .unwrap_or(false);
    if !safe {
        println!("  (source unavailable)");
        return;
    }
    let Ok(text) = fs::read_to_string(&source) else {
        println!("  (source unavailable)");
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = line.saturating_sub(context + 1);
    let end = end_line
        .unwrap_or(line)
        .saturating_add(context)
        .min(lines.len());
    let visible_end = end.min(start.saturating_add(16));
    for number in start..visible_end {
        println!("  {:>5} | {}", number + 1, lines[number]);
    }
    if visible_end < end {
        println!("  ... {} more line(s)", end - visible_end);
    }
}

/// Languages the annotate command can target. Detection is deliberately line-anchored
/// string matching (no AST, no regex dep) — declarations are matched only at the start
/// of a line after indentation, which excludes almost all string-literal false positives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Rust,
    Elixir,
    Js,
}

fn comment_leader(lang: Lang) -> &'static str {
    match lang {
        Lang::Elixir => "#",
        _ => "//",
    }
}

fn language_for_path(path: &Path, include_exs: bool) -> Option<Lang> {
    match path.extension().and_then(OsStr::to_str)? {
        "rs" => Some(Lang::Rust),
        "ex" => Some(Lang::Elixir),
        // .exs is scripts/migrations — opt-in only (`--lang exs`).
        "exs" if include_exs => Some(Lang::Elixir),
        "js" | "ts" | "mjs" | "tsx" => Some(Lang::Js),
        _ => None,
    }
}

fn detect_public_decl(lang: Lang, trimmed: &str) -> Option<(String, MarkerKind)> {
    match lang {
        Lang::Rust => detect_rust_pub_fn(trimmed).map(|name| (name, MarkerKind::Function)),
        Lang::Elixir => detect_elixir_public_def(trimmed),
        Lang::Js => detect_js_export(trimmed).map(|name| (name, MarkerKind::Function)),
    }
}

/// Alias-shaped identifier: `Foo`, `MyApp.Auth`, `Foo.Bar!?` — the name shape
/// of defmodule/defprotocol/defimpl heads.
fn dotted_alias(source: &str) -> Option<String> {
    let mut name = String::new();
    for c in source.chars() {
        if c == '.' || c == '?' || c == '!' || c == '_' || c.is_ascii_alphanumeric() {
            name.push(c);
        } else {
            break;
        }
    }
    let first = name.chars().next()?;
    if !first.is_ascii_uppercase() || name.ends_with('.') {
        return None;
    }
    Some(name)
}

fn ident(source: &str) -> Option<String> {
    let mut name = String::new();
    for c in source.chars() {
        if c == '_' || c == '$' || c.is_ascii_alphanumeric() {
            name.push(c);
        } else {
            break;
        }
    }
    let first = name.chars().next()?;
    if first.is_ascii_digit() {
        return None;
    }
    Some(name)
}

fn detect_rust_pub_fn(trimmed: &str) -> Option<String> {
    // Plain `pub` only — `pub(crate)`/`pub(super)` are internal API by declaration
    // and are deliberately not annotated ("pub = annotated" keeps the rule crisp).
    let mut rest = trimmed.strip_prefix("pub ")?.trim_start();
    loop {
        if let Some(next) = rest.strip_prefix("async ") {
            rest = next.trim_start();
            continue;
        }
        if let Some(next) = rest.strip_prefix("unsafe ") {
            rest = next.trim_start();
            continue;
        }
        if let Some(next) = rest.strip_prefix("const ") {
            rest = next.trim_start();
            continue;
        }
        if let Some(next) = rest.strip_prefix("extern ") {
            let next = next.trim_start();
            let after_quote = next.strip_prefix('"')?;
            let close = after_quote.find('"')?;
            rest = after_quote[close + 1..].trim_start();
            continue;
        }
        break;
    }
    ident(rest.strip_prefix("fn ")?.trim_start())
}

fn detect_elixir_public_def(trimmed: &str) -> Option<(String, MarkerKind)> {
    // Exact keyword + space, so defp/defmacrop/defdelegate and friends never match.
    if let Some(rest) = trimmed.strip_prefix("defmodule ") {
        return dotted_alias(rest.trim_start()).map(|name| (name, MarkerKind::Module));
    }
    if let Some(rest) = trimmed.strip_prefix("defprotocol ") {
        return dotted_alias(rest.trim_start()).map(|name| (name, MarkerKind::Contract));
    }
    if let Some(rest) = trimmed.strip_prefix("defimpl ") {
        let rest = rest.trim_start();
        let alias = dotted_alias(rest)?;
        // `defimpl Foo, for: Bar` — keep the for-target in the pointer name.
        let tail = &rest[alias.len()..];
        let name = match tail.find("for:") {
            Some(pos) => {
                let target = dotted_alias(tail[pos + 4..].trim_start())?;
                format!("{alias} for {target}")
            }
            None => alias,
        };
        return Some((name, MarkerKind::Contract));
    }
    if let Some(rest) = trimmed.strip_prefix("@callback ") {
        let name = ident(rest.trim_start()).filter(|name| !name.is_empty())?;
        return Some((name, MarkerKind::Contract));
    }
    let rest = trimmed
        .strip_prefix("def ")
        .or_else(|| trimmed.strip_prefix("defmacro "))
        .or_else(|| trimmed.strip_prefix("defguard "))?
        .trim_start();
    if rest.starts_with("unquote") {
        return None; // macro-generated head
    }
    let mut name = ident(rest)?;
    let first = name.chars().next()?;
    if !(first.is_ascii_lowercase() || first == '_') {
        return None;
    }
    if let Some(c) = rest[name.len()..].chars().next() {
        if c == '?' || c == '!' {
            name.push(c);
        }
    }
    Some((name, MarkerKind::Function))
}

fn detect_js_export(trimmed: &str) -> Option<String> {
    if let Some(rest) = trimmed.strip_prefix("export ") {
        let mut rest = rest.trim_start();
        if let Some(next) = rest.strip_prefix("default ") {
            rest = next.trim_start();
        }
        let mut fn_rest = rest;
        if let Some(next) = fn_rest.strip_prefix("async ") {
            fn_rest = next.trim_start();
        }
        if let Some(next) = fn_rest.strip_prefix("function") {
            let next = next.trim_start_matches('*').trim_start();
            return ident(next); // anonymous default export -> None
        }
        if let Some(next) = rest.strip_prefix("const ") {
            let next = next.trim_start();
            let name = ident(next)?;
            let after = next[name.len()..].trim_start();
            let eq = find_assignment_eq(after)?;
            if is_function_shaped(after[eq + 1..].trim_start()) {
                return Some(name);
            }
        }
        return None;
    }
    let rest = trimmed.strip_prefix("module.").unwrap_or(trimmed);
    let next = rest.strip_prefix("exports.")?;
    let name = ident(next)?;
    let after = next[name.len()..].trim_start();
    if after.starts_with('=') && !after.starts_with("==") && !after.starts_with("=>") {
        return Some(name);
    }
    None
}

/// Find the top-level `=` of an assignment, skipping TS type annotations that may
/// contain `=>` (e.g. `const f: (x: T) => U = ...`) and comparison operators.
fn find_assignment_eq(source: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    for (index, &byte) in bytes.iter().enumerate() {
        if byte != b'=' {
            continue;
        }
        let next = bytes.get(index + 1).copied();
        let prev = if index > 0 {
            Some(bytes[index - 1])
        } else {
            None
        };
        if next == Some(b'>') || next == Some(b'=') {
            continue;
        }
        if matches!(prev, Some(b'!') | Some(b'<') | Some(b'>') | Some(b'=')) {
            continue;
        }
        return Some(index);
    }
    None
}

fn is_function_shaped(rhs: &str) -> bool {
    let rhs = rhs
        .strip_prefix("async ")
        .map(str::trim_start)
        .unwrap_or(rhs);
    if rhs.starts_with('(') || rhs.starts_with("function") {
        return true;
    }
    match ident(rhs) {
        Some(name) => rhs[name.len()..].trim_start().starts_with("=>"),
        None => false,
    }
}

/// True when the contiguous comment/attribute/doc block immediately above the
/// declaration already contains a pointer marker — makes annotate idempotent and
/// tolerates humans relocating the marker within the doc block.
fn block_above_has_marker(lang: Lang, lines: &[&str], decl_idx: usize) -> bool {
    let mut index = decl_idx;
    let mut in_heredoc = false;
    while index > 0 {
        index -= 1;
        let line = lines[index];
        let trimmed = line.trim();
        if in_heredoc {
            if line.contains('〚') || line.contains('⟦') {
                return true;
            }
            if trimmed.starts_with('@') && trimmed.contains("\"\"\"") {
                in_heredoc = false;
            }
            continue;
        }
        if trimmed.is_empty() {
            break;
        }
        if lang == Lang::Elixir && trimmed == "\"\"\"" {
            in_heredoc = true;
            continue;
        }
        let is_comment = match lang {
            Lang::Rust => trimmed.starts_with("//") || trimmed.starts_with("#["),
            Lang::Elixir => trimmed.starts_with('#') || trimmed.starts_with('@'),
            Lang::Js => {
                trimmed.starts_with("//")
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('*')
                    || trimmed.starts_with('@')
            }
        };
        if !is_comment {
            break;
        }
        if line.contains('〚') || line.contains('⟦') {
            return true;
        }
    }
    false
}

/// First sentence of the declaration's doc block (Rust `///`, Elixir `@doc`, JSDoc/`//`),
/// sanitized for the marker grammar. `None` when no usable doc text exists.
fn derive_description(lang: Lang, lines: &[&str], decl_idx: usize) -> Option<String> {
    // Locate the start of the contiguous block above.
    let mut start = decl_idx;
    let mut index = decl_idx;
    let mut in_heredoc = false;
    while index > 0 {
        index -= 1;
        let trimmed = lines[index].trim();
        if in_heredoc {
            start = index;
            if trimmed.starts_with('@') && trimmed.contains("\"\"\"") {
                in_heredoc = false;
            }
            continue;
        }
        if trimmed.is_empty() {
            break;
        }
        if lang == Lang::Elixir && trimmed == "\"\"\"" {
            in_heredoc = true;
            start = index;
            continue;
        }
        let is_comment = match lang {
            Lang::Rust => trimmed.starts_with("//") || trimmed.starts_with("#["),
            Lang::Elixir => trimmed.starts_with('#') || trimmed.starts_with('@'),
            Lang::Js => {
                trimmed.starts_with("//")
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('*')
                    || trimmed.starts_with('@')
            }
        };
        if !is_comment {
            break;
        }
        start = index;
    }
    if start == decl_idx {
        return None;
    }
    // Downward pass: first usable doc-text line.
    let mut heredoc_doc = false;
    for line in &lines[start..decl_idx] {
        let trimmed = line.trim();
        let text: Option<&str> = match lang {
            Lang::Rust => trimmed.strip_prefix("///").map(str::trim),
            Lang::Elixir => {
                if heredoc_doc {
                    if trimmed == "\"\"\"" {
                        heredoc_doc = false;
                        None
                    } else {
                        Some(trimmed)
                    }
                } else if let Some(rest) = trimmed.strip_prefix("@doc") {
                    let rest = rest.trim();
                    if rest.starts_with("\"\"\"") {
                        heredoc_doc = true;
                        None
                    } else {
                        rest.strip_prefix('"')
                            .and_then(|r| r.rsplit_once('"'))
                            .map(|(body, _)| body)
                    }
                } else {
                    None
                }
            }
            Lang::Js => {
                let stripped = trimmed
                    .trim_start_matches("/**")
                    .trim_start_matches("/*")
                    .trim_start_matches('*')
                    .trim_start_matches("//")
                    .trim();
                let stripped = stripped.trim_end_matches("*/").trim();
                if stripped.is_empty() {
                    None
                } else {
                    Some(stripped)
                }
            }
        };
        if let Some(text) = text {
            let text = text.trim();
            if text.is_empty()
                || text.starts_with('〚')
                || text.starts_with('⟦')
                || text.starts_with('@')
            {
                continue;
            }
            return Some(sanitize_description(text));
        }
    }
    None
}

/// Keep descriptions marker-grammar-safe: `::` is the name/description separator,
/// so any embedded `::` is softened; long docs are cut to the first sentence / 100 chars.
fn sanitize_description(text: &str) -> String {
    let mut clean = text.replace("::", ":");
    if let Some(pos) = clean.find(". ") {
        clean.truncate(pos + 1);
    }
    if clean.chars().count() > 100 {
        clean = clean
            .chars()
            .take(100)
            .collect::<String>()
            .trim_end()
            .to_string();
    }
    clean.trim().to_string()
}

/// Shape of an existing `@doc`/`@moduledoc` attribute so annotate can merge
/// the marker into it instead of stacking a second attribute.
#[derive(Debug)]
enum ElixirDocAttr {
    /// No doc attribute found — a fresh one may be inserted.
    Missing,
    /// `@doc false`, keyword forms, or anything malformed: leave the line
    /// untouched and hang the marker on a `#` comment above it instead.
    Leave(usize),
    /// `@doc "one line"` — index of the attribute line.
    SingleLine(usize),
    /// `@doc """` heredoc — (opener index, closing `"""` index).
    Heredoc(usize, usize),
}

/// Locate the `@doc`-style attribute in the contiguous block above a def.
fn find_elixir_doc_attr(lines: &[&str], decl_idx: usize, attr: &str) -> ElixirDocAttr {
    let mut index = decl_idx;
    let mut heredoc_close: Option<usize> = None;
    while index > 0 {
        index -= 1;
        let trimmed = lines[index].trim();
        if let Some(close) = heredoc_close {
            if trimmed.starts_with('@') && trimmed.contains("\"\"\"") {
                if trimmed.starts_with(attr) {
                    return ElixirDocAttr::Heredoc(index, close);
                }
                heredoc_close = None; // some other attribute's heredoc; keep walking
            }
            continue;
        }
        if trimmed.is_empty() {
            break;
        }
        if trimmed == "\"\"\"" {
            heredoc_close = Some(index);
            continue;
        }
        if !(trimmed.starts_with('#') || trimmed.starts_with('@')) {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix(attr) {
            let rest = rest.trim();
            if rest == "false" {
                return ElixirDocAttr::Leave(index);
            }
            if rest.starts_with("\"\"\"") {
                // A heredoc opener seen before its closer only happens in
                // unbalanced source; safer to leave it alone than to rewrite.
                return ElixirDocAttr::Leave(index);
            }
            if rest.starts_with('"') {
                return ElixirDocAttr::SingleLine(index);
            }
            // keyword forms like `@doc since: "1.2"` carry no docstring; keep walking
        }
    }
    ElixirDocAttr::Missing
}

/// Locate the `@moduledoc` in the attribute header block below `defmodule X do`.
/// Returns the attribute shape plus the line index a fresh `@moduledoc` belongs at.
fn find_elixir_moduledoc(lines: &[&str], decl_idx: usize) -> (ElixirDocAttr, usize) {
    let mut index = decl_idx + 1;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if let Some(rest) = trimmed.strip_prefix("@moduledoc") {
            let rest = rest.trim();
            if rest == "false" {
                return (ElixirDocAttr::Leave(index), index);
            }
            if rest.starts_with("\"\"\"") {
                return match lines[index + 1..]
                    .iter()
                    .position(|line| line.trim() == "\"\"\"")
                {
                    Some(offset) => (ElixirDocAttr::Heredoc(index, index + 1 + offset), index),
                    None => (ElixirDocAttr::Leave(index), index),
                };
            }
            if rest.starts_with('"') {
                return (ElixirDocAttr::SingleLine(index), index);
            }
            // keyword form — keep scanning the header block
        } else if trimmed == "\"\"\""
            || (!trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with('@'))
        {
            break; // module body starts — no moduledoc in the header block
        }
        index += 1;
    }
    (ElixirDocAttr::Missing, decl_idx + 1)
}

/// defmodule targets carry their marker in the @moduledoc BELOW the head, so
/// idempotency has to look down there, not just at the block above.
fn module_doc_has_marker(lines: &[&str], decl_idx: usize) -> bool {
    let mut index = decl_idx + 1;
    let mut in_heredoc = false;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        if in_heredoc {
            if trimmed == "\"\"\"" {
                in_heredoc = false;
            } else if line.contains('〚') || line.contains('⟦') {
                return true;
            }
            index += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("@moduledoc") {
            if rest.trim().starts_with("\"\"\"") {
                in_heredoc = true;
            } else if line.contains('〚') || line.contains('⟦') {
                return true;
            }
        } else if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@') {
            if line.contains('〚') || line.contains('⟦') {
                return true;
            }
        } else {
            break;
        }
        index += 1;
    }
    false
}

/// First sentence of an existing doc attribute's docstring, for reuse as the
/// marker description when merging into it.
fn elixir_doc_attr_summary(lines: &[&str], attr: &ElixirDocAttr) -> Option<String> {
    match attr {
        ElixirDocAttr::Heredoc(open, close) => lines[*open + 1..*close]
            .iter()
            .map(|line| line.trim())
            .find(|line| !line.is_empty() && !line.starts_with('〚') && !line.starts_with('⟦'))
            .map(|line| sanitize_description(line)),
        ElixirDocAttr::SingleLine(idx) => {
            let trimmed = lines[*idx].trim();
            let body = trimmed
                .split_once(' ')
                .map(|(_, rest)| rest.trim().trim_matches('"').to_string())
                .unwrap_or_default();
            (!body.is_empty()).then(|| sanitize_description(&body))
        }
        _ => None,
    }
}

fn elixir_new_doc_block(
    attr: &str,
    indent: &str,
    summary: Option<&str>,
    how: Option<&str>,
    marker_line: &str,
) -> Vec<String> {
    let mut block = vec![format!("{indent}{attr} \"\"\"\n")];
    if let Some(summary) = summary.filter(|s| !s.is_empty()) {
        block.push(format!("{indent}{summary}\n"));
        if how.is_some() {
            // Plain empty line: an indented blank separator is normalized away
            // by `mix format`, which would break --check-formatted afterwards.
            block.push("\n".to_string());
        }
    }
    if let Some(how) = how.filter(|h| !h.is_empty()) {
        block.push(format!("{indent}How: {how}\n"));
    }
    if block.len() > 1 {
        block.push("\n".to_string());
    }
    block.push(format!("{indent}{marker_line}\n"));
    block.push(format!("{indent}\"\"\"\n"));
    block
}

/// Widen `@doc "text"` into a heredoc that preserves the existing text and
/// appends the marker line.
fn elixir_widen_single_line(line: &str, marker_line: &str) -> Vec<String> {
    let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    let trimmed = line.trim();
    let attr = trimmed.split_whitespace().next().unwrap_or("@doc");
    let body = trimmed
        .strip_prefix(attr)
        .map(|rest| rest.trim().trim_matches('"').to_string())
        .unwrap_or_default();
    let mut block = vec![format!("{indent}{attr} \"\"\"\n")];
    if !body.is_empty() {
        block.push(format!("{indent}{body}\n"));
    }
    block.push("\n".to_string());
    block.push(format!("{indent}{marker_line}\n"));
    block.push(format!("{indent}\"\"\"\n"));
    block
}

/// Lines inserted just before a heredoc's closing `"""`: a plain empty line and
/// the marker, at the closer's indent so heredoc unindenting keeps the marker.
/// The separator stays truly empty — an indented blank line is normalized away
/// by `mix format`, which would break --check-formatted afterwards.
fn elixir_heredoc_marker_lines(close_line: &str, marker_line: &str) -> Vec<String> {
    let indent: String = close_line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    vec!["\n".to_string(), format!("{indent}{marker_line}\n")]
}

fn indent_of(line: &str) -> String {
    line.chars().take_while(|c| c.is_whitespace()).collect()
}

/// Derive a "How:" clause from the declaration signature: argument list
/// (defaults stripped), Elixir `when` guards and `@spec`/`@callback` return
/// type, Rust return type. `None` when nothing is derivable from the head.
fn derive_how(lang: Lang, lines: &[&str], decl_idx: usize) -> Option<String> {
    let mut head = lines[decl_idx].trim();
    let mut parts: Vec<String> = Vec::new();
    match lang {
        Lang::Elixir => {
            let mut args = elixir_head_args(head);
            // Multi-clause heads often lead with an error pass-through clause
            // (`def id({:error, _} = e), do: e`); describing only it misleads.
            // Prefer the first substantive clause of the same function.
            if !head.starts_with("@callback ") && elixir_error_passthrough(&args) {
                if let Some(primary) = elixir_primary_clause_head(lines, decl_idx) {
                    head = primary;
                    args = elixir_head_args(head);
                }
            }
            if !args.is_empty() {
                parts.push(format_args_clause(&args));
            }
            if let Some(guard) = elixir_head_guard(head) {
                parts.push(format!("guards `{guard}`"));
            }
            let ret = if head.starts_with("@callback ") {
                head.split_once("::")
                    .map(|(_, ret)| ret.trim().trim_end_matches(',').to_string())
            } else {
                elixir_spec_return(lines, decl_idx)
            };
            if let Some(ret) = ret.filter(|ret| !ret.is_empty()) {
                parts.push(format!("returns `{ret}`"));
            }
        }
        Lang::Rust => {
            let (args, ret) = rust_head_parts(head)?;
            if !args.is_empty() {
                parts.push(format_args_clause(&args));
            }
            if let Some(ret) = ret.filter(|ret| !ret.is_empty()) {
                parts.push(format!("returns `{ret}`"));
            }
        }
        Lang::Js => {
            let args = js_head_args(head);
            if !args.is_empty() {
                parts.push(format_args_clause(&args));
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

fn format_args_clause(args: &[String]) -> String {
    let list = args
        .iter()
        .map(|arg| format!("`{arg}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("takes {list}")
}

/// `&source[..offset]` of the bracket matching the first unbalanced opener —
/// i.e. the argument-list body for a head whose `(` we already consumed.
fn matching_bracket(source: &str, close: char) -> Option<&str> {
    let mut depth = 0usize;
    for (offset, c) in source.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => return (c == close).then(|| &source[..offset]),
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn split_top_level_commas(body: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for c in body.chars() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if depth == 0 => {
                parts.push(current.clone());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
}

/// Top-level argument list of an Elixir head, defaults (`\\ value`) stripped.
/// Multi-line heads degrade to an empty list rather than guessing.
fn elixir_head_args(head: &str) -> Vec<String> {
    let Some(open) = head.find('(') else {
        return Vec::new();
    };
    let Some(body) = matching_bracket(&head[open + 1..], ')') else {
        return Vec::new();
    };
    if body.trim().is_empty() {
        return Vec::new();
    }
    split_top_level_commas(body)
        .into_iter()
        .map(|arg| arg.split("\\\\").next().unwrap_or(&arg).trim().to_string())
        .filter(|arg| !arg.is_empty())
        .collect()
}

fn elixir_head_guard(head: &str) -> Option<String> {
    let (_, rest) = head.split_once(" when ")?;
    let mut guard = rest;
    if let Some(pos) = guard.find(", do:") {
        guard = &guard[..pos];
    }
    let guard = guard.trim_end().trim_end_matches(" do").trim();
    (!guard.is_empty()).then(|| guard.to_string())
}

/// A clause whose FIRST arg matches `{:error, _} = e` (or bare `{:error, _}`),
/// any arity — the conventional error pass-through head of a multi-clause
/// function (`def id({:error, _} = e), do: e`, `def entity({:error, _} = e, _)`).
fn elixir_error_passthrough(args: &[String]) -> bool {
    args.first()
        .is_some_and(|first| first.trim_start().starts_with("{:error,"))
}

/// First substantive (non-error-pass-through) clause head of the function
/// declared at `decl_idx`, scanning sibling `def`/`defmacro`/`defguard` heads
/// with the same name. Best-effort: stops at the first clause of a different
/// function or at a scope boundary, and gives up after a small window.
fn elixir_primary_clause_head<'a>(lines: &'a [&'a str], decl_idx: usize) -> Option<&'a str> {
    let (decl_name, _) = detect_elixir_public_def(lines[decl_idx].trim())?;
    for line in lines.iter().skip(decl_idx + 1).take(12) {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with('@')
            || trimmed == "end"
        {
            continue;
        }
        let Some((name, _)) = detect_elixir_public_def(trimmed) else {
            continue; // body line of the current clause (e.g. `do: e`)
        };
        if name != decl_name {
            return None; // reached the next function
        }
        if !elixir_error_passthrough(&elixir_head_args(trimmed)) {
            return Some(trimmed);
        }
    }
    None
}

fn elixir_spec_return(lines: &[&str], decl_idx: usize) -> Option<String> {
    let mut index = decl_idx;
    while index > 0 {
        index -= 1;
        let trimmed = lines[index].trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(spec) = trimmed.strip_prefix("@spec ") {
            return spec
                .split_once("::")
                .map(|(_, ret)| ret.trim().trim_end_matches(',').to_string());
        }
        if trimmed.starts_with('#') || trimmed.starts_with('@') {
            continue;
        }
        break;
    }
    None
}

/// `(args, return)` from a single-line Rust head; `self` receivers are dropped.
fn rust_head_parts(head: &str) -> Option<(Vec<String>, Option<String>)> {
    let open = head.find('(')?;
    let body = matching_bracket(&head[open + 1..], ')')?;
    let args = if body.trim().is_empty() {
        Vec::new()
    } else {
        split_top_level_commas(body)
            .into_iter()
            .map(|arg| arg.trim().to_string())
            .filter(|arg| !arg.is_empty() && arg != "self" && arg != "&self" && arg != "&mut self")
            .collect()
    };
    let after = head[open + 2 + body.len()..].trim_start(); // skip the closing ')'
    let ret = after.strip_prefix("->").map(|rest| {
        let ret = rest.split('{').next().unwrap_or(rest);
        let ret = ret.split(" where").next().unwrap_or(ret);
        ret.trim().to_string()
    });
    let ret = match ret {
        Some(ret) if !ret.is_empty() => Some(ret),
        _ => None,
    };
    Some((args, ret))
}

fn js_head_args(head: &str) -> Vec<String> {
    let Some(open) = head.find('(') else {
        return Vec::new();
    };
    let Some(body) = matching_bracket(&head[open + 1..], ')') else {
        return Vec::new();
    };
    if body.trim().is_empty() {
        return Vec::new();
    }
    split_top_level_commas(body)
        .into_iter()
        .map(|arg| arg.split('=').next().unwrap_or(&arg).trim().to_string())
        .map(|arg| arg.split(':').next().unwrap_or(&arg).trim().to_string())
        .filter(|arg| !arg.is_empty())
        .collect()
}

/// Honest description fallback when nothing is derivable from docs: the shape
/// of the declaration itself (name/arity for Elixir, signature elsewhere).
fn fallback_description(lang: Lang, name: &str, head: &str, kind: MarkerKind) -> String {
    if kind == MarkerKind::Module {
        return format!("{name} module");
    }
    let head = head.trim_start();
    match lang {
        Lang::Elixir => {
            if head.starts_with("defprotocol ") {
                format!("{name} protocol")
            } else if head.starts_with("defimpl ") {
                format!("{name} implementation")
            } else {
                let args = elixir_head_args(head);
                if !args.is_empty() {
                    format!("{name}/{}", args.len())
                } else if head.contains('(') {
                    format!("{name}/0")
                } else {
                    name.to_string()
                }
            }
        }
        Lang::Rust => match rust_head_parts(head) {
            Some((args, Some(ret))) if !args.is_empty() => {
                format!("{name}({}) -> {ret}", args.join(", "))
            }
            Some((args, None)) if !args.is_empty() => format!("{name}({})", args.join(", ")),
            Some((_, Some(ret))) => format!("{name}() -> {ret}"),
            _ => name.to_string(),
        },
        Lang::Js => match js_head_args(head) {
            args if !args.is_empty() => format!("{name}({})", args.join(", ")),
            _ => format!("{name}()"),
        },
    }
}

fn annotate_command(args: &[String]) -> Result<(), String> {
    let options = parse_annotate_options(args)?;
    let root = absolute_path(&options.root)?;
    let db_path = legacy_db_path(&root, &options.db)?;

    // Live collision map: DB entries plus every marker in the scanned tree (sources are
    // scanned too now, so stray markers not yet indexed are collision-checked as well).
    let mut token_index = store_token_index(&root, &db_path)?;
    let (mut pointers, errors) = collect_pointers(&root, &db_path, &options.filter, &token_index)?;
    for pointer in backend_status(&root)? {
        pointers.entry(pointer.code.clone()).or_insert(pointer);
    }
    for pointer in legacy_records(&db_path)?.into_values() {
        pointers.entry(pointer.code.clone()).or_insert(pointer);
    }
    for error in &errors {
        eprintln!("WARNING: {error}");
    }

    let mut planned = 0usize;
    let mut minted: HashMap<String, Uuid> = HashMap::new();
    let mut file_reports: Vec<String> = Vec::new();
    let mut review_flags: Vec<String> = Vec::new();

    for path in scan_files(&root, &db_path, &options.filter)? {
        let Some(lang) = language_for_path(&path, options.include_exs) else {
            continue;
        };
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let relpath = rel_path(&path, &root);
        let lines: Vec<&str> = text.lines().collect();
        let mut targets: Vec<(usize, String, MarkerKind)> = Vec::new();
        let mut seen_names: HashSet<String> = HashSet::new();
        for (idx, line) in lines.iter().enumerate() {
            if line.len() > 500 {
                continue; // minification tripwire
            }
            let trimmed = line.trim_start();
            let Some((name, kind)) = detect_public_decl(lang, trimmed) else {
                if lang == Lang::Js && trimmed.starts_with("module.exports = {") {
                    review_flags.push(format!(
                        "{relpath}:{}: object-literal module.exports — annotate members manually",
                        idx + 1
                    ));
                }
                continue;
            };
            if lang == Lang::Elixir && !seen_names.insert(name.clone()) {
                continue; // later clause / other arity of an already-annotated function
            }
            if block_above_has_marker(lang, &lines, idx) {
                continue;
            }
            if kind == MarkerKind::Module && module_doc_has_marker(&lines, idx) {
                continue;
            }
            targets.push((idx, name, kind));
        }
        if targets.is_empty() {
            continue;
        }
        if lang == Lang::Rust && text.contains("macro_rules!") {
            review_flags.push(format!(
                "{relpath}: contains macro_rules! — review inserted markers manually"
            ));
        }

        // (line index, remove count, replacement lines, sequence) — applied
        // bottom-up so earlier indices never shift under later edits.
        let mut edits: Vec<(usize, usize, Vec<String>, usize)> = Vec::new();
        for (seq, (idx, name, kind)) in targets.iter().enumerate() {
            let idx = *idx;
            let head = lines[idx];
            let indent = indent_of(head);
            let seed = format!("{relpath}::{name}");
            let (code, uuid) = resolve_annotate_pointer(
                &seed,
                &relpath,
                &name,
                &pointers,
                &minted,
                options.force_remint,
            )?;
            minted.insert(code.clone(), uuid);
            // Fresh markers are token-form; index them so the closing build's
            // re-collect resolves them without a store round-trip.
            token_index.insert(code.clone(), uuid);
            let how = derive_how(lang, &lines, idx);

            let (doc_attr, summary) = if lang == Lang::Elixir && *kind == MarkerKind::Module {
                let (attr, _) = find_elixir_moduledoc(&lines, idx);
                let summary = elixir_doc_attr_summary(&lines, &attr);
                (Some(attr), summary)
            } else {
                (None, derive_description(lang, &lines, idx))
            };
            let desc = match summary {
                Some(ref summary) if !summary.is_empty() => summary.clone(),
                _ => sanitize_description(&fallback_description(lang, name, head, *kind)),
            };
            let marker_line = format!("〚{}:{code}〛 {name} :: {desc}", kind.emoji());

            if lang == Lang::Elixir {
                let attr_name = if *kind == MarkerKind::Module {
                    "@moduledoc"
                } else {
                    "@doc"
                };
                // @doc above defprotocol/defimpl/@callback heads warns or
                // breaks compilation — contracts keep comment markers.
                let effective = if *kind == MarkerKind::Contract {
                    ElixirDocAttr::Missing
                } else if *kind == MarkerKind::Module {
                    doc_attr.unwrap_or(ElixirDocAttr::Missing)
                } else {
                    find_elixir_doc_attr(&lines, idx, "@doc")
                };
                match effective {
                    _ if *kind == MarkerKind::Contract => {
                        edits.push((idx, 0, vec![format!("{indent}# {marker_line}\n")], seq));
                    }
                    ElixirDocAttr::Missing if *kind == MarkerKind::Module => {
                        let inner_indent = format!("{indent}  ");
                        edits.push((
                            idx + 1,
                            0,
                            elixir_new_doc_block(
                                attr_name,
                                &inner_indent,
                                summary.as_deref(),
                                how.as_deref(),
                                &marker_line,
                            ),
                            seq,
                        ));
                    }
                    ElixirDocAttr::Missing => {
                        edits.push((
                            idx,
                            0,
                            elixir_new_doc_block(
                                attr_name,
                                &indent,
                                summary.as_deref(),
                                how.as_deref(),
                                &marker_line,
                            ),
                            seq,
                        ));
                    }
                    ElixirDocAttr::Leave(attr_idx) => {
                        // `@doc false` and malformed attributes: never change
                        // doc visibility — hang the marker on a comment above.
                        let attr_indent = indent_of(lines[attr_idx]);
                        edits.push((
                            attr_idx,
                            0,
                            vec![format!("{attr_indent}# {marker_line}\n")],
                            seq,
                        ));
                    }
                    ElixirDocAttr::SingleLine(attr_idx) => {
                        edits.push((
                            attr_idx,
                            1,
                            elixir_widen_single_line(lines[attr_idx], &marker_line),
                            seq,
                        ));
                    }
                    ElixirDocAttr::Heredoc(_open, close) => {
                        // Existing docstring: append blank + marker before the
                        // closing `"""` — never stack a second attribute.
                        edits.push((
                            close,
                            0,
                            elixir_heredoc_marker_lines(lines[close], &marker_line),
                            seq,
                        ));
                    }
                }
            } else {
                let leader = comment_leader(lang);
                edits.push((
                    idx,
                    0,
                    vec![format!("{indent}{leader} {marker_line}\n")],
                    seq,
                ));
            }
            pointers.insert(
                code.clone(),
                Pointer {
                    uuid: Some(uuid),
                    kind: Some(kind.canonical().to_string()),
                    code,
                    path: relpath.clone(),
                    line: idx + 1, // provisional; the closing build records exact lines
                    name: name.clone(),
                    description: desc,
                    locations: vec![],
                },
            );
        }

        planned += edits.len();
        file_reports.push(format!("{relpath}: {} pointer(s)", edits.len()));

        if options.write && !edits.is_empty() {
            let mut new_lines: Vec<String> =
                text.split_inclusive('\n').map(str::to_string).collect();
            edits.sort_by(|a, b| (b.0, b.3).cmp(&(a.0, a.3)));
            for (index, remove, replacement, _seq) in edits {
                let end = (index + remove).min(new_lines.len());
                new_lines.splice(index..end, replacement);
            }
            fs::write(&path, new_lines.concat())
                .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        }
    }

    for report in &file_reports {
        println!("{report}");
    }
    for flag in &review_flags {
        println!("REVIEW: {flag}");
    }

    if options.write {
        // Closing build: re-collect (markers shifted lines) and persist DB + deeplinks
        // in the same invocation so `build --check` is green immediately after.
        let (mut fresh, build_errors) =
            collect_pointers(&root, &db_path, &options.filter, &token_index)?;
        for (code, uuid) in minted {
            if let Some(pointer) = fresh.get_mut(&code) {
                pointer.uuid = Some(uuid);
            }
        }
        hydrate_pointer_ids(&root, &db_path, &mut fresh)?;
        for error in &build_errors {
            eprintln!("WARNING: {error}");
        }
        let records = reconcile_records(&root, &db_path, &fresh, &options.filter)?;
        let db_changed = backend_reconcile(&root, &records, true)?;
        let expansion = expansion_index(&root, &db_path, &fresh)?;
        let (changed_links, link_errors) =
            expand_markdown_links(&root, &expansion, true, &options.filter)?;
        for error in &link_errors {
            eprintln!("ERROR: {error}");
        }
        println!(
            "inserted {planned} pointer(s); stores {}{}",
            if db_changed { "updated" } else { "unchanged" },
            if changed_links.is_empty() {
                String::new()
            } else {
                format!("; expanded deeplinks in: {}", changed_links.join(", "))
            }
        );
        if !link_errors.is_empty() {
            std::process::exit(1);
        }
    } else {
        println!("would insert {planned} pointer(s); run with --write to apply");
    }

    Ok(())
}

fn parse_annotate_options(args: &[String]) -> Result<AnnotateOptions, String> {
    let mut options = AnnotateOptions {
        root: PathBuf::from("."),
        db: DEFAULT_DB_PATH.to_string(),
        write: false,
        include_exs: false,
        force_remint: false,
        filter: ScanFilter::default(),
    };

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => {
                print_annotate_help();
                std::process::exit(0);
            }
            "--root" => {
                index += 1;
                options.root = PathBuf::from(expect_value(args, index, "--root")?);
            }
            "--db" => {
                index += 1;
                options.db = expect_value(args, index, "--db")?.to_string();
            }
            "--include" => {
                index += 1;
                options
                    .filter
                    .include
                    .push(PathBuf::from(expect_value(args, index, "--include")?));
            }
            "--exclude" => {
                index += 1;
                options
                    .filter
                    .exclude
                    .push(PathBuf::from(expect_value(args, index, "--exclude")?));
            }
            "--force-remint" => options.force_remint = true,
            "--lang" => {
                index += 1;
                match expect_value(args, index, "--lang")? {
                    "exs" => options.include_exs = true,
                    value => return Err(format!("unsupported --lang value: {value}")),
                }
            }
            "--write" => options.write = true,
            value => return Err(format!("unknown option: {value}")),
        }
        index += 1;
    }

    Ok(options)
}

fn print_annotate_help() {
    println!(
        "usage: doc-pointers annotate [--root ROOT] [--db DB] [--include P]... [--exclude P]... [--lang exs] [--write]\n\n\
Walk the tree and insert `〚emoji:TOKEN〛 Name :: Description` markers (4-glyph token; full\n\
UUIDs stay accepted on lookup) above every public/exported Rust (`pub fn`), Elixir\n\
(`def`/`defmacro`/`defguard`), and JS/TS (`export`/`exports.`) function that does not\n\
already have one in its doc/comment block. Rust and JS get `//` comment lines; Elixir\n\
markers go into `@doc` docstrings (merged into existing ones, never stacked), modules\n\
get `@moduledoc`, and contracts (`defprotocol`/`defimpl`/`@callback`) plus `@doc false`\n\
declarations get `#` comments. Dry-run by default; --write applies the insertions and\n\
then reconciles .meta/pointers.yaml + expands deeplinks in the same run.\n\n\
options:\n  --root ROOT       repository root, default: current directory\n  --db DB           deprecated; only docs/doc-pointer-db.json is accepted for migration\n  --include P       only scan/annotate under this root-relative prefix (repeatable)\n  --exclude P       skip this root-relative prefix or glob, e.g. **/vendored/** (repeatable;\n                    worktree/staging duplicates are always excluded)\n  --force-remint    mint fresh tokens instead of reusing existing records for\n                    the same {{file}}::{{name}}\n  --lang exs        also annotate .exs scripts (skipped by default)\n  --write           apply insertions (otherwise dry-run report only)"
    );
}

fn parse_scan_options(args: &[String]) -> Result<ScanOptions, String> {
    let mut options = ScanOptions {
        root: PathBuf::from("."),
        db: DEFAULT_DB_PATH.to_string(),
        write: false,
        check: false,
        install_hook: false,
        filter: ScanFilter::default(),
    };

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => {
                print_scan_help();
                std::process::exit(0);
            }
            "--root" => {
                index += 1;
                options.root = PathBuf::from(expect_value(args, index, "--root")?);
            }
            "--db" => {
                index += 1;
                options.db = expect_value(args, index, "--db")?.to_string();
            }
            "--include" => {
                index += 1;
                options
                    .filter
                    .include
                    .push(PathBuf::from(expect_value(args, index, "--include")?));
            }
            "--exclude" => {
                index += 1;
                options
                    .filter
                    .exclude
                    .push(PathBuf::from(expect_value(args, index, "--exclude")?));
            }
            "--write" => options.write = true,
            "--check" => options.check = true,
            "--install-hook" => options.install_hook = true,
            value => return Err(format!("unknown option: {value}")),
        }
        index += 1;
    }

    Ok(options)
}

fn parse_uuid5_options(args: &[String]) -> Result<Uuid5Options, String> {
    let mut name = None;
    let mut root = PathBuf::from(".");
    let mut db = DEFAULT_DB_PATH.to_string();
    let mut namespace = "doc-pointers".to_string();
    let mut salt = String::new();
    let mut format = PointerFormat::Marker;
    let mut kind = "function".to_string();
    let mut description = String::new();
    let mut no_clipboard = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => {
                print_uuid5_help();
                std::process::exit(0);
            }
            "--root" => {
                index += 1;
                root = PathBuf::from(expect_value(args, index, "--root")?);
            }
            "--db" => {
                index += 1;
                db = expect_value(args, index, "--db")?.to_string();
            }
            "--namespace" => {
                index += 1;
                namespace = expect_value(args, index, "--namespace")?.to_string();
            }
            "--salt" => {
                index += 1;
                salt = expect_value(args, index, "--salt")?.to_string();
            }
            "--format" => {
                index += 1;
                format = match expect_value(args, index, "--format")? {
                    "marker" => PointerFormat::Marker,
                    "code" => PointerFormat::Code,
                    "declaration" => PointerFormat::Declaration,
                    "deeplink" => PointerFormat::Deeplink,
                    value => return Err(format!("invalid --format value: {value}")),
                };
            }
            "--kind" => {
                index += 1;
                let value = expect_value(args, index, "--kind")?;
                kind = parse_kind_string(&value)
                    .ok_or_else(|| format!("invalid --kind value: {value}"))?;
            }
            "--description" => {
                index += 1;
                description = expect_value(args, index, "--description")?.to_string();
            }
            "--no-clipboard" => no_clipboard = true,
            value if value.starts_with('-') => return Err(format!("unknown option: {value}")),
            value => {
                if name.is_some() {
                    return Err(format!("unexpected argument: {value}"));
                }
                name = Some(value.to_string());
            }
        }
        index += 1;
    }

    Ok(Uuid5Options {
        name,
        root,
        db,
        namespace,
        salt,
        format,
        kind,
        description,
        no_clipboard,
    })
}

fn expect_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, String> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn print_help() {
    println!(
        "\
doc-pointers — durable UUID pointer markers

A canonical marker has a type emoji and the pointer's 4-glyph token:
  # 〚🔧:𓳔𔐮𔘟𔄵〛 run :: Runs the task
  # 〚🧩:𓳔𔐮𔘟𔄵〛 component :: reusable span
  # 〚/🧩:𓳔𔐮𔘟𔄵〛
Full UUIDv5 payloads (〚🔧:5c692577-ad0c-51f1-992c-759b5e5fffb5〛) remain
accepted everywhere markers are parsed or looked up; new markers embed the
token, derived from the UUID — tokens are never invented.

Markers use scope emoji; stored kinds are canonical strings. 📁 file,
       📦 module/class/struct, 🔌 interface/protocol/behaviour, 🧩 component,
       🔧 function, 🔀 logic, 📐 diagram.
Spans for 🧩, 🔀, and 📐 require a matching closing marker. A component may
appear in multiple files; lookup lists all its locations. Old ⟦4-glyph⟧
declarations and deeplinks are read for migration, never generated.

Commands:
  doc-pointers uuid5 [NAME]       mint UUID and typed marker (--kind KIND)
  doc-pointers lookup UUID|token  show metadata and source snippets
  doc-pointers build              scan without writing
  doc-pointers build --write      reconcile .meta/pointers.yaml and links
  doc-pointers build --check      fail if stores or links are stale
  doc-pointers annotate           report unmarked public functions
  doc-pointers annotate --write   insert typed function markers, reconcile
  doc-pointers hook               install pre-commit build --check hook

Canonical Markdown link: [label](deeplink:〚🔧:𓳔𔐮𔘟𔄵〛)
Expanded target: path:line?pointer=UUID

Use `COMMAND --help` for options. Bare --write/--check/--install-hook
remain aliases for build/hook."
    );
}

fn print_scan_help() {
    println!(
        "usage: doc-pointers build [--root ROOT] [--db DB] [--write] [--check]\n\n\
Reconcile doc pointer stores and expand deeplink: markdown links.\n\
Run without --write/--check, this is a dry run: it reports how many\n\
declarations it found and any errors, but changes nothing.\n\n\
options:\n  --root ROOT       repository root, default: current directory\n  --db DB           deprecated; only docs/doc-pointer-db.json is accepted for migration\n  --include P       only scan under this root-relative prefix (repeatable)\n  --exclude P       skip this root-relative prefix or glob, e.g. **/vendored/** (repeatable;\n                    worktree/staging duplicates are always excluded)\n  --write           reconcile stores and expand markdown deeplinks\n  --check           fail if writes would be needed (CI / pre-commit)"
    );
}

fn print_uuid5_help() {
    println!(
        "usage: doc-pointers uuid5 [options] [NAME]\n\n\
Generate a deterministic UUIDv5 and typed pointer marker.\n\
The token is collision-checked against current stores and copied to\n\
the clipboard unless --no-clipboard is given.\n\n\
options:\n  --root ROOT             repository root, default: current directory\n  --db DB                 deprecated; only docs/doc-pointer-db.json is accepted\n  --namespace NAMESPACE   doc-pointers, dns, url, oid, x500, or a UUID\n  --salt SALT             optional deterministic salt\n  --format FORMAT         marker, code, declaration, or deeplink\n  --kind KIND             file, module, class, struct, interface, protocol, behaviour,
                          function, logic, component, or diagram (legacy emoji accepted)\n  --description TEXT      description used by --format declaration\n  --no-clipboard          print without copying to clipboard"
    );
}

fn collect_pointers(
    root: &Path,
    db_path: &Path,
    filter: &ScanFilter,
    token_index: &HashMap<String, Uuid>,
) -> Result<(HashMap<String, Pointer>, Vec<String>), String> {
    let mut pointers: HashMap<String, Pointer> = HashMap::new();
    let mut errors = Vec::new();
    for path in scan_files(root, db_path, filter)? {
        let file_path = rel_path(&path, root);
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let ext = path.extension().and_then(OsStr::to_str).unwrap_or("");
        let is_md = ext == "md";
        let is_elixir = ext == "ex" || ext == "exs";
        let is_rust = ext == "rs";
        // Fence/string state, per file:
        // - .md: plain ``` fences hide their contents.
        // - .ex/.exs: inside @doc/@moduledoc heredocs, ``` examples hide their
        //   contents; markers outside those fences (how annotate inserts them)
        //   still parse.
        // - .rs: fenced examples in runs of `///` doc comments hide their
        //   contents; `\`-continued string literals (help text, embedded
        //   examples) carry string state across lines so marker-shaped tokens
        //   inside them are not mistaken for real markers.
        let mut md_fence = false;
        let mut ex_in_doc = false;
        let mut ex_doc_fence = false;
        let mut rs_doc_fence = false;
        let mut rs_prev_doc = false;
        let mut rs_string_next = false;
        let mut spans: Vec<(MarkerKind, Uuid, String, usize)> = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if is_md {
                if fence_toggle(trimmed) {
                    md_fence = !md_fence;
                }
                if md_fence {
                    continue;
                }
            } else {
                if is_elixir {
                    if ex_in_doc {
                        if trimmed == "\"\"\"" {
                            ex_in_doc = false;
                        } else if fence_toggle(comment_body(trimmed)) {
                            ex_doc_fence = !ex_doc_fence;
                        }
                    } else if (trimmed.starts_with("@doc") || trimmed.starts_with("@moduledoc"))
                        && trimmed.ends_with("\"\"\"")
                    {
                        ex_in_doc = true;
                    }
                    if ex_in_doc && ex_doc_fence {
                        continue;
                    }
                }
                if is_rust {
                    let is_doc = trimmed.starts_with("///") || trimmed.starts_with("//!");
                    if rs_prev_doc && !is_doc {
                        rs_doc_fence = false;
                    }
                    rs_prev_doc = is_doc;
                    if is_doc && fence_toggle(comment_body(trimmed)) {
                        rs_doc_fence = !rs_doc_fence;
                    }
                    if rs_doc_fence {
                        continue;
                    }
                    let begins_in_string = rs_string_next;
                    // Inside a Rust string literal every `"` is escaped, so a
                    // line's unescaped-quote parity can only come from code
                    // context. Carrying cumulative parity across lines keeps
                    // multi-line string literals (plain or `\`-continued —
                    // help text, embedded examples) from yielding phantom
                    // pointers; a stray odd quote in a comment self-corrects
                    // on the next even line.
                    rs_string_next = begins_in_string != unescaped_quote_parity(line);
                    if begins_in_string {
                        continue;
                    }
                }
            }
            if let Some((kind, payload, closing, marker_end)) = parse_canonical_marker(line) {
                // Token markers carry no UUID; resolve through the store index
                // (backend YAML + legacy JSON + anything minted this run).
                let uuid = match &payload {
                    MarkerPayload::Uuid(uuid) => Some(*uuid),
                    MarkerPayload::Token(code) => token_index.get(code).copied(),
                };
                let Some(uuid) = uuid else {
                    errors.push(format!(
                        "{file_path}:{}: token marker 〚{}:{}〛 has no store record",
                        index + 1,
                        kind.emoji(),
                        payload.as_str()
                    ));
                    continue;
                };
                let code = match &payload {
                    MarkerPayload::Uuid(uuid) => unicode4_encode_uuid(*uuid),
                    MarkerPayload::Token(code) => code.clone(),
                };
                if closing {
                    if !kind.closable() {
                        errors.push(format!(
                            "{file_path}:{}: {} cannot have a closing marker",
                            index + 1,
                            kind.emoji()
                        ));
                    } else if let Some(position) = spans
                        .iter()
                        .rposition(|open| open.0 == kind && open.1 == uuid)
                    {
                        // Spans can overlap: A-open, B-open, A-close, B-close.
                        // Keep each opening's location so repeated components
                        // close the correct occurrence.
                        let (_, _, code, location_index) = spans.remove(position);
                        if let Some(location) = pointers
                            .get_mut(&code)
                            .and_then(|pointer| pointer.locations.get_mut(location_index))
                        {
                            location.end_line = Some(index + 1);
                        }
                    } else {
                        errors.push(format!(
                            "{file_path}:{}: closing marker does not match an opening marker",
                            index + 1
                        ));
                    }
                    continue;
                }

                let rest = clean_comment_tail(&line[marker_end..]);
                let (name, description) = match rest.split_once("::") {
                    Some((name, description)) => {
                        (clean_comment_tail(name), clean_comment_tail(description))
                    }
                    None => (rest, String::new()),
                };
                let name = if name.is_empty() {
                    uuid.to_string()
                } else {
                    name
                };
                let location = Location {
                    file_path: file_path.clone(),
                    line: index + 1,
                    end_line: None,
                };
                let location_index = if let Some(existing) = pointers.get_mut(&code) {
                    if kind == MarkerKind::Component
                        && existing.uuid == Some(uuid)
                        && stored_kind_matches(&existing.kind, kind)
                    {
                        existing.locations.push(location);
                        Some(existing.locations.len() - 1)
                    } else {
                        errors.push(format!(
                            "duplicate pointer {uuid}: {}:{} and {file_path}:{}",
                            existing.path,
                            existing.line,
                            index + 1
                        ));
                        None
                    }
                } else {
                    pointers.insert(
                        code.clone(),
                        Pointer {
                            uuid: Some(uuid),
                            kind: Some(kind.canonical().to_string()),
                            code: code.clone(),
                            path: file_path.clone(),
                            line: index + 1,
                            name,
                            description,
                            locations: vec![location],
                        },
                    );
                    Some(0)
                };
                if kind.closable() {
                    if let Some(location_index) = location_index {
                        spans.push((kind, uuid, code, location_index));
                    }
                }
                continue;
            }
            let Some((code, name, description)) = parse_declaration(line) else {
                continue;
            };
            let pointer = Pointer {
                uuid: None,
                kind: None,
                code: code.clone(),
                path: file_path.clone(),
                line: index + 1,
                name,
                description,
                locations: vec![],
            };
            if let Some(first) = pointers.get(&code) {
                errors.push(format!(
                    "duplicate pointer {code}: {}:{} and {}:{}",
                    first.path, first.line, pointer.path, pointer.line
                ));
            } else {
                pointers.insert(code, pointer);
            }
        }
        for (kind, uuid, _, _) in spans {
            errors.push(format!(
                "{file_path}: unclosed marker 〚{}:{uuid}〛",
                kind.emoji()
            ));
        }
    }
    Ok((pointers, errors))
}

fn parse_canonical_marker(line: &str) -> Option<(MarkerKind, MarkerPayload, bool, usize)> {
    let start = line.find('〚')?;
    if !declaration_context_allows(line, start) {
        return None;
    }
    let body_start = start + '〚'.len_utf8();
    let body_end = body_start + line[body_start..].find('〛')?;
    let body = &line[body_start..body_end];
    let (closing, body) = match body.strip_prefix('/') {
        Some(body) => (true, body),
        None => (false, body),
    };
    let (emoji, payload_text) = body.split_once(':')?;
    let kind = MarkerKind::parse(emoji)?;
    if kind.emoji() != emoji {
        return None;
    }
    let payload = if let Ok(uuid) = Uuid::parse_str(payload_text) {
        if uuid.to_string() != payload_text
            || uuid.get_version_num() != 5
            || uuid.get_variant() != uuid::Variant::RFC4122
        {
            return None;
        }
        MarkerPayload::Uuid(uuid)
    } else if valid_token_glyphs(payload_text) {
        MarkerPayload::Token(payload_text.to_string())
    } else {
        return None;
    };
    Some((kind, payload, closing, body_end + '〛'.len_utf8()))
}

fn parse_declaration(line: &str) -> Option<(String, String, String)> {
    let start = line.find('⟦')?;
    if !declaration_context_allows(line, start) {
        return None;
    }
    let after_start = start + '⟦'.len_utf8();
    let end_offset = line[after_start..].find('⟧')?;
    let end = after_start + end_offset;
    let code = &line[after_start..end];
    if code.chars().count() != 4 || !valid_code(code) {
        return None;
    }
    let rest = line[end + '⟧'.len_utf8()..].trim_start();
    let (name, description) = rest.split_once("::")?;
    let name = clean_comment_tail(name);
    if name.is_empty() {
        return None;
    }
    Some((code.to_string(), name, clean_comment_tail(description)))
}

/// True when the line toggles a fenced code block. A fence that opens and
/// closes on the same line does not change state.
fn fence_toggle(trimmed: &str) -> bool {
    if !trimmed.starts_with("```") {
        return false;
    }
    let body = trimmed.trim_start_matches('`');
    !(body.len() >= 3 && body.ends_with("```"))
}

/// Strip repeating line-comment prefixes so fence detection works on
/// comment-wrapped examples (`/// ```rust`, `# ```elixir`).
fn comment_body(trimmed: &str) -> &str {
    const PREFIXES: [&str; 9] = ["////", "///", "//!", "<!--", "//", "/*", "--", "#", "*"];
    let mut rest = trimmed;
    loop {
        match PREFIXES.iter().find(|prefix| rest.starts_with(**prefix)) {
            Some(prefix) => rest = rest[prefix.len()..].trim_start(),
            None => return rest,
        }
    }
}

/// Whether a line holds an odd number of string-delimiting `"` characters.
/// Escapes consume the next char (`"\\"`, `"a\"b"`), `'"'` char literals are
/// not delimiters, and comment lines never carry string state (doc comments
/// quote examples freely). Used to carry Rust string state across lines so
/// multi-line string literals (help text, embedded examples) don't yield
/// phantom pointers.
fn unescaped_quote_parity(line: &str) -> bool {
    if line.trim_start().starts_with("//") {
        return false;
    }
    let chars: Vec<char> = line.chars().collect();
    let mut odd = false;
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 2,
            '"' => {
                let prev = if index > 0 { chars[index - 1] } else { '\0' };
                let next = chars.get(index + 1).copied().unwrap_or('\0');
                if !(prev == '\'' && next == '\'') {
                    odd = !odd;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    odd
}

fn declaration_context_allows(line: &str, start: usize) -> bool {
    let prefix = &line[..start];
    // An odd number of unescaped double-quotes before the marker means it sits inside
    // a string literal (test fixtures, codegen templates) — never a real declaration.
    // Source files are scanned since the annotate release, so this guard keeps the
    // tool's own fixtures (and any code embedding marker examples in strings) out of
    // the DB. Escaped quotes (\") are literal characters, not string delimiters.
    let mut quotes = 0usize;
    let mut prev = '\0';
    for c in prefix.chars() {
        if c == '"' && prev != '\\' {
            quotes += 1;
        }
        prev = c;
    }
    if quotes % 2 == 1 {
        return false;
    }
    let before = prefix.trim_end();
    if before.is_empty() {
        return true;
    }

    let trimmed = before.trim_start();
    let leading_comment_markers = ["//", "#", "<!--", "/*", "*", "--", ";"];
    if leading_comment_markers
        .iter()
        .any(|marker| trimmed.starts_with(marker))
    {
        return true;
    }

    let inline_comment_markers = ["//", "/*", "<!--", " #", "\t#", " --"];
    if inline_comment_markers
        .iter()
        .any(|marker| trimmed.contains(marker))
    {
        return true;
    }

    false
}

fn clean_comment_tail(value: &str) -> String {
    value
        .trim()
        .trim_end_matches("-->")
        .trim_end_matches("*/")
        .trim()
        .to_string()
}

fn valid_code(code: &str) -> bool {
    code.chars()
        .all(|ch| !ch.is_whitespace() && !"⟦⟧/?#:%".contains(ch))
}

fn scan_files(root: &Path, db_path: &Path, filter: &ScanFilter) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let skip_dirs: HashSet<&str> = [
        ".DS_Store",
        ".Spotlight-V100",
        ".Trashes",
        ".claude",
        ".elixir_ls",
        ".fseventsd",
        ".git",
        ".idea",
        ".next",
        ".vscode",
        "Builds",
        "DerivedData",
        "Library",
        "Logs",
        "Temp",
        "UserSettings",
        "_build",
        ".worktrees",
        "build",
        "coverage",
        "deps",
        "dist",
        "node_modules",
        "obj",
        "staging",
        "target",
    ]
    .into_iter()
    .collect();
    let suffixes: HashSet<&str> = [
        "asmdef", "cs", "css", "ex", "exs", "html", "js", "json", "md", "meta", "mjs", "rs",
        "shader", "ts", "tsx", "txt", "uxml", "yaml", "yml",
    ]
    .into_iter()
    .collect();
    walk_scan(
        root, root, db_path, &skip_dirs, &suffixes, filter, &mut files,
    )?;
    Ok(files)
}

/// Generated/minified artifacts that must never be scanned or annotated.
fn skip_file_name(name: &str) -> bool {
    name == "packages-lock.json"
        || name == "package-lock.json"
        || name.ends_with(".min.js")
        || name.ends_with(".d.ts")
}

#[allow(clippy::too_many_arguments)]
fn walk_scan(
    root: &Path,
    dir: &Path,
    db_path: &Path,
    skip_dirs: &HashSet<&str>,
    suffixes: &HashSet<&str>,
    filter: &ScanFilter,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => return Err(format!("could not read {}: {error}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        if file_type.is_dir() {
            let name = entry.file_name();
            if !skip_dirs.contains(name.to_string_lossy().as_ref()) && filter.allows_dir(&rel) {
                walk_scan(root, &path, db_path, skip_dirs, suffixes, filter, files)?;
            }
            continue;
        }
        if !file_type.is_file() || absolute_path(&path)? == db_path {
            continue;
        }
        if path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(skip_file_name)
        {
            continue;
        }
        if path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| suffixes.contains(extension))
            && path.starts_with(root)
            && filter.allows_file(&rel)
        {
            files.push(path);
        }
    }
    Ok(())
}

fn expansion_index(
    root: &Path,
    db_path: &Path,
    scanned: &HashMap<String, Pointer>,
) -> Result<HashMap<String, Pointer>, String> {
    Ok(merge_expansion_pointers(
        scanned,
        backend_status(root)?,
        legacy_records(db_path)?,
    ))
}

fn merge_expansion_pointers(
    scanned: &HashMap<String, Pointer>,
    stored: Vec<Pointer>,
    legacy: HashMap<String, Pointer>,
) -> HashMap<String, Pointer> {
    let mut index: HashMap<String, Pointer> = HashMap::new();
    let mut by_uuid: HashMap<Uuid, String> = HashMap::new();
    for pointer in legacy
        .into_values()
        .chain(stored)
        .chain(scanned.values().cloned())
    {
        if let Some(previous) = index.remove(&pointer.code) {
            if let Some(uuid) = previous.uuid {
                by_uuid.remove(&uuid);
            }
        }
        if let Some(uuid) = pointer.uuid {
            if let Some(old_code) = by_uuid.insert(uuid, pointer.code.clone()) {
                index.remove(&old_code);
            }
        }
        index.insert(pointer.code.clone(), pointer);
    }
    index
}

fn expand_markdown_links(
    root: &Path,
    pointers: &HashMap<String, Pointer>,
    write: bool,
    filter: &ScanFilter,
) -> Result<(Vec<String>, Vec<String>), String> {
    let mut changed = Vec::new();
    let mut errors = Vec::new();
    for path in scan_files(root, &root.join("__no_db__"), filter)? {
        if path.extension() != Some(OsStr::new("md")) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let mut new_text = String::new();
        let mut in_fence = false;
        for line in text.split_inclusive('\n') {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
                new_text.push_str(line);
            } else if in_fence {
                new_text.push_str(line);
            } else {
                new_text.push_str(&expand_line(root, &path, line, pointers, &mut errors));
            }
        }
        if new_text != text {
            changed.push(rel_path(&path, root));
            if write {
                fs::write(&path, new_text)
                    .map_err(|error| format!("could not write {}: {error}", path.display()))?;
            }
        }
    }
    Ok((changed, errors))
}

fn expand_line(
    root: &Path,
    path: &Path,
    line: &str,
    pointers: &HashMap<String, Pointer>,
    errors: &mut Vec<String>,
) -> String {
    let mut output = String::new();
    let mut remaining = line;
    while let Some(start) = remaining.find("](deeplink:") {
        let before = &remaining[..start];
        let after_prefix = &remaining[start + "](deeplink:".len()..];
        let Some(end) = after_prefix.find(')') else {
            output.push_str(remaining);
            return output;
        };
        let raw_code = &after_prefix[..end];
        let canonical = parse_canonical_marker(raw_code)
            .filter(|(_, _, closing, marker_end)| !closing && *marker_end == raw_code.len());
        let code = normalize_code(raw_code);
        if canonical.is_some() || (code.chars().count() == 4 && valid_code(&code)) {
            output.push_str(before);
            let pointer = match canonical.map(|(kind, payload, _, _)| (kind, payload)) {
                Some((kind, MarkerPayload::Uuid(uuid))) => pointers.values().find(|pointer| {
                    pointer.uuid == Some(uuid) && stored_kind_matches(&pointer.kind, kind)
                }),
                Some((kind, MarkerPayload::Token(token))) => pointers
                    .get(&token)
                    .filter(|pointer| stored_kind_matches(&pointer.kind, kind)),
                None => pointers.get(&code),
            };
            if let Some(pointer) = pointer {
                output.push_str(&format!("]({})", expanded_target(pointer)));
            } else {
                errors.push(format!(
                    "{}: unresolved deeplink:{raw_code}",
                    rel_path(path, root)
                ));
                output.push_str("](deeplink:");
                output.push_str(raw_code);
                output.push(')');
            }
            remaining = &after_prefix[end + 1..];
        } else {
            output.push_str(&remaining[..start + "](deeplink:".len()]);
            remaining = after_prefix;
        }
    }
    output.push_str(remaining);
    output
}

fn normalize_code(raw: &str) -> String {
    raw.strip_prefix('⟦')
        .and_then(|value| value.strip_suffix('⟧'))
        .unwrap_or(raw)
        .to_string()
}

fn expanded_target(pointer: &Pointer) -> String {
    // Derivation key is `{path}::{name}` so same-named functions in different
    // files never share a UUID.
    let uuid = pointer.uuid.unwrap_or_else(|| {
        Uuid::new_v5(
            &DOC_POINTER_NAMESPACE,
            uuid5_name(&format!("{}::{}", pointer.path, pointer.name), "", 0).as_bytes(),
        )
    });
    format!("{}:{}?pointer={uuid}", pointer.path, pointer.line)
}

fn legacy_db_path(root: &Path, db: &str) -> Result<PathBuf, String> {
    if db != DEFAULT_DB_PATH {
        return Err(format!(
            "--db {db} is unsupported: stores now live in each repository's .meta/pointers.yaml; migrate custom JSON explicitly before running this CLI"
        ));
    }
    absolute_path(&root.join(DEFAULT_DB_PATH))
}

fn legacy_records(db_path: &Path) -> Result<HashMap<String, Pointer>, String> {
    let text = match fs::read_to_string(db_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(format!("could not read {}: {error}", db_path.display())),
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("invalid legacy JSON {}: {error}", db_path.display()))?;
    let entries = value
        .as_object()
        .ok_or_else(|| format!("legacy JSON {} must be an object", db_path.display()))?;
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for (code, data) in entries {
        *name_counts.entry(legacy_key(code, data)).or_default() += 1;
    }
    let mut pointers = HashMap::new();
    for (code, data) in entries {
        let path = data
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("legacy pointer {code} in {} has no path", db_path.display()))?;
        let name = data
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .unwrap_or(code);
        // Legacy JSON has no UUID field. The derivation key is `{path}::{name}`
        // so same-named functions in different files stay distinct; a key that
        // still repeats (same name twice in one file) binds its UUID to the
        // stable token.
        let uuid_name = if name_counts[&legacy_key(code, data)] == 1 {
            uuid5_name(&format!("{path}::{name}"), "", 0)
        } else {
            format!("doc-pointers:legacy-token:{code}")
        };
        pointers.insert(
            code.clone(),
            Pointer {
                uuid: Some(Uuid::new_v5(&DOC_POINTER_NAMESPACE, uuid_name.as_bytes())),
                kind: None,
                code: code.clone(),
                path: path.to_string(),
                line: data.get("line").and_then(Value::as_u64).unwrap_or(0) as usize,
                name: name.to_string(),
                description: data
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                locations: vec![],
            },
        );
    }
    Ok(pointers)
}

/// Legacy ambiguity key: `{path}::{name}`, falling back to the token when the
/// JSON entry has no usable name.
fn legacy_key(code: &str, data: &Value) -> String {
    let name = data
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or(code);
    format!(
        "{}::{name}",
        data.get("path").and_then(Value::as_str).unwrap_or("")
    )
}

fn pointer_record(pointer: &Pointer) -> Value {
    let mut record = json!({
        "token": pointer.code,
        "file_path": pointer.path,
        "function": pointer.name,
        "description": pointer.description,
        "line": pointer.line,
    });
    if let Some(uuid) = pointer.uuid {
        record["uuid"] = json!(uuid.to_string());
    }
    if let Some(kind) = &pointer.kind {
        record["kind"] = json!(kind);
    }
    if !pointer.locations.is_empty() {
        let mut locations = pointer.locations.clone();
        locations.sort_by(|a, b| (&a.file_path, a.line).cmp(&(&b.file_path, b.line)));
        let values: Vec<Value> = locations
            .iter()
            .map(|location| {
                let mut value = json!({"file_path": location.file_path, "line": location.line});
                if let Some(end_line) = location.end_line {
                    value["end_line"] = json!(end_line);
                }
                value
            })
            .collect();
        record["file_path"] = json!(locations[0].file_path);
        record["line"] = json!(locations[0].line);
        record["locations"] = json!(values);
    }
    record
}

fn hydrate_pointer_ids(
    root: &Path,
    db_path: &Path,
    pointers: &mut HashMap<String, Pointer>,
) -> Result<(), String> {
    let status: HashMap<String, Pointer> = backend_status(root)?
        .into_iter()
        .map(|pointer| (pointer.code.clone(), pointer))
        .collect();
    let legacy = legacy_records(db_path)?;
    for (code, pointer) in pointers {
        if pointer.uuid.is_none() {
            pointer.uuid = status
                .get(code)
                .and_then(|existing| existing.uuid)
                .or_else(|| legacy.get(code).and_then(|old| old.uuid))
                .or_else(|| {
                    Some(Uuid::new_v5(
                        &DOC_POINTER_NAMESPACE,
                        uuid5_name(&format!("{}::{}", pointer.path, pointer.name), "", 0)
                            .as_bytes(),
                    ))
                });
        }
        if pointer.kind.is_none() {
            pointer.kind = status
                .get(code)
                .and_then(|existing| existing.kind.clone())
                .or_else(|| Some("function".to_string()));
        }
    }
    Ok(())
}

fn reconcile_records(
    root: &Path,
    db_path: &Path,
    scanned: &HashMap<String, Pointer>,
    filter: &ScanFilter,
) -> Result<Vec<Value>, String> {
    let status = backend_status(root)?;
    let known: HashSet<String> = status.iter().map(|p| p.code.clone()).collect();
    let by_uuid: HashMap<Uuid, Pointer> = status
        .into_iter()
        .filter_map(|pointer| pointer.uuid.map(|uuid| (uuid, pointer)))
        .collect();
    let mut records = legacy_records(db_path)?;
    records.retain(|code, _| !known.contains(code));
    for (code, pointer) in scanned {
        let mut pointer = pointer.clone();
        if let Some(uuid) = pointer.uuid {
            if let Some(existing) = by_uuid.get(&uuid) {
                pointer.code = existing.code.clone();
                if stored_kind_matches(&pointer.kind, MarkerKind::Component) {
                    pointer.locations.extend(
                        existing
                            .locations
                            .iter()
                            .filter(|location| !filter.allows_file(Path::new(&location.file_path)))
                            .cloned(),
                    );
                }
            } else if let Some(legacy) = records.values().find(|record| record.uuid == Some(uuid)) {
                pointer.code = legacy.code.clone();
            }
            records.retain(|_, record| record.uuid != Some(uuid));
        } else if !known.contains(code) {
            pointer.uuid = records.get(code).and_then(|legacy| legacy.uuid);
        }
        records.insert(pointer.code.clone(), pointer);
    }
    let mut entries: Vec<_> = records.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries
        .iter()
        .map(|(_, pointer)| pointer_record(pointer))
        .collect())
}

fn backend_status(root: &Path) -> Result<Vec<Pointer>, String> {
    let response = backend_request(root, &json!({"op": "status"}))?;
    let records = response
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(|| "backend status omitted records".to_string())?;
    records
        .iter()
        .map(|record| {
            let text = |key| {
                record
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| format!("backend record omitted {key}"))
            };
            Ok(Pointer {
                uuid: Some(Uuid::parse_str(&text("uuid")?).map_err(|error| error.to_string())?),
                kind: record
                    .get("kind")
                    .and_then(Value::as_str)
                    .and_then(parse_kind_string),
                code: text("token")?,
                path: text("file_path")?,
                line: record.get("line").and_then(Value::as_u64).unwrap_or(0) as usize,
                name: text("function")?,
                description: text("description")?,
                locations: record
                    .get("locations")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .map(|item| {
                                Ok(Location {
                                    file_path: item
                                        .get("file_path")
                                        .and_then(Value::as_str)
                                        .ok_or_else(|| {
                                            "backend location omitted file_path".to_string()
                                        })?
                                        .to_string(),
                                    line: item.get("line").and_then(Value::as_u64).unwrap_or(0)
                                        as usize,
                                    end_line: item
                                        .get("end_line")
                                        .and_then(Value::as_u64)
                                        .map(|value| value as usize),
                                })
                            })
                            .collect::<Result<Vec<_>, String>>()
                    })
                    .transpose()?
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// Token -> UUID resolution index from persisted stores (backend YAML + legacy
/// JSON). Token-form markers embed no UUID; this map is how they resolve.
fn store_token_index(root: &Path, db_path: &Path) -> Result<HashMap<String, Uuid>, String> {
    let mut index = HashMap::new();
    for pointer in legacy_records(db_path)?.into_values() {
        if let Some(uuid) = pointer.uuid {
            index.entry(pointer.code).or_insert(uuid);
        }
    }
    for pointer in backend_status(root)? {
        if let Some(uuid) = pointer.uuid {
            index.insert(pointer.code, uuid);
        }
    }
    Ok(index)
}

fn backend_reconcile(root: &Path, records: &[Value], write: bool) -> Result<bool, String> {
    let response = backend_request(
        root,
        &json!({"op": "reconcile", "records": records, "write": write}),
    )?;
    response
        .get("changed")
        .and_then(Value::as_bool)
        .ok_or_else(|| "backend reconcile omitted changed flag".to_string())
}

fn backend_request(root: &Path, request: &Value) -> Result<Value, String> {
    let project = env::var_os("DOC_POINTERS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(built_project_path);
    if !project.join("mix.exs").is_file() {
        return Err(format!(
            "Elixir backend not found at {}; set DOC_POINTERS_HOME to the doc-pointers checkout",
            project.display()
        ));
    }
    let mut child = Command::new("mix")
        .args(["doc_pointers.cli", "--root"])
        .arg(root)
        .current_dir(&project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start Elixir backend: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "could not open backend stdin".to_string())?
        .write_all(format!("{request}\n").as_bytes())
        .map_err(|error| format!("could not write backend request: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("backend returned invalid UTF-8: {error}"))?;
    let response_line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let response: Value = serde_json::from_str(response_line).map_err(|error| {
        format!(
            "invalid backend response: {error}; stderr: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    })?;
    if !output.status.success() || response.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Elixir backend failed")
            .to_string());
    }
    Ok(response)
}

fn built_project_path() -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Rust crate has a parent")
        .to_path_buf();
    if source.join("mix.exs").is_file() {
        return source;
    }

    // An installed binary may outlive the feature worktree where it was built.
    // Fall back to that worktree's canonical checkout after the change lands.
    source
        .ancestors()
        .find(|path| {
            path.file_name() == Some(OsStr::new("worktrees"))
                && path.parent().and_then(Path::file_name) == Some(OsStr::new(".claude"))
        })
        .and_then(|path| path.parent()?.parent())
        .map(Path::to_path_buf)
        .unwrap_or(source)
}

fn install_hook(root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .map_err(|error| format!("git directory not found; cannot install hook: {error}"))?;
    if !output.status.success() {
        return Err("git directory not found; cannot install hook".to_string());
    }
    let git_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let hook = PathBuf::from(git_dir).join("hooks/pre-commit");
    let marker = "# therobotdrafts-doc-pointers";
    if hook.exists() {
        let existing = fs::read_to_string(&hook).unwrap_or_default();
        if !existing.contains(marker) {
            return Err(format!(
                "{} already exists and is not managed by this script",
                hook.display()
            ));
        }
    }
    let executable = env::current_exe()
        .map_err(|error| format!("could not locate doc-pointers executable: {error}"))?;
    let body = format!(
        "#!/bin/sh\n{marker}\nset -eu\nexec {} build --root {} --check\n",
        shell_quote(&executable),
        shell_quote(root)
    );
    if let Some(parent) = hook.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&hook, body).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&hook)
            .map_err(|error| error.to_string())?
            .permissions();
        permissions.set_mode(permissions.mode() | 0o111);
        fs::set_permissions(&hook, permissions).map_err(|error| error.to_string())?;
    }
    println!("installed {}", hook.display());
    Ok(())
}

fn parse_namespace(raw: &str) -> Result<Uuid, String> {
    match raw.to_ascii_lowercase().as_str() {
        "doc-pointers" => Ok(DOC_POINTER_NAMESPACE),
        "dns" => Ok(Uuid::NAMESPACE_DNS),
        "url" => Ok(Uuid::NAMESPACE_URL),
        "oid" => Ok(Uuid::NAMESPACE_OID),
        "x500" => Ok(Uuid::NAMESPACE_X500),
        _ => Uuid::parse_str(raw).map_err(|error| format!("invalid UUID namespace: {error}")),
    }
}

fn generate_uuid5_code(
    name: &str,
    namespace: Uuid,
    salt: &str,
    pointers: &HashMap<String, Pointer>,
) -> Result<(String, Uuid, String, usize), String> {
    for attempt in 0..10000 {
        let uuid_name = uuid5_name(name, salt, attempt);
        let value = Uuid::new_v5(&namespace, uuid_name.as_bytes());
        let code = unicode4_encode_uuid(value);
        if !pointers.contains_key(&code) {
            return Ok((code, value, uuid_name, attempt));
        }
    }
    Err("could not generate an unused pointer code after 10000 attempts".to_string())
}

/// Resolve the token/UUID for a target `{relpath}::{name}` before insertion.
/// An existing store record (matched by derivation UUID, or by file+name for
/// legacy bare-name records) is reused so re-annotation keeps its historical
/// token instead of minting a collision-bumped successor; `--force-remint`
/// skips reuse and always mints fresh. Records minted earlier in this run are
/// excluded from reuse so repeated names within one file still get distinct
/// tokens via the collision counter.
fn resolve_annotate_pointer(
    seed: &str,
    relpath: &str,
    name: &str,
    pointers: &HashMap<String, Pointer>,
    minted: &HashMap<String, Uuid>,
    force_remint: bool,
) -> Result<(String, Uuid), String> {
    if !force_remint {
        let derivation = Uuid::new_v5(&DOC_POINTER_NAMESPACE, uuid5_name(seed, "", 0).as_bytes());
        if let Some(existing) = pointers.values().find(|pointer| {
            pointer.uuid == Some(derivation)
                || (pointer.path == relpath
                    && pointer.name == name
                    && !minted.contains_key(&pointer.code))
        }) {
            if let Some(uuid) = existing.uuid {
                return Ok((existing.code.clone(), uuid));
            }
        }
    }
    let (code, uuid, _, _) = generate_uuid5_code(seed, DOC_POINTER_NAMESPACE, "", pointers)?;
    Ok((code, uuid))
}

fn uuid5_name(name: &str, salt: &str, attempt: usize) -> String {
    let mut parts = vec!["doc-pointers".to_string(), name.to_string()];
    if !salt.is_empty() {
        parts.push(salt.to_string());
    }
    if attempt > 0 {
        parts.push(attempt.to_string());
    }
    parts.join(":")
}

fn unicode4_encode_uuid(value: Uuid) -> String {
    let mut number = u128::from_be_bytes(*value.as_bytes());
    let mut chars = Vec::new();
    for _ in 0..TOKEN_LENGTH {
        let index = (number % TOKEN_SIZE) as u32;
        number /= TOKEN_SIZE;
        chars.push(token_char_from_index(index));
    }
    chars.into_iter().rev().collect()
}

fn token_char_from_index(mut index: u32) -> char {
    for &(start, end) in TOKEN_RANGES.iter() {
        let range_size = end - start + 1;
        if index < range_size {
            return char::from_u32(start + index).expect("valid token code point");
        }
        index -= range_size;
    }
    panic!("token alphabet index out of range");
}

fn format_pointer(
    format: PointerFormat,
    kind: &str,
    code: &str,
    name: Option<&str>,
    description: &str,
) -> Result<String, String> {
    // Emitted markers embed the 4-glyph token; full-UUID payloads stay
    // accepted on lookup, they are just no longer what we print.
    let marker = format!("〚{}:{code}〛", emoji_for_kind(kind));
    let payload = match format {
        PointerFormat::Marker => marker,
        PointerFormat::Code => code.to_string(),
        PointerFormat::Deeplink => format!("deeplink:{marker}"),
        PointerFormat::Declaration => {
            let name = name.ok_or_else(|| "--format declaration requires NAME".to_string())?;
            format!("{marker} {name} :: {description}")
                .trim_end()
                .to_string()
        }
    };
    Ok(payload)
}

fn copy_to_clipboard(payload: &str) -> Result<(), String> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run pbcopy: {error}"))?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| "could not open pbcopy stdin".to_string())?
        .write_all(payload.as_bytes())
        .map_err(|error| format!("could not write to pbcopy: {error}"))?;
    let status = child.wait().map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("pbcopy exited with status {status}"))
    }
}

fn shell_quote(path: &Path) -> String {
    let value = path.display().to_string();
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn rel_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        fs::canonicalize(path)
            .map_err(|error| format!("could not resolve {}: {error}", path.display()))
    } else if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        env::current_dir()
            .map_err(|error| error.to_string())
            .map(|cwd| cwd.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_declaration_accepts_bare_and_comment_lines() {
        assert_eq!(
            parse_declaration("⟦DPTR⟧ Documentation pointer convention :: Defines hard pointers."),
            Some((
                "DPTR".to_string(),
                "Documentation pointer convention".to_string(),
                "Defines hard pointers.".to_string()
            ))
        );
        assert_eq!(
            parse_declaration("// ⟦ABCD⟧ Pointer routing :: Centralizes pointer input."),
            Some((
                "ABCD".to_string(),
                "Pointer routing".to_string(),
                "Centralizes pointer input.".to_string()
            ))
        );
    }

    #[test]
    fn parse_declaration_ignores_quoted_prompt_examples() {
        let line = "            \"\\\"⟦𓅕𓀦𓈽𓆡⟧ Name :: uuid5:01234567-89ab-cdef-0123-456789abcdef\\\", remove that line\"";
        assert_eq!(parse_declaration(line), None);
    }

    #[test]
    fn parse_declaration_ignores_markers_inside_string_literals() {
        // Source files are scanned now; fixture strings like these must never register.
        assert_eq!(
            parse_declaration("        parse_declaration(\"// ⟦ABCD⟧ Name :: Desc\"),"),
            None
        );
        assert_eq!(
            parse_declaration("            \"/// ⟦ABCD⟧ alpha :: existing marker\","),
            None
        );
        assert_eq!(
            parse_declaration("            \"<!-- ⟦REAL⟧ Real pointer :: fixture -->\\n\","),
            None
        );
        // Balanced quotes before the marker keep real comments working.
        assert_eq!(
            parse_declaration("// \"quoted\" ⟦ABCD⟧ Name :: Desc"),
            Some(("ABCD".to_string(), "Name".to_string(), "Desc".to_string()))
        );
    }

    #[test]
    fn parse_declaration_ignores_non_comment_code_prefixes() {
        assert_eq!(
            parse_declaration("let marker = \"⟦ABCD⟧ Name :: Description\";"),
            None
        );
        assert_eq!(
            parse_declaration("value * \"⟦ABCD⟧ Name :: Description\""),
            None
        );
    }

    #[test]
    fn canonical_marker_parses_seven_kinds_and_rejects_uppercase_uuid() {
        let uuid = Uuid::new_v5(
            &DOC_POINTER_NAMESPACE,
            b"canonical_marker_parses_seven_kinds_and_rejects_uppercase_uuid",
        );
        for emoji in ["📁", "📦", "🔌", "🧩", "🔧", "🔀", "📐"] {
            let line = format!("# 〚{emoji}:{uuid}〛 Name :: Description");
            let (kind, parsed, closing, _) = parse_canonical_marker(&line).unwrap();
            assert_eq!(kind.emoji(), emoji);
            assert_eq!(parsed, MarkerPayload::Uuid(uuid));
            assert!(!closing);
        }
        assert!(
            parse_canonical_marker(&format!("# 〚📐:{}〛", uuid.to_string().to_uppercase()))
                .is_none()
        );
        let v4 = Uuid::new_v4();
        assert!(parse_canonical_marker(&format!("# 〚📐:{v4}〛")).is_none());
        assert!(parse_lookup_key(&v4.to_string()).is_err());
        assert_eq!(
            parse_lookup_key(&format!("〚🔧:{uuid}〛")).unwrap(),
            (Some(MarkerKind::Function), MarkerPayload::Uuid(uuid))
        );
        assert_eq!(
            parse_lookup_key(&uuid.to_string()).unwrap(),
            (None, MarkerPayload::Uuid(uuid))
        );
    }

    #[test]
    fn token_markers_parse_and_require_store_resolution() {
        // The canonical golden token — embeds are 4 glyphs, never the UUID.
        let line = "# 〚🔧:𓳔𔐮𔘟𔄵〛 TestPointer :: description";
        let (kind, parsed, closing, _end) = parse_canonical_marker(line).unwrap();
        assert_eq!(kind, MarkerKind::Function);
        assert_eq!(parsed, MarkerPayload::Token("𓳔𔐮𔘟𔄵".to_string()));
        assert!(!closing);

        let (_, closing_parsed, closing, _) =
            parse_canonical_marker("<!-- 〚/🧩:𓳔𔐮𔘟𔄵〛 -->").unwrap();
        assert_eq!(closing_parsed, MarkerPayload::Token("𓳔𔐮𔘟𔄵".to_string()));
        assert!(closing);

        // Exactly four glyphs from the hieroglyph ranges — not three, not ASCII.
        assert!(parse_canonical_marker("# 〚🔧:𓳔𔐮𔘟〛 short :: x").is_none());
        assert!(parse_canonical_marker("# 〚🔧:abCD〛 ascii :: x").is_none());
        assert!(parse_canonical_marker("# 〚🔧:𓳔𔐮𔘟𔄵𓳔〛 extra :: x").is_none());
        assert_eq!(
            parse_lookup_key("〚🔧:𓳔𔐮𔘟𔄵〛").unwrap(),
            (
                Some(MarkerKind::Function),
                MarkerPayload::Token("𓳔𔐮𔘟𔄵".to_string())
            )
        );

        // Token markers carry no UUID: collect resolves them through the index.
        let uuid = Uuid::new_v5(&DOC_POINTER_NAMESPACE, b"token-marker-resolution");
        let token = unicode4_encode_uuid(uuid);
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("a.ex"),
            format!("# 〚🔧:{token}〛 run :: Runs the task\ndef run, do: :ok\n"),
        )
        .unwrap();
        let mut index = HashMap::new();
        index.insert(token.clone(), uuid);
        let (pointers, errors) = collect_pointers(
            &root,
            &root.join(DEFAULT_DB_PATH),
            &ScanFilter::default(),
            &index,
        )
        .unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(pointers[&token].uuid, Some(uuid));

        // An unknown token fails loudly instead of minting a phantom pointer.
        fs::write(
            root.join("b.ex"),
            format!(
                "# 〚🔧:{token}〛 fine\n# 〚🔧:𓀀𓀻𓃉𓏦〛 stranger :: unknown\ndef other, do: :ok\n"
            ),
        )
        .unwrap();
        let (_, errors) = collect_pointers(
            &root,
            &root.join(DEFAULT_DB_PATH),
            &ScanFilter::default(),
            &index,
        )
        .unwrap();
        assert!(errors.iter().any(|error| error.contains("no store record")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn kind_strings_normalize_and_match_marker_scope() {
        let cases = [
            ("file", "file"),
            ("module", "module"),
            ("class", "class"),
            ("struct", "struct"),
            ("interface", "interface"),
            ("protocol", "protocol"),
            ("behaviour", "behaviour"),
            ("behavior", "behaviour"),
            ("function", "function"),
            ("logic", "logic"),
            ("component", "component"),
            ("diagram", "diagram"),
            ("contract", "interface"),
            ("mermaid", "diagram"),
            ("plantuml", "diagram"),
            ("📁", "file"),
            ("📦", "module"),
            ("🔌", "interface"),
            ("🧩", "component"),
            ("🔧", "function"),
            ("🔀", "logic"),
            ("📐", "diagram"),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_kind_string(input).as_deref(), Some(expected));
        }
        assert_eq!(parse_kind_string("widget"), None);

        assert_eq!(emoji_for_kind("class"), "📦");
        assert_eq!(emoji_for_kind("protocol"), "🔌");
        assert_eq!(emoji_for_kind("behaviour"), "🔌");
        assert!(kind_is_closable("component"));
        assert!(!kind_is_closable("module"));

        let stored = Some("class".to_string());
        assert!(stored_kind_matches(&stored, MarkerKind::Module));
        assert!(!stored_kind_matches(&stored, MarkerKind::Function));
        assert!(stored_kind_matches(&None, MarkerKind::Function) == false);
    }

    #[test]
    fn repeated_component_spans_collect_all_locations() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let uuid = Uuid::new_v5(
            &DOC_POINTER_NAMESPACE,
            b"repeated_component_spans_collect_all_locations",
        );
        for file in ["a.md", "b.md"] {
            fs::write(
                root.join(file),
                format!(
                    "<!-- 〚🧩:{uuid}〛 shared :: component -->\nbody\n<!-- 〚/🧩:{uuid}〛 -->\n"
                ),
            )
            .unwrap();
        }
        let (pointers, errors) = collect_pointers(
            &root,
            &root.join(DEFAULT_DB_PATH),
            &ScanFilter::default(),
            &HashMap::new(),
        )
        .unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        let pointer = &pointers[&unicode4_encode_uuid(uuid)];
        assert_eq!(pointer.kind.as_deref(), Some("component"));
        assert_eq!(pointer.locations.len(), 2);
        assert!(pointer
            .locations
            .iter()
            .all(|location| location.end_line == Some(3)));
        let record = pointer_record(pointer);
        assert_eq!(record["locations"].as_array().unwrap().len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn overlapping_component_spans_close_by_uuid() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let a = Uuid::new_v5(&DOC_POINTER_NAMESPACE, b"overlapping-component-a");
        let b = Uuid::new_v5(&DOC_POINTER_NAMESPACE, b"overlapping-component-b");
        fs::write(
            root.join("a.md"),
            format!("# 〚🧩:{a}〛 A :: first\nA and B\n# 〚🧩:{b}〛 B :: second\nA and B\n# 〚/🧩:{a}〛\nB only\n# 〚/🧩:{b}〛\n"),
        )
        .unwrap();
        let (pointers, errors) = collect_pointers(
            &root,
            &root.join(DEFAULT_DB_PATH),
            &ScanFilter::default(),
            &HashMap::new(),
        )
        .unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            pointers[&unicode4_encode_uuid(a)].locations[0].end_line,
            Some(5)
        );
        assert_eq!(
            pointers[&unicode4_encode_uuid(b)].locations[0].end_line,
            Some(7)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn yaml_only_uuid_deeplink_expands() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("lib")).unwrap();
        fs::write(root.join("lib/anchor.rs"), "pub fn anchor() {}\n").unwrap();
        let uuid = Uuid::new_v5(&DOC_POINTER_NAMESPACE, b"yaml-only-uuid-deeplink");
        let token = unicode4_encode_uuid(uuid);
        let record = json!({
            "uuid": uuid.to_string(), "token": token, "kind": "🔧",
            "file_path": "lib/anchor.rs", "line": 1,
            "function": "anchor", "description": "YAML only"
        });
        backend_reconcile(&root, &[record], true).unwrap();
        fs::write(
            root.join("reference.md"),
            format!("[anchor](deeplink:〚🔧:{uuid}〛)\n"),
        )
        .unwrap();
        let (scanned, errors) = collect_pointers(
            &root,
            &root.join(DEFAULT_DB_PATH),
            &ScanFilter::default(),
            &HashMap::new(),
        )
        .unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(scanned.is_empty());
        let index = expansion_index(&root, &root.join(DEFAULT_DB_PATH), &scanned).unwrap();
        let (changed, errors) =
            expand_markdown_links(&root, &index, true, &ScanFilter::default()).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(changed, vec!["reference.md"]);
        assert_eq!(
            fs::read_to_string(root.join("reference.md")).unwrap(),
            format!("[anchor](lib/anchor.rs:1?pointer={uuid})\n")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mismatched_closing_marker_is_an_error() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let uuid = Uuid::new_v5(
            &DOC_POINTER_NAMESPACE,
            b"mismatched_closing_marker_is_an_error",
        );
        fs::write(
            root.join("a.md"),
            format!("# 〚📐:{uuid}〛 diagram :: graph\n# 〚/🔀:{uuid}〛\n"),
        )
        .unwrap();
        let (_, errors) = collect_pointers(
            &root,
            &root.join(DEFAULT_DB_PATH),
            &ScanFilter::default(),
            &HashMap::new(),
        )
        .unwrap();
        assert!(errors.iter().any(|error| error.contains("does not match")));
        assert!(errors.iter().any(|error| error.contains("unclosed")));
        fs::remove_dir_all(root).unwrap();
    }

    // MUST match repo-lock's glyph.rs golden test (golden_matches_doc_pointers_fixture) —
    // these constants are a cross-crate pact guarding drift between the duplicated encoders.
    #[test]
    fn generated_token_matches_unity_fixture() {
        let uuid = Uuid::new_v5(
            &DOC_POINTER_NAMESPACE,
            "doc-pointers:TestPointer".as_bytes(),
        );
        assert_eq!(uuid.to_string(), "5c692577-ad0c-51f1-992c-759b5e5fffb5");
        assert_eq!(unicode4_encode_uuid(uuid), "𓳔𔐮𔘟𔄵");
        // Sign-alphabet residue the four glyphs display (u128 big-endian mod 5744^4);
        // codepoints U+13CD4 U+1442E U+1461F U+14135.
        let residue = u128::from_be_bytes(*uuid.as_bytes()) % TOKEN_SIZE.pow(TOKEN_LENGTH as u32);
        assert_eq!(residue, 619_504_546_873_269);
        let codepoints: Vec<String> = unicode4_encode_uuid(uuid)
            .chars()
            .map(|c| format!("U+{:X}", c as u32))
            .collect();
        assert_eq!(codepoints.join(" "), "U+13CD4 U+1442E U+1461F U+14135");
    }

    #[test]
    fn legacy_record_keeps_uuid5_identity() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("docs")).unwrap();
        let db = root.join(DEFAULT_DB_PATH);
        fs::write(
            &db,
            r#"{"ABCD":{"path":"docs/old.md","name":"legacy","description":"old"}}"#,
        )
        .unwrap();
        let pointer = legacy_records(&db).unwrap().remove("ABCD").unwrap();
        assert_eq!(
            pointer.uuid,
            Some(Uuid::new_v5(
                &DOC_POINTER_NAMESPACE,
                uuid5_name("docs/old.md::legacy", "", 0).as_bytes()
            ))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_duplicate_names_get_distinct_stable_uuids() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("docs")).unwrap();
        let db = root.join(DEFAULT_DB_PATH);
        fs::write(
            &db,
            r#"{"ABCD":{"path":"docs/a.md","name":"shared"},"EFGH":{"path":"docs/b.md","name":"shared"},"IJKL":{"path":"docs/c.md","name":"unique"}}"#,
        )
        .unwrap();
        let first = legacy_records(&db).unwrap();
        assert_ne!(first["ABCD"].uuid, first["EFGH"].uuid);
        assert_eq!(
            first["ABCD"].uuid,
            Some(Uuid::new_v5(
                &DOC_POINTER_NAMESPACE,
                uuid5_name("docs/a.md::shared", "", 0).as_bytes()
            ))
        );
        assert_eq!(
            first["IJKL"].uuid,
            Some(Uuid::new_v5(
                &DOC_POINTER_NAMESPACE,
                uuid5_name("docs/c.md::unique", "", 0).as_bytes()
            ))
        );
        fs::write(
            &db,
            r#"{"IJKL":{"path":"docs/c.md","name":"unique"},"EFGH":{"path":"docs/b.md","name":"shared"},"ABCD":{"path":"docs/a.md","name":"shared"}}"#,
        )
        .unwrap();
        let second = legacy_records(&db).unwrap();
        assert_eq!(first["ABCD"].uuid, second["ABCD"].uuid);
        assert_eq!(first["EFGH"].uuid, second["EFGH"].uuid);
        // A key that still repeats within one file binds its UUID to the token.
        fs::write(
            &db,
            r#"{"MNOP":{"path":"docs/d.md","name":"twin"},"QRST":{"path":"docs/d.md","name":"twin"}}"#,
        )
        .unwrap();
        let twins = legacy_records(&db).unwrap();
        assert_eq!(
            twins["MNOP"].uuid,
            Some(Uuid::new_v5(
                &DOC_POINTER_NAMESPACE,
                b"doc-pointers:legacy-token:MNOP"
            ))
        );
        assert_ne!(twins["MNOP"].uuid, twins["QRST"].uuid);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn collect_pointers_skips_spotlight_cache() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".Spotlight-V100/cache")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(
            root.join(".Spotlight-V100/cache/noise.txt"),
            "⟦NOIS⟧ Noise :: Should be ignored.\n",
        )
        .unwrap();
        fs::write(
            root.join("docs/real.md"),
            "<!-- ⟦REAL⟧ Real pointer :: Should be indexed. -->\n",
        )
        .unwrap();

        let (pointers, errors) = collect_pointers(
            &root,
            &root.join("docs/doc-pointer-db.json"),
            &ScanFilter::default(),
            &HashMap::new(),
        )
        .unwrap();
        assert!(errors.is_empty());
        assert!(pointers.contains_key("REAL"));
        assert!(!pointers.contains_key("NOIS"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_pointers_skips_fenced_examples_in_doc_heredocs() {
        // Fenced examples inside an @doc heredoc are documentation, not real
        // pointers; markers on surrounding heredoc lines still parse.
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("lib")).unwrap();
        fs::write(
            root.join("lib/cart.ex"),
            "defmodule Cart do\n  @doc \"\"\"\n  Adds an item.\n\n  ## Example\n\n      ```elixir\n      ⟦FAKE⟧ fake :: Fenced example only.\n      ```\n\n  ⟦REAL⟧ add :: Real marker in the docstring.\n  \"\"\"\n  def add(cart, item), do: cart\nend\n",
        )
        .unwrap();

        let (pointers, errors) = collect_pointers(
            &root,
            &root.join("docs/doc-pointer-db.json"),
            &ScanFilter::default(),
            &HashMap::new(),
        )
        .unwrap();
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert!(pointers.contains_key("REAL"));
        assert!(!pointers.contains_key("FAKE"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_pointers_skips_rustdoc_fenced_examples() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "/// Adds numbers.\n///\n/// ```text\n/// ⟦FAKE⟧ fake :: Fenced example only.\n/// ```\n///\n/// ⟦REAL⟧ add :: Real marker in the doc comment.\npub fn add(a: u8, b: u8) -> u8 {\n    a + b\n}\n",
        )
        .unwrap();

        let (pointers, errors) = collect_pointers(
            &root,
            &root.join("docs/doc-pointer-db.json"),
            &ScanFilter::default(),
            &HashMap::new(),
        )
        .unwrap();
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert!(pointers.contains_key("REAL"));
        assert!(!pointers.contains_key("FAKE"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn collect_pointers_skips_multiline_string_contents() {
        // `\`-continued string literals (help text, embedded examples) must not
        // yield phantom pointers — the scanner's own help text used to trip this.
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/help.rs"),
            "pub fn print_help() {\n    println!(\"\\\n    usage: tool [options]\\n\\\n\\\n      ⟦FAKE⟧ fake :: Help text, not a pointer.\\n\\\n    \");\n}\n\n// ⟦REAL⟧ real :: Real pointer in a comment.\n",
        )
        .unwrap();

        let (pointers, errors) = collect_pointers(
            &root,
            &root.join("docs/doc-pointer-db.json"),
            &ScanFilter::default(),
            &HashMap::new(),
        )
        .unwrap();
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert!(pointers.contains_key("REAL"));
        assert!(!pointers.contains_key("FAKE"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_filter_excludes_worktrees_and_staging_by_default() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        for (dir, token) in [
            ("src", "TOKN"),
            (".claude/worktrees/feature", "TOKN"),
            (".worktrees/legacy", "TOKN"),
            ("myrepo.worktrees/sibling", "TOKN"),
            ("staging/experiment", "TOKN"),
            ("vendor/generated", "VEND"),
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
            fs::write(
                root.join(dir).join("note.md"),
                format!("<!-- ⟦{token}⟧ sample :: Sample pointer. -->\n"),
            )
            .unwrap();
        }

        // Defaults: worktree/staging duplicates are skipped everywhere; normal
        // trees (src/, vendor/) still scan.
        let (pointers, errors) = collect_pointers(
            &root,
            &root.join("docs/doc-pointer-db.json"),
            &ScanFilter::default(),
            &HashMap::new(),
        )
        .unwrap();
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert!(pointers.contains_key("TOKN"));
        assert!(pointers.contains_key("VEND"));
        assert_eq!(
            pointers.len(),
            2,
            "only the src/ and vendor/ copies survive"
        );

        // User excludes: plain prefixes keep working...
        let mut filter = ScanFilter::default();
        filter.exclude.push(PathBuf::from("src"));
        let (pointers, _) = collect_pointers(
            &root,
            &root.join("docs/doc-pointer-db.json"),
            &filter,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(pointers.len(), 1);
        assert!(pointers.contains_key("VEND"));

        // ...and globs match by path segment.
        let mut filter = ScanFilter::default();
        filter.exclude.push(PathBuf::from("**/generated/**"));
        let (pointers, _) = collect_pointers(
            &root,
            &root.join("docs/doc-pointer-db.json"),
            &filter,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(pointers.len(), 1, "glob exclude removed vendor/");
        assert!(pointers.contains_key("TOKN"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn glob_matcher_matches_segments() {
        assert!(glob_match("**/vendored/**", "libs/x/vendored/y/z.ex"));
        assert!(glob_match("src/*.gen.ex", "src/auth.gen.ex"));
        assert!(!glob_match("src/*.gen.ex", "src/sub/auth.gen.ex"));
        assert!(glob_match("src/**/*.ex", "src/sub/deep/auth.ex"));
        assert!(!glob_match("**/vendored/**", "src/auth.ex"));
        assert!(pattern_matches("plain/prefix", "plain/prefix/file.ex"));
        assert!(!pattern_matches("plain/prefix", "plain/prefixother.ex"));
    }

    #[test]
    fn annotate_reuses_store_records_unless_force_remint() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "/// Adds numbers.\npub fn add(a: u8, b: u8) -> u8 {\n    a + b\n}\n",
        )
        .unwrap();

        let args: Vec<String> = vec![
            "--root".into(),
            root.to_string_lossy().into_owned(),
            "--write".into(),
        ];
        annotate_command(&args).unwrap();
        let first = fs::read_to_string(root.join("src/lib.rs")).unwrap();
        let first_token = marker_token(&first).expect("marker minted");

        // Strip the marker (store keeps the record) and re-annotate: the same
        // {file}::{name} record is reused, not collision-bumped.
        let stripped: String = first
            .lines()
            .filter(|line| !line.contains("〚🔧:"))
            .map(|line| format!("{line}\n"))
            .collect();
        fs::write(root.join("src/lib.rs"), stripped).unwrap();
        annotate_command(&args).unwrap();
        let reused = fs::read_to_string(root.join("src/lib.rs")).unwrap();
        assert_eq!(marker_token(&reused).as_deref(), Some(first_token.as_str()));

        // --force-remint mints a fresh token instead of reusing.
        let stripped: String = reused
            .lines()
            .filter(|line| !line.contains("〚🔧:"))
            .map(|line| format!("{line}\n"))
            .collect();
        fs::write(root.join("src/lib.rs"), stripped).unwrap();
        let mut remint_args = args.clone();
        remint_args.push("--force-remint".into());
        annotate_command(&remint_args).unwrap();
        let reminted = fs::read_to_string(root.join("src/lib.rs")).unwrap();
        let reminted_token = marker_token(&reminted).expect("marker minted");
        assert_ne!(reminted_token, first_token);

        let _ = fs::remove_dir_all(&root);
    }

    /// First 〚🔧:…〛 token code in the text, as minted by annotate.
    fn marker_token(text: &str) -> Option<String> {
        let start = text.find("〚🔧:")? + "〚🔧:".len();
        let rest = &text[start..];
        let end = rest.find('〛')?;
        Some(rest[..end].to_string())
    }

    #[test]
    fn expand_markdown_links_rewrites_deeplink_targets() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(
            root.join("docs/target.md"),
            "<!-- ⟦ABCD⟧ Target pointer :: Used by markdown expansion. -->\n",
        )
        .unwrap();
        fs::write(
            root.join("docs/ref.md"),
            "[target](deeplink:⟦ABCD⟧)\n\n```markdown\n[ignored](deeplink:ABCD)\n```\n",
        )
        .unwrap();

        let (pointers, errors) = collect_pointers(
            &root,
            &root.join("docs/doc-pointer-db.json"),
            &ScanFilter::default(),
            &HashMap::new(),
        )
        .unwrap();
        assert!(errors.is_empty());
        let (changed, link_errors) =
            expand_markdown_links(&root, &pointers, true, &ScanFilter::default()).unwrap();
        assert!(link_errors.is_empty());
        assert_eq!(changed, vec!["docs/ref.md".to_string()]);

        let rewritten = fs::read_to_string(root.join("docs/ref.md")).unwrap();
        let uuid = Uuid::new_v5(
            &DOC_POINTER_NAMESPACE,
            uuid5_name("docs/target.md::Target pointer", "", 0).as_bytes(),
        );
        assert!(rewritten.contains(&format!("[target](docs/target.md:1?pointer={uuid})")));
        assert!(rewritten.contains("[ignored](deeplink:ABCD)"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn detects_rust_public_functions_only() {
        assert_eq!(
            detect_public_decl(Lang::Rust, "pub fn alpha() {"),
            Some(("alpha".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(Lang::Rust, "pub async fn beta(x: u8) -> u8 {"),
            Some(("beta".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(Lang::Rust, "pub unsafe extern \"C\" fn gamma() {"),
            Some(("gamma".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(Lang::Rust, "pub(crate) fn hidden() {"),
            None
        );
        assert_eq!(detect_public_decl(Lang::Rust, "fn private() {"), None);
        assert_eq!(detect_public_decl(Lang::Rust, "pub struct Thing {"), None);
        assert_eq!(
            detect_public_decl(Lang::Rust, "let s = \"pub fn fake\";"),
            None
        );
    }

    #[test]
    fn detects_elixir_public_defs_only() {
        assert_eq!(
            detect_public_decl(Lang::Elixir, "def fetch(id) do"),
            Some(("fetch".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(Lang::Elixir, "def valid?(x), do: true"),
            Some(("valid?".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(Lang::Elixir, "defmacro is_ok(x) do"),
            Some(("is_ok".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(Lang::Elixir, "defguard is_flag(x) when is_integer(x)"),
            Some(("is_flag".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(Lang::Elixir, "defmodule MyApp.Auth do"),
            Some(("MyApp.Auth".to_string(), MarkerKind::Module))
        );
        assert_eq!(
            detect_public_decl(Lang::Elixir, "defprotocol Serializable do"),
            Some(("Serializable".to_string(), MarkerKind::Contract))
        );
        assert_eq!(
            detect_public_decl(Lang::Elixir, "defimpl JSON.Encoder, for: User do"),
            Some(("JSON.Encoder for User".to_string(), MarkerKind::Contract))
        );
        assert_eq!(
            detect_public_decl(Lang::Elixir, "@callback handle_event(arg) :: :ok"),
            Some(("handle_event".to_string(), MarkerKind::Contract))
        );
        assert_eq!(detect_public_decl(Lang::Elixir, "defp helper(x) do"), None);
        assert_eq!(detect_public_decl(Lang::Elixir, "defmacrop m(x) do"), None);
        assert_eq!(
            detect_public_decl(Lang::Elixir, "def unquote(name)(x) do"),
            None
        );
    }

    #[test]
    fn detects_js_exports_only() {
        assert_eq!(
            detect_public_decl(Lang::Js, "export function run() {"),
            Some(("run".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(Lang::Js, "export default async function boot() {"),
            Some(("boot".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(Lang::Js, "export const handler = async (req) => {"),
            Some(("handler".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(
                Lang::Js,
                "export const fn2: (x: number) => void = (x) => {}"
            ),
            Some(("fn2".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(Lang::Js, "exports.helper = function () {"),
            Some(("helper".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(Lang::Js, "module.exports.util = () => {}"),
            Some(("util".to_string(), MarkerKind::Function))
        );
        assert_eq!(
            detect_public_decl(Lang::Js, "export const LIMIT = 5;"),
            None
        );
        assert_eq!(
            detect_public_decl(Lang::Js, "export default function () {"),
            None
        );
        assert_eq!(detect_public_decl(Lang::Js, "module.exports = {"), None);
        assert_eq!(
            detect_public_decl(Lang::Js, "const local = () => {};"),
            None
        );
    }

    #[test]
    fn marker_in_block_above_suppresses_annotation() {
        let lines: Vec<&str> = vec![
            "/// ⟦ABCD⟧ alpha :: existing marker",
            "/// docs continue",
            "pub fn alpha() {}",
        ];
        assert!(block_above_has_marker(Lang::Rust, &lines, 2));

        let lines: Vec<&str> = vec![
            "# ⟦ABCD⟧ fetch :: existing marker",
            "@doc \"\"\"",
            "Fetch a thing.",
            "\"\"\"",
            "def fetch(id) do",
        ];
        assert!(block_above_has_marker(Lang::Elixir, &lines, 4));

        let lines: Vec<&str> = vec!["pub fn bare() {}"];
        assert!(!block_above_has_marker(Lang::Rust, &lines, 0));

        let lines: Vec<&str> = vec!["let x = 1;", "", "// ⟦ABCD⟧ far away", "pub fn far() {}"];
        assert!(block_above_has_marker(Lang::Rust, &lines, 3));
        let lines: Vec<&str> = vec!["// ⟦ABCD⟧ other", "let x = 1;", "pub fn near() {}"];
        assert!(!block_above_has_marker(Lang::Rust, &lines, 2));
    }

    #[test]
    fn derives_descriptions_from_doc_blocks() {
        let lines: Vec<&str> = vec![
            "/// Fetches the widget. Retries twice.",
            "pub fn fetch() {}",
        ];
        assert_eq!(
            derive_description(Lang::Rust, &lines, 1),
            Some("Fetches the widget.".to_string())
        );
        let lines: Vec<&str> = vec![
            "@doc \"\"\"",
            "Loads config :: from disk.",
            "\"\"\"",
            "def load do",
        ];
        assert_eq!(
            derive_description(Lang::Elixir, &lines, 3),
            Some("Loads config : from disk.".to_string())
        );
        let lines: Vec<&str> = vec!["pub fn undocumented() {}"];
        assert_eq!(derive_description(Lang::Rust, &lines, 0), None);
    }

    #[test]
    fn derive_how_prefers_primary_clause_over_error_passthrough() {
        // Multi-clause function leading with an error pass-through clause: the
        // How text must describe the substantive clause, not `{:error, _} = e`.
        let lines: Vec<&str> = vec![
            "@spec id(any) :: {:ok, any} | {:error, any}",
            "def id({:error, _} = e), do: e",
            "",
            "def id(R.ref(module: h) = subject) do",
            "  h.id(subject)",
            "end",
        ];
        let how = derive_how(Lang::Elixir, &lines, 1).unwrap();
        assert!(how.contains("R.ref(module: h)"), "how was: {how}");
        assert!(!how.contains("takes `{:error,"), "how was: {how}");
        // Spec return is still picked up from above the first clause.
        assert!(
            how.contains("returns `{:ok, any} | {:error, any}`"),
            "how was: {how}"
        );

        // Two-arg pass-through (entity/2 shape) also picks the primary clause.
        let lines: Vec<&str> = vec![
            "def entity({:error, _} = e, _), do: e",
            "",
            "def entity(R.ref(module: h) = subject, context) do",
            "  h.entity(subject, context)",
            "end",
        ];
        let how = derive_how(Lang::Elixir, &lines, 0).unwrap();
        assert!(how.contains("R.ref(module: h)"), "how was: {how}");
        assert!(!how.contains("takes `{:error,"), "how was: {how}");

        // All-clauses-error shape: keeps describing the first head.
        let lines: Vec<&str> = vec!["def id({:error, _} = e), do: e"];
        let how = derive_how(Lang::Elixir, &lines, 0).unwrap();
        assert!(how.contains("{:error,"), "how was: {how}");

        // Different-function boundary stops the scan.
        let lines: Vec<&str> = vec!["def id({:error, _} = e), do: e", "", "def other(x), do: x"];
        let how = derive_how(Lang::Elixir, &lines, 0).unwrap();
        assert!(how.contains("{:error,"), "how was: {how}");
    }

    #[test]
    fn annotate_inserts_markers_and_is_idempotent() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "/// Adds numbers.\npub fn add(a: u8, b: u8) -> u8 {\n    a + b\n}\n\nimpl Widget {\n    pub fn new() -> Self {\n        Widget\n    }\n}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/app.ex"),
            "defmodule App do\n  def run(x) do\n    x\n  end\n\n  def run(x, y) do\n    {x, y}\n  end\n\n  defp hidden, do: :ok\nend\n",
        )
        .unwrap();
        fs::write(
            root.join("src/index.js"),
            "export function main() {}\nconst secret = () => {};\n",
        )
        .unwrap();

        let args: Vec<String> = vec![
            "--root".into(),
            root.to_string_lossy().into_owned(),
            "--write".into(),
        ];
        annotate_command(&args).unwrap();

        let rust = fs::read_to_string(root.join("src/lib.rs")).unwrap();
        assert!(rust.contains("// 〚🔧:"));
        assert!(rust.contains(" add :: Adds numbers."));
        // Fallback description is the signature itself, not boilerplate.
        assert!(rust.contains(" new :: new() -> Self"));
        // Inserted method marker keeps the declaration's indentation.
        assert!(rust.contains("\n    // 〚🔧:"));

        let elixir = fs::read_to_string(root.join("src/app.ex")).unwrap();
        assert_eq!(
            elixir.matches("〚🔧:").count(),
            1,
            "one marker per function name across clauses/arities"
        );
        // Module marker lands in a fresh @moduledoc; function marker in @doc.
        assert!(elixir.contains("@moduledoc \"\"\""));
        assert!(elixir.matches("〚📦:").count() == 1);
        assert!(elixir.contains("@doc \"\"\""));
        assert!(elixir.contains("How: takes `x`"));
        assert!(elixir.contains("run :: run/1"));
        assert!(!elixir.contains("hidden ::"));

        let js = fs::read_to_string(root.join("src/index.js")).unwrap();
        assert!(js.contains("// 〚🔧:"));
        assert!(!js.contains("secret ::"));

        // The closing build stores records through the Elixir backend.
        let records = backend_status(&root).unwrap();
        assert!(records.iter().any(|record| record.path == "src/lib.rs"));
        assert!(!root.join("docs/doc-pointer-db.json").exists());

        // Idempotency: second run changes nothing.
        annotate_command(&args).unwrap();
        assert_eq!(rust, fs::read_to_string(root.join("src/lib.rs")).unwrap());
        assert_eq!(elixir, fs::read_to_string(root.join("src/app.ex")).unwrap());
        assert_eq!(js, fs::read_to_string(root.join("src/index.js")).unwrap());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn annotate_merges_elixir_markers_into_doc_attributes() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("lib")).unwrap();
        let source = concat!(
            "defmodule Cart do\n",
            "  @moduledoc \"\"\"\n",
            "  Shopping cart.\n",
            "\n",
            "  Long description.\n",
            "  \"\"\"\n",
            "\n",
            "  @doc \"Adds an item.\"\n",
            "  @spec add(Cart.t(), item()) :: Cart.t()\n",
            "  def add(cart, item) do\n",
            "    cart\n",
            "  end\n",
            "\n",
            "  @doc \"\"\"\n",
            "  Removes an item.\n",
            "  \"\"\"\n",
            "  def remove(cart) do\n",
            "    cart\n",
            "  end\n",
            "\n",
            "  @doc false\n",
            "  def debug(cart), do: cart\n",
            "\n",
            "  defprotocol Encoder do\n",
            "    @callback encode(term) :: binary\n",
            "  end\n",
            "end\n",
        );
        fs::write(root.join("lib/cart.ex"), source).unwrap();
        let args: Vec<String> = vec![
            "--root".into(),
            root.to_string_lossy().into_owned(),
            "--write".into(),
        ];
        annotate_command(&args).unwrap();
        let text = fs::read_to_string(root.join("lib/cart.ex")).unwrap();

        // moduledoc heredoc: marker appended inside the existing docstring,
        // never a second attribute. Separators are plain empty lines so
        // `mix format` cannot normalize them away.
        assert_eq!(text.matches("@moduledoc").count(), 1);
        assert!(text.contains("Long description.\n\n  〚📦:"));
        assert!(text.contains("Cart :: Shopping cart."));

        // single-line @doc widened to a heredoc preserving the text.
        assert!(text.contains("@doc \"\"\"\n  Adds an item.\n\n  〚🔧:"));
        assert!(text.contains("add :: Adds an item."));
        assert_eq!(text.matches("@doc \"\"\"").count(), 2); // widened add + remove heredoc

        // heredoc @doc: marker appended before the closing quotes.
        assert!(text.contains("Removes an item.\n\n  〚🔧:"));
        assert!(text.contains("remove :: Removes an item."));
        assert!(!text.contains("\n  \n"));

        // @doc false stays untouched; marker hangs on a comment above.
        assert!(text.contains("@doc false"));
        assert!(text.contains("# 〚🔧:"));
        assert!(text.contains("debug :: debug/1"));

        // contracts get comment markers (an @doc above defprotocol warns).
        assert!(text.contains("# 〚🔌:"));
        assert!(text.contains("Encoder :: Encoder protocol"));
        assert!(text.contains("encode/1"));

        // Idempotency: second run changes nothing.
        annotate_command(&args).unwrap();
        assert_eq!(text, fs::read_to_string(root.join("lib/cart.ex")).unwrap());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn annotate_is_deterministic_across_tree_copies() {
        let make_tree = || {
            let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("src")).unwrap();
            fs::write(root.join("src/lib.rs"), "pub fn solo() {}\n").unwrap();
            root
        };
        let a = make_tree();
        let b = make_tree();
        for root in [&a, &b] {
            let args: Vec<String> = vec![
                "--root".into(),
                root.to_string_lossy().into_owned(),
                "--write".into(),
            ];
            annotate_command(&args).unwrap();
        }
        assert_eq!(
            fs::read_to_string(a.join("src/lib.rs")).unwrap(),
            fs::read_to_string(b.join("src/lib.rs")).unwrap(),
            "same tree must mint identical codes"
        );
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    #[test]
    fn annotate_preserves_missing_trailing_newline_layout() {
        let root = env::temp_dir().join(format!("doc-pointers-test-{}", Uuid::new_v4()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/tail.rs"), "pub fn tail() {}").unwrap(); // no trailing \n
        let args: Vec<String> = vec![
            "--root".into(),
            root.to_string_lossy().into_owned(),
            "--write".into(),
        ];
        annotate_command(&args).unwrap();
        let text = fs::read_to_string(root.join("src/tail.rs")).unwrap();
        assert!(
            text.ends_with("pub fn tail() {}"),
            "no trailing newline added"
        );
        assert!(text.starts_with("// 〚🔧:"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_filter_scopes_dirs_and_files() {
        let filter = ScanFilter {
            include: vec![PathBuf::from("utilities"), PathBuf::from("libs")],
            exclude: vec![PathBuf::from("utilities/vendored")],
        };
        assert!(filter.allows_dir(Path::new("utilities")));
        assert!(filter.allows_dir(Path::new("utilities/shell")));
        assert!(!filter.allows_dir(Path::new("utilities/vendored")));
        assert!(!filter.allows_dir(Path::new("projects")));
        assert!(filter.allows_file(Path::new("libs/core/lib/helpers.ex")));
        assert!(!filter.allows_file(Path::new("utilities/vendored/x.js")));
        assert!(!filter.allows_file(Path::new("README.md")));
        let open = ScanFilter::default();
        assert!(open.allows_dir(Path::new("anything")));
        assert!(open.allows_file(Path::new("any/file.rs")));
    }

    #[test]
    fn generated_and_minified_files_are_skipped() {
        assert!(skip_file_name("bundle.min.js"));
        assert!(skip_file_name("types.d.ts"));
        assert!(skip_file_name("next-env.d.ts"));
        assert!(skip_file_name("package-lock.json"));
        assert!(!skip_file_name("main.js"));
        assert!(!skip_file_name("lib.rs"));
    }
}
