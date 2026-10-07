# doc-pointers

**Repo:** https://github.com/the-robot-lives/doc-pointers

Mint durable, code-stable citations for a source locus: a **UUIDv5** identity plus a **4-glyph hieroglyph token** you can paste into docs. Line numbers rot; `⟦𓳔𔐮𔘟𔄵⟧` does not.

## What

An Elixir (`~> 1.18`) Mix app (`:doc_pointers`) with two surfaces sharing one store:

- **MCP** — stdio (preferred) or loopback Streamable HTTP, so agents can resolve pointers.
- **Library** — `DocPointers.generate/3,4` for Elixir callers.
- **CLI** — Rust `doc-pointers` scans markers and Markdown, using the Elixir store as its persistence backend. See [CLI guide](docs/CLI.md).

Default MCP tools are read-only (`doc-pointer/lookup`, `doc-pointer/list`); `generate`/`update` stay off unless you pass `--write` or `confirm=true`.

## Why

Docs that cite `file:line` go stale the moment code moves. A doc-pointer is an identity for a *locus* that survives refactors — the token resolves to the code wherever it lives now. Part of the Noizu NPL dev-tooling MCP family (the same review/memory surfaces that consume doc-pointers).

## Getting Started

```bash
mix deps.get
mix compile
mix test
mix doc_pointers.mcp.stdio --root /path/to/project   # stdio MCP (default read-only)
DOC_POINTERS_PORT=4242 mix doc_pointers.mcp.server --root /path/to/project   # optional HTTP
```

`--root` defaults to `DOC_POINTERS_ROOT` or cwd. Add `--write` (or `DOC_POINTERS_MCP_WRITES=1`) to expose generate/update without per-call `confirm=true` — same flag on the HTTP task. The HTTP listener binds to `127.0.0.1` only, with no auth; its MCP endpoint is `http://127.0.0.1:4242/` (or your selected port). Run `mix compile` once so Mix doesn't print to stdout and corrupt the stdio stream. On launch, either task prints connection commands to stderr, leaving the stdio JSON-RPC stream clean.

## MCP Client Install

Replace `/ABS/doc-pointers` with this checkout and `/ABS/project` with the document root. The launch banner prints these commands with the actual paths and port.

**Stdio (Claude Code and Codex):**

```bash
claude mcp add doc-pointers -- sh -c 'cd "$1" && exec mix doc_pointers.mcp.stdio --root "$2"' sh /ABS/doc-pointers /ABS/project
codex mcp add doc-pointers -- sh -c 'cd "$1" && exec mix doc_pointers.mcp.stdio --root "$2"' sh /ABS/doc-pointers /ABS/project
```

These register a subprocess that starts in the repository and uses the specified document root, regardless of the client's working directory. To enable write tools, add `--write` to the `mix doc_pointers.mcp.stdio` portion of the command.

**HTTP (Claude Code and Codex):** Start `mix doc_pointers.mcp.server` first, then register its URL:

```bash
claude mcp add --transport http doc-pointers http://127.0.0.1:4242/
codex mcp add doc-pointers --url http://127.0.0.1:4242/
```

**Claude Desktop** (`claude_desktop_config.json`):

```json
{"mcpServers": {"doc-pointers": {"command": "mix", "args": ["doc_pointers.mcp.stdio"], "cwd": "/ABS/doc-pointers"}}}
```

**Cursor** (`.cursor/mcp.json`): same JSON object as Claude Desktop.

**VS Code** (`.vscode/mcp.json`):

```json
{"servers": {"doc-pointers": {"type": "stdio", "command": "mix", "args": ["doc_pointers.mcp.stdio"], "cwd": "/ABS/doc-pointers"}}}
```

**Codex** (`~/.codex/config.toml`):

```toml
[mcp_servers.doc-pointers]
command = "mix"
args = ["doc_pointers.mcp.stdio"]
cwd = "/ABS/doc-pointers"
startup_timeout_sec = 60
```
