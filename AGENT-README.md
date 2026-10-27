# AGENT-README

> Orientation for AI coding agents working in this repo. Human docs: README.md.

## Type

elixir-lib+mcp-server+rust-cli

## Purpose

Mint durable, code-stable citations for a source locus: UUIDv5 identity + hieroglyph/typed marker pasteable into docs, resolvable after refactors. Two surfaces share one store: an MCP server (stdio preferred, optional loopback HTTP) and an Elixir library; a Rust CLI scans markers/Markdown using the Elixir store as backend.

## Stack

Elixir ~> 1.18 (`noizu_mcp`, `bandit`, `plug`, `yaml_elixir`/`ymlr`, `jason`); Rust CLI under `rust/` (see `docs/CLI.md`).

## Dev mode

```bash
mix deps.get
mix compile
mix test
mix doc_pointers.mcp.stdio --root /path/to/project                          # stdio MCP (read-only default)
DOC_POINTERS_PORT=4242 mix doc_pointers.mcp.server --root /path/to/project  # optional HTTP MCP
```

## Ports / URLs

- `http://127.0.0.1:4242/` — optional loopback Streamable-HTTP MCP endpoint (`DOC_POINTERS_PORT`, default 4242, binds 127.0.0.1 only, no auth). Stdio mode has no port.

## Tests

```bash
mix test              # coverage gate enforced in CI: mix test --cover, threshold 61 (mix.exs)
```

## Gotchas

- Run `mix compile` before launching the stdio MCP — otherwise Mix prints to stdout and corrupts the JSON-RPC stream.
- Write tools (`generate`/`update`) are off by default; enable with `--write` or `DOC_POINTERS_MCP_WRITES=1`.
- `--root` defaults to `DOC_POINTERS_ROOT` or cwd; both MCP tasks print connection commands to stderr, keeping stdout clean.
- Legacy `⟦glyph⟧` markers stay readable; new markers use the typed `〚emoji:uuid〛` form.
- Worktrees (monorepo convention): the main checkout owns `deps/` + `_build/`; symlink them by absolute path.
