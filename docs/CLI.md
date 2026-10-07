# doc-pointers CLI

The Rust CLI scans source markers and Markdown links. The Elixir store owns pointer records in `.meta/pointers.yaml` at the root and in each nested Git submodule. The CLI sends JSON requests to `mix doc_pointers.cli`; it never writes pointer YAML or legacy JSON itself.

## Build and install

From this checkout:

```sh
mix deps.get && mix compile
make cli-test
make cli-install                 # installs to ~/.local/bin by default
# or: INSTALL_DIR=/usr/local/bin make cli-install
```

`bin/doc-pointers` runs the source crate without installing it. The installed binary needs this checkout for its Elixir backend. Set `DOC_POINTERS_HOME=/absolute/path/to/doc-pointers` if you move or build the binary elsewhere. Make sure `mix` is on `PATH`.

## Mint and use a pointer

```sh
doc-pointers uuid5 "routing table init" --root /path/to/project --no-clipboard
```

The command prints a UUIDv5, four glyph token, and marker. Omit `--no-clipboard` on macOS to copy the marker with `pbcopy`. Put the marker beside its anchor in a source comment or Markdown declaration:

```text
# ⟦𓆴𓎲𓋝𓁅⟧ routing table init :: builds the initial route table
```

Reference it in Markdown as `[routing init](deeplink:⟦𓆴𓎲𓋝𓁅⟧)`. Then run:

```sh
doc-pointers build --root /path/to/project --write
```

The CLI reconciles declarations into the owning `.meta/pointers.yaml` files and expands `deeplink:` references to current `file:line?code=⟦…⟧` targets. `build` without `--write` reports the scan without changing files. `build --check` exits nonzero if the stores or links need an update, making it suitable for CI.

`--include PATH` and `--exclude PATH` scope scans to root-relative prefixes. Use them for large monorepos. Duplicate tokens are errors and are never reconciled.

## Annotate public functions

```sh
doc-pointers annotate --root /path/to/project --include lib
# review the report, then:
doc-pointers annotate --root /path/to/project --include lib --write
```

`annotate` inserts markers above unmarked public Rust, Elixir, JavaScript, and TypeScript functions. It is a dry run by default. `--write` inserts markers, reconciles store records, and expands links. Add `--lang exs` to include Elixir scripts. Repeated runs leave already marked functions alone. Generated tokens use the UUIDv5 seed `doc-pointers:<root-relative-path>::<function>`; the backend retains this UUID for newly annotated markers.

## Pre-commit hook

```sh
doc-pointers hook --root /path/to/project
```

The installed hook invokes the CLI executable with `build --check`. It refuses to replace a hook it does not own. Install it from a stable binary path, such as `~/.local/bin/doc-pointers`, so the hook keeps working after a rebuild.

## Legacy JSON migration

The CLI recognizes `docs/doc-pointer-db.json` under the selected root. `build --check` reports pending migration without writing; `build --write` sends legacy records missing from YAML to the Elixir backend. Source declarations for the same token take precedence. Existing YAML records retain their UUID and creation time. The JSON file stays intact as a backup; review the YAML and remove the JSON in a separate change when satisfied.

`--db docs/doc-pointer-db.json` is accepted for compatibility. A custom `--db` path is rejected with an explanation. To migrate a custom JSON file, first back it up and place a reviewed copy at `docs/doc-pointer-db.json` under the target root.

## Troubleshooting

- If the CLI cannot find the Elixir project, set `DOC_POINTERS_HOME` to this checkout.
- If `mix` is missing, install Elixir and put `mix` on `PATH`.
- If `uuid5` cannot run `pbcopy`, pass `--no-clipboard` and copy its `clipboard:` output.
- The scanner recognizes source and document extensions listed in `doc-pointers build --help`; markers inside unrecognized files are not indexed.
