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
    code: String,
    path: String,
    line: usize,
    name: String,
    description: String,
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
#[derive(Debug, Default, Clone)]
struct ScanFilter {
    include: Vec<PathBuf>,
    exclude: Vec<PathBuf>,
}

impl ScanFilter {
    fn allows_dir(&self, rel: &Path) -> bool {
        if self.exclude.iter().any(|e| rel.starts_with(e)) {
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
        if self.exclude.iter().any(|e| rel.starts_with(e)) {
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

    let (pointers, mut errors) = collect_pointers(&root, &db_path, &options.filter)?;
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let records = reconcile_records(&root, &db_path, &pointers)?;
    let db_changed = if options.write || options.check {
        backend_reconcile(&root, &records, options.write)?
    } else {
        false
    };
    let (changed_links, link_errors) =
        expand_markdown_links(&root, &pointers, options.write, &options.filter)?;
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
    let (mut pointers, errors) = collect_pointers(&root, &db_path, &ScanFilter::default())?;
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
    println!("marker: ⟦{code}⟧");
    println!("clipboard: {payload}");

    if !options.no_clipboard {
        copy_to_clipboard(&payload)?;
        println!("copied to clipboard");
    }

    Ok(())
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

fn detect_public_decl(lang: Lang, trimmed: &str) -> Option<String> {
    match lang {
        Lang::Rust => detect_rust_pub_fn(trimmed),
        Lang::Elixir => detect_elixir_public_def(trimmed),
        Lang::Js => detect_js_export(trimmed),
    }
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

fn detect_elixir_public_def(trimmed: &str) -> Option<String> {
    // Exact `def ` / `defmacro ` keyword + space, so defp/defmodule/defmacrop/
    // defdelegate/defimpl never match.
    let rest = trimmed
        .strip_prefix("def ")
        .or_else(|| trimmed.strip_prefix("defmacro "))?
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
    Some(name)
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
/// declaration already contains a `⟦…⟧` marker — makes annotate idempotent and
/// tolerates humans relocating the marker within the doc block.
fn block_above_has_marker(lang: Lang, lines: &[&str], decl_idx: usize) -> bool {
    let mut index = decl_idx;
    let mut in_heredoc = false;
    while index > 0 {
        index -= 1;
        let line = lines[index];
        let trimmed = line.trim();
        if in_heredoc {
            if line.contains('⟦') {
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
        if line.contains('⟦') {
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
            if text.is_empty() || text.starts_with('⟦') || text.starts_with('@') {
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

fn annotate_command(args: &[String]) -> Result<(), String> {
    let options = parse_annotate_options(args)?;
    let root = absolute_path(&options.root)?;
    let db_path = legacy_db_path(&root, &options.db)?;

    // Live collision map: DB entries plus every marker in the scanned tree (sources are
    // scanned too now, so stray markers not yet indexed are collision-checked as well).
    let (mut pointers, errors) = collect_pointers(&root, &db_path, &options.filter)?;
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
        let mut targets: Vec<(usize, String)> = Vec::new();
        let mut seen_names: HashSet<String> = HashSet::new();
        for (idx, line) in lines.iter().enumerate() {
            if line.len() > 500 {
                continue; // minification tripwire
            }
            let trimmed = line.trim_start();
            let Some(name) = detect_public_decl(lang, trimmed) else {
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
            targets.push((idx, name));
        }
        if targets.is_empty() {
            continue;
        }
        if lang == Lang::Rust && text.contains("macro_rules!") {
            review_flags.push(format!(
                "{relpath}: contains macro_rules! — review inserted markers manually"
            ));
        }

        let mut insertions: Vec<(usize, String)> = Vec::new();
        for (idx, name) in &targets {
            let seed = format!("{relpath}::{name}");
            let (code, uuid, _uuid_name, _attempt) =
                generate_uuid5_code(&seed, DOC_POINTER_NAMESPACE, "", &pointers)?;
            minted.insert(code.clone(), uuid);
            let description = derive_description(lang, &lines, *idx)
                .unwrap_or_else(|| format!("auto-generated pointer for public function {name}"));
            let indent: String = lines[*idx]
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect();
            let leader = comment_leader(lang);
            insertions.push((
                *idx,
                format!("{indent}{leader} ⟦{code}⟧ {name} :: {description}"),
            ));
            pointers.insert(
                code.clone(),
                Pointer {
                    uuid: Some(uuid),
                    code,
                    path: relpath.clone(),
                    line: idx + 1, // provisional; the closing build records exact lines
                    name: name.clone(),
                    description,
                },
            );
        }

        planned += insertions.len();
        file_reports.push(format!("{relpath}: {} pointer(s)", insertions.len()));

        if options.write {
            let mut new_lines: Vec<String> =
                text.split_inclusive('\n').map(str::to_string).collect();
            for (idx, marker) in insertions.iter().rev() {
                new_lines.insert(*idx, format!("{marker}\n"));
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
        let (mut fresh, build_errors) = collect_pointers(&root, &db_path, &options.filter)?;
        for (code, uuid) in minted {
            if let Some(pointer) = fresh.get_mut(&code) {
                pointer.uuid = Some(uuid);
            }
        }
        for error in &build_errors {
            eprintln!("WARNING: {error}");
        }
        let records = reconcile_records(&root, &db_path, &fresh)?;
        let db_changed = backend_reconcile(&root, &records, true)?;
        let (changed_links, link_errors) =
            expand_markdown_links(&root, &fresh, true, &options.filter)?;
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
Walk the tree and insert `⟦code⟧ Name :: Description` markers above every public/exported\n\
Rust (`pub fn`), Elixir (`def`/`defmacro`), and JS/TS (`export`/`exports.`) function that\n\
does not already have one in its doc/comment block. Dry-run by default; --write applies\n\
the insertions and then reconciles .meta/pointers.yaml + expands deeplinks in the same run.\n\n\
options:\n  --root ROOT       repository root, default: current directory\n  --db DB           deprecated; only docs/doc-pointer-db.json is accepted for migration\n  --include P       only scan/annotate under this root-relative prefix (repeatable)\n  --exclude P       skip this root-relative prefix (repeatable)\n  --lang exs        also annotate .exs scripts (skipped by default)\n  --write           apply insertions (otherwise dry-run report only)"
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
doc-pointers — durable, code-stable cross-document pointers

WHAT
  Doc pointers are 4-character tokens drawn from curated printable Unicode sign
  blocks: Meroitic Hieroglyphs, Egyptian Hieroglyphs, Egyptian Hieroglyphs
  Extended-A, and Anatolian Hieroglyphs. A declaration looks like:

      > ⟦𓳔𔐮𔘟𔄵⟧ Pointer name :: Human readable description.

  Because the token is a fixed Unicode glyph sequence and not a path or line
  number, it survives renames, refactors, and file moves. Other documents then
  reference it with a `deeplink:` Markdown link, e.g.

      [see routing](deeplink:⟦𓳔𔐮𔘟𔄵⟧)

  and `doc-pointers` rewrites that link into a concrete `path:line?code=⟦…⟧` target.

HOW
  1. Place a ⟦token⟧ Name :: Description declaration at the anchor you want to
     point at (a function, a heading, a config stanza). The token must live in a
     comment context (//, #, <!--, /*, *, --, ;) or on its own line, so it never
     collides with string literals or code.
  2. Generate new tokens with `doc-pointers uuid5` (deterministic UUIDv5,
     collision-checked against existing stores, copied to your clipboard).
  3. Reference any token from Markdown with `[label](deeplink:⟦token⟧)`.
  4. Run `doc-pointers build --write` to (a) reconcile declarations into
     `.meta/pointers.yaml` in each owning repo and (b) expand `deeplink:` into a
     real `file:line` target. Run `doc-pointers build --check` in CI / a
     pre-commit hook to fail when stores or any link is stale.

WHY
  Ordinary file:line and URL links rot the instant code moves. Branch names and
  permalinks are worse. A doc pointer decouples the *identity* of an anchor
  (the token) from its *current location* (which `build` resolves on demand), so
  docs stay accurate without manual re-pointing. The sign blocks are chosen so
  tokens are visually distinct and unlikely to appear in real source. UUIDv5
  derivation makes them reproducible across machines, and generation checks the
  current stores for display-token collisions.

USAGE
  doc-pointers                       print this help
  doc-pointers build                 collect declarations + expand deeplinks (read-only)
  doc-pointers build --write         reconcile .meta/pointers.yaml + expanded links
  doc-pointers build --check         fail (exit 1) if a write would change anything
  doc-pointers annotate              dry-run: report public fns lacking markers
  doc-pointers annotate --write      insert markers + reconcile stores in one pass
  doc-pointers hook                  install a pre-commit hook that runs --check
  doc-pointers uuid5 [NAME]          mint a new 4-glyph token, copied to clipboard
  doc-pointers help                  print this help

SUB-COMMAND HELP
  doc-pointers build --help          options for the build/scan command
  doc-pointers annotate --help       options for the annotate command
  doc-pointers uuid5 --help          options for the uuid5 command

LEGACY
  The bare flags still work for existing scripts:
    doc-pointers --write        ==  doc-pointers build --write
    doc-pointers --check        ==  doc-pointers build --check
    doc-pointers --install-hook ==  doc-pointers hook
  Prefer the subcommand form in new code."
    );
}

fn print_scan_help() {
    println!(
        "usage: doc-pointers build [--root ROOT] [--db DB] [--write] [--check]\n\n\
Reconcile doc pointer stores and expand deeplink: markdown links.\n\
Run without --write/--check, this is a dry run: it reports how many\n\
declarations it found and any errors, but changes nothing.\n\n\
options:\n  --root ROOT       repository root, default: current directory\n  --db DB           deprecated; only docs/doc-pointer-db.json is accepted for migration\n  --write           reconcile stores and expand markdown deeplinks\n  --check           fail if writes would be needed (CI / pre-commit)"
    );
}

fn print_uuid5_help() {
    println!(
        "usage: doc-pointers uuid5 [options] [NAME]\n\n\
Generate a deterministic UUIDv5-backed four-character doc pointer token.\n\
The token is collision-checked against current stores and copied to\n\
the clipboard unless --no-clipboard is given.\n\n\
options:\n  --root ROOT             repository root, default: current directory\n  --db DB                 deprecated; only docs/doc-pointer-db.json is accepted\n  --namespace NAMESPACE   doc-pointers, dns, url, oid, x500, or a UUID\n  --salt SALT             optional deterministic salt\n  --format FORMAT         marker, code, declaration, or deeplink\n  --description TEXT      description used by --format declaration\n  --no-clipboard          print without copying to clipboard"
    );
}

fn collect_pointers(
    root: &Path,
    db_path: &Path,
    filter: &ScanFilter,
) -> Result<(HashMap<String, Pointer>, Vec<String>), String> {
    let mut pointers: HashMap<String, Pointer> = HashMap::new();
    let mut errors = Vec::new();
    for path in scan_files(root, db_path, filter)? {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let mut in_fence = false;
        for (index, line) in text.lines().enumerate() {
            if path.extension() == Some(OsStr::new("md")) && line.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            let Some((code, name, description)) = parse_declaration(line) else {
                continue;
            };
            let pointer = Pointer {
                uuid: None,
                code: code.clone(),
                path: rel_path(&path, root),
                line: index + 1,
                name,
                description,
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
    }
    Ok((pointers, errors))
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
        "build",
        "coverage",
        "deps",
        "dist",
        "node_modules",
        "obj",
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
        let code = normalize_code(raw_code);
        if code.chars().count() == 4 && valid_code(&code) {
            output.push_str(before);
            if let Some(pointer) = pointers.get(&code) {
                output.push_str(&format!("]({})", expanded_target(pointer)));
            } else {
                errors.push(format!(
                    "{}: unresolved deeplink:{code}",
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
    format!("{}:{}?code=⟦{}⟧", pointer.path, pointer.line, pointer.code)
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
    let mut pointers = HashMap::new();
    for (code, data) in entries {
        let path = data
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("legacy pointer {code} in {} has no path", db_path.display()))?;
        let name = data.get("name").and_then(Value::as_str).unwrap_or(code);
        pointers.insert(
            code.clone(),
            Pointer {
                uuid: Some(Uuid::new_v5(
                    &DOC_POINTER_NAMESPACE,
                    uuid5_name(name, "", 0).as_bytes(),
                )),
                code: code.clone(),
                path: path.to_string(),
                line: data.get("line").and_then(Value::as_u64).unwrap_or(0) as usize,
                name: name.to_string(),
                description: data
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            },
        );
    }
    Ok(pointers)
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
    record
}

fn reconcile_records(
    root: &Path,
    db_path: &Path,
    scanned: &HashMap<String, Pointer>,
) -> Result<Vec<Value>, String> {
    let known: HashSet<String> = backend_status(root)?.into_iter().map(|p| p.code).collect();
    let mut records = legacy_records(db_path)?;
    records.retain(|code, _| !known.contains(code));
    for (code, pointer) in scanned {
        let mut pointer = pointer.clone();
        if pointer.uuid.is_none() && !known.contains(code) {
            pointer.uuid = records.get(code).and_then(|legacy| legacy.uuid);
        }
        records.insert(code.clone(), pointer);
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
                code: text("token")?,
                path: text("file_path")?,
                line: record.get("line").and_then(Value::as_u64).unwrap_or(0) as usize,
                name: text("function")?,
                description: text("description")?,
            })
        })
        .collect()
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
    code: &str,
    name: Option<&str>,
    description: &str,
) -> Result<String, String> {
    let marker = format!("⟦{code}⟧");
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
            Some(Uuid::new_v5(&DOC_POINTER_NAMESPACE, b"doc-pointers:legacy"))
        );
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
        )
        .unwrap();
        assert!(errors.is_empty());
        assert!(pointers.contains_key("REAL"));
        assert!(!pointers.contains_key("NOIS"));

        let _ = fs::remove_dir_all(&root);
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
        )
        .unwrap();
        assert!(errors.is_empty());
        let (changed, link_errors) =
            expand_markdown_links(&root, &pointers, true, &ScanFilter::default()).unwrap();
        assert!(link_errors.is_empty());
        assert_eq!(changed, vec!["docs/ref.md".to_string()]);

        let rewritten = fs::read_to_string(root.join("docs/ref.md")).unwrap();
        assert!(rewritten.contains("[target](docs/target.md:1?code=⟦ABCD⟧)"));
        assert!(rewritten.contains("[ignored](deeplink:ABCD)"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn detects_rust_public_functions_only() {
        assert_eq!(
            detect_public_decl(Lang::Rust, "pub fn alpha() {"),
            Some("alpha".to_string())
        );
        assert_eq!(
            detect_public_decl(Lang::Rust, "pub async fn beta(x: u8) -> u8 {"),
            Some("beta".to_string())
        );
        assert_eq!(
            detect_public_decl(Lang::Rust, "pub unsafe extern \"C\" fn gamma() {"),
            Some("gamma".to_string())
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
            Some("fetch".to_string())
        );
        assert_eq!(
            detect_public_decl(Lang::Elixir, "def valid?(x), do: true"),
            Some("valid?".to_string())
        );
        assert_eq!(
            detect_public_decl(Lang::Elixir, "defmacro is_ok(x) do"),
            Some("is_ok".to_string())
        );
        assert_eq!(detect_public_decl(Lang::Elixir, "defp helper(x) do"), None);
        assert_eq!(detect_public_decl(Lang::Elixir, "defmodule Foo do"), None);
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
            Some("run".to_string())
        );
        assert_eq!(
            detect_public_decl(Lang::Js, "export default async function boot() {"),
            Some("boot".to_string())
        );
        assert_eq!(
            detect_public_decl(Lang::Js, "export const handler = async (req) => {"),
            Some("handler".to_string())
        );
        assert_eq!(
            detect_public_decl(
                Lang::Js,
                "export const fn2: (x: number) => void = (x) => {}"
            ),
            Some("fn2".to_string())
        );
        assert_eq!(
            detect_public_decl(Lang::Js, "exports.helper = function () {"),
            Some("helper".to_string())
        );
        assert_eq!(
            detect_public_decl(Lang::Js, "module.exports.util = () => {}"),
            Some("util".to_string())
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
        assert!(rust.contains("// ⟦"));
        assert!(rust.contains(" add :: Adds numbers."));
        assert!(rust.contains(" new :: auto-generated pointer for public function new"));
        // Inserted method marker keeps the declaration's indentation.
        assert!(rust.contains("\n    // ⟦"));

        let elixir = fs::read_to_string(root.join("src/app.ex")).unwrap();
        assert_eq!(
            elixir.matches("# ⟦").count(),
            1,
            "one marker per function name across clauses/arities"
        );
        assert!(!elixir.contains("hidden ::"));

        let js = fs::read_to_string(root.join("src/index.js")).unwrap();
        assert!(js.contains("// ⟦"));
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
        assert!(text.starts_with("// ⟦"));
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
