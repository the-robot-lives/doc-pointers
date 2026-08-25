# doc-pointers

Mint durable, code-stable citations for a source locus: a **UUIDv5** identity plus a
**4-glyph hieroglyph token** you can paste into docs.

Line numbers rot. `⟦𓳔𔐮𔘟𔄵⟧` does not.

Elixir `~> 1.18` Mix app (`:doc_pointers`). Two surfaces share one store:

- **MCP** — Streamable HTTP tools for agents (`doc-pointer/generate`, `lookup`, `list`, `update`)
- **Library** — `DocPointers.generate/3,4` for Elixir callers

## Quick start

```bash
mix deps.get
mix test
mix doc_pointers.mcp.server --port 4242 --root /path/to/project
```

Server: `http://localhost:4242/mcp` (no auth — local-dev only).

Point an MCP client at that URL (Streamable HTTP). Claude Code:

```bash
claude mcp add doc-pointers --transport http http://localhost:4242/mcp
```

The process must be running; a handshake failure usually means the Mix task is not up.

## What a pointer is

Each pointer is a pair:

| Piece | Role |
|-------|------|
| UUIDv5 | Stable identity; YAML map key |
| 4-glyph token | Human/doc face (Egyptian / Meroitic / Anatolian blocks) |

Markers in prose:

```text
⟦𓳔𔐮𔘟𔄵⟧
⟦𓳔𔐮𔘟𔄵⟧ TestPointer :: Golden vector
```

Generation is deterministic from `file_path` + `function` (+ optional `salt`). The name
hashed under a fixed namespace is:

```text
doc-pointers:{file_path}::{function}[:salt][:attempt]
```

If the derived token is already taken, the attempt suffix increments (max 10 000).

Golden vector (`mix test`):

```text
name  doc-pointers:TestPointer
uuid  5c692577-ad0c-51f1-992c-759b5e5fffb5
token 𓳔𔐮𔘟𔄵
```

Namespace: `64e9408c-37a7-5f92-8893-f149cbde01c0`.

## MCP tools

| Tool | Mutates? | Required | Does |
|------|----------|----------|------|
| `doc-pointer/generate` | yes | `file_path`, `function_name`, `description` | Mint UUID + token; persist; return `marker` / `declaration` |
| `doc-pointer/lookup` | no | one of `token`, `uuid`, `file_path`, `function_name` | Find existing pointers |
| `doc-pointer/list` | no | — | Paginated list (`limit` default 50, max 500) |
| `doc-pointer/update` | yes | `uuid` or `token` | Metadata only (`description`, `class`, `line`, `file_path`) |

Optional on generate: `class`, `line`, `salt`, `name_override`.

Optional on list: `file_prefix`, `class`, `limit`, `offset`.

## Elixir API

```elixir
{:ok, pointer} =
  DocPointers.generate("lib/my_app/auth.ex", "login/2", "OIDC login entry",
    class: "MyApp.Auth",
    line: 42
  )

pointer.uuid   # hyphenated UUIDv5
pointer.token  # 4-glyph string

DocPointers.Hieroglyph.marker(pointer.token)
# "⟦…⟧"

DocPointers.Hieroglyph.declaration(pointer.token, "login/2", "OIDC login entry")
# "⟦…⟧ login/2 :: OIDC login entry"
```

Returns `{:ok, %DocPointers.Pointer{}}` or `{:error, :max_attempts}`.

## Storage

Pointers live at `{root}/.meta/pointers.yaml`, keyed by UUID.

If `{root}` has a `.gitmodules`, a pointer whose `file_path` falls under a submodule is
written to **that submodule’s** `.meta/pointers.yaml` (longest-path match; prefix stripped).

When YAML is empty, the store will import legacy `{root}/docs/doc-pointer-db.json` once
(the older Rust CLI format).

## Configuration

| Flag / env | Default | Meaning |
|------------|---------|---------|
| `--root` / `DOC_POINTERS_ROOT` | cwd | Project root for `.meta/` |
| `--port` / `DOC_POINTERS_PORT` | `4242` | HTTP port for `/mcp` |
| `config :doc_pointers, root: …` | — | OTP app env, used if the env var is unset |

## Mix dependency (git)

This is **not** a Hex package. From another Mix project:

```elixir
def deps do
  [
    {:doc_pointers, git: "git@github.com:the-robot-lives/doc-pointers.git"}
  ]
end
```

## Not in this repo

- Scanning trees for `⟦…⟧` markers, or CI that enforces them
- Auth on the HTTP MCP endpoint
- Multi-node store clustering
- Target-project `.meta/` files (they belong in the annotated repo)
- Mix artifacts (`_build/`, `deps/`)

A Rust CLI with the same encode pipeline (scan / mint / git hook, not MCP) lives in
[`the-robot-lives/util-misc`](https://github.com/the-robot-lives/util-misc) as `doc-pointers`.

## Docs

| Doc | Role |
|-----|------|
| [docs/PROJ-ARCH.md](docs/PROJ-ARCH.md) | Design, mint pipeline, data model |
| [docs/PROJ-ARCH.summary.md](docs/PROJ-ARCH.summary.md) | Short architecture digest |
| [docs/PROJ-LAYOUT.md](docs/PROJ-LAYOUT.md) | Directory map and setup |
| [docs/PROJ-LAYOUT.summary.md](docs/PROJ-LAYOUT.summary.md) | Tree-only companion |
