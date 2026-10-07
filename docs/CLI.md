# doc-pointers CLI

The Rust CLI scans source markers and Markdown links. The Elixir store owns pointer records in `.meta/pointers.yaml` at the root and in each nested Git submodule. The CLI sends JSON requests to `mix doc_pointers.cli`; it does not write pointer YAML or legacy JSON itself.

## Build and install

```sh
mix deps.get && mix compile
make cli-test
make cli-install                 # installs to ~/.local/bin by default
# or: INSTALL_DIR=/usr/local/bin make cli-install
```

`bin/doc-pointers` runs the source crate without installing it. The installed binary needs this checkout for its Elixir backend. Set `DOC_POINTERS_HOME=/absolute/path/to/doc-pointers` if you move the checkout or build the binary elsewhere. Put `mix` on `PATH`.

## Markers and references

Canonical markers use a type emoji and a lowercase hyphenated UUIDv5:

```text
# 〚🔧:5c692577-ad0c-51f1-992c-759b5e5fffb5〛 run :: runs the task
```

| Emoji | Kind |
|---|---|
| 📁 | File |
| 📦 | Module, class, or struct |
| 🔌 | Interface, protocol, behavior, or other contract |
| 🧩 | Reusable component, which may occur in multiple files |
| 🔧 | Function |
| 🔀 | Logic section |
| 📐 | Diagram, including Mermaid and PlantUML |

A component, logic section, or diagram has a matching closing marker with the same emoji and UUID. Closers are validated per file and may nest. A reusable component can have several opening/closing spans; all its locations are stored under one UUID.

```text
# 〚🧩:5c692577-ad0c-51f1-992c-759b5e5fffb5〛 shared panel :: reusable section
...
# 〚/🧩:5c692577-ad0c-51f1-992c-759b5e5fffb5〛
```

Mint a marker with:

```sh
doc-pointers uuid5 "routing table init" --kind logic --root /path/to/project --no-clipboard
```

The command prints UUIDv5, the legacy four glyph token alias, and the canonical marker. Omit `--no-clipboard` on macOS to copy the marker with `pbcopy`. `--kind` defaults to `function`.

Reference a UUID from Markdown as `[routing](deeplink:〚🔧:UUID〛)`. Then run:

```sh
doc-pointers build --root /path/to/project --write
```

The CLI reconciles declarations into the owning `.meta/pointers.yaml` files and expands `deeplink:` references to current `file:line?pointer=UUID` targets. `build` without `--write` reports the scan without changing files. `build --check` exits nonzero if the stores or links need an update, making it suitable for CI. Old `⟦4-glyph⟧` declarations and deeplinks are still read for migration; new output uses UUID markers.

`--include PATH` and `--exclude PATH` scope scans to root-relative prefixes. Use them for large monorepos. Duplicate UUID declarations are errors except repeated 🧩 spans, which are collected as occurrences.

## Look up a pointer

```sh
doc-pointers lookup 5c692577-ad0c-51f1-992c-759b5e5fffb5 --root /path/to/project
doc-pointers lookup '🔧:5c692577-ad0c-51f1-992c-759b5e5fffb5' --root /path/to/project
doc-pointers lookup '〚🧩:UUID〛' --root /path/to/project --context 4
```

Lookup prints metadata and source snippets. For 🧩 it prints every occurrence. The typed forms also verify the requested kind. `--context` sets the number of surrounding lines (default 2, maximum 20).

## Annotate public functions

```sh
doc-pointers annotate --root /path/to/project --include lib
doc-pointers annotate --root /path/to/project --include lib --write
```

`annotate` inserts `〚🔧:UUID〛` markers above unmarked public Rust, Elixir, JavaScript, and TypeScript functions. It is a dry run by default. `--write` inserts markers, reconciles store records, and expands links. Add `--lang exs` to include Elixir scripts. Repeated runs leave marked functions alone. Generated UUIDs use the seed `doc-pointers:<root-relative-path>::<function>`.

## Pre-commit hook

```sh
doc-pointers hook --root /path/to/project
```

The installed hook invokes the CLI executable with `build --check`. It refuses to replace a hook it does not own. Install it from a stable binary path, such as `~/.local/bin/doc-pointers`.

## Legacy JSON migration

The CLI recognizes `docs/doc-pointer-db.json` under the selected root. `build --check` reports pending migration without writing; `build --write` sends legacy records missing from YAML to the Elixir backend. Source declarations for the same token take precedence. Existing YAML records retain their UUID and creation time. Legacy names that occur more than once get stable UUIDs based on their token, avoiding identity collisions. The JSON file stays intact as a backup; review the YAML and remove the JSON in a separate change when satisfied.

`--db docs/doc-pointer-db.json` is accepted for compatibility. A custom `--db` path is rejected with an explanation. To migrate a custom JSON file, first back it up and place a reviewed copy at `docs/doc-pointer-db.json` under the target root.

## Troubleshooting

- If the CLI cannot find the Elixir project, set `DOC_POINTERS_HOME` to this checkout.
- If `mix` is missing, install Elixir and put `mix` on `PATH`.
- If `uuid5` cannot run `pbcopy`, pass `--no-clipboard` and copy its `clipboard:` output.
- The scanner recognizes source and document extensions listed in `doc-pointers build --help`; markers inside unrecognized files are not indexed.
