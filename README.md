# doc-pointers

**Repo:** https://github.com/the-robot-lives/doc-pointers

Mint durable, code-stable citations for a source locus: a **UUIDv5** identity plus a **4-glyph hieroglyph token** you can paste into docs. Line numbers rot; `⟦𓳔𔐮𔘟𔄵⟧` does not.

## What

An Elixir (`~> 1.18`) Mix app (`:doc_pointers`) with two surfaces sharing one store:

- **MCP** — stdio (preferred) or loopback Streamable HTTP, so agents can resolve pointers.
- **Library** — `DocPointers.generate/3,4` for Elixir callers.

Default MCP tools are read-only (`doc-pointer/lookup`, `doc-pointer/list`); `generate`/`update` stay off unless you pass `--write` or `confirm=true`.

## Why

Docs that cite `file:line` go stale the moment code moves. A doc-pointer is an identity for a *locus* that survives refactors — the token resolves to the code wherever it lives now. Part of the Noizu NPL dev-tooling MCP family (the same review/memory surfaces that consume doc-pointers).

## Getting Started

```bash
mix deps.get
mix compile
mix test
mix doc_pointers.mcp.stdio --root /path/to/project   # stdio MCP (default read-only)
mix doc_pointers.mcp.server --port 4242 --root /path/to/project   # optional HTTP, 127.0.0.1 only, no auth
```

`--root` defaults to `DOC_POINTERS_ROOT` or cwd. Add `--write` (or `DOC_POINTERS_MCP_WRITES=1`) to expose generate/update without per-call `confirm=true` — same flag on the HTTP task. Run `mix compile` once so Mix doesn't print to stdout and corrupt the stdio stream.

## MCP Client Install

Replace `/ABS/doc-pointers` with this checkout.

**Claude Code:**

```bash
claude mcp add doc-pointers -- mix doc_pointers.mcp.stdio          # lookup/list only
claude mcp add doc-pointers -- mix doc_pointers.mcp.stdio --write  # with generate/update
```

From another directory:

```bash
claude mcp add-json doc-pointers '{"command": "mix", "args": ["doc_pointers.mcp.stdio"], "cwd": "/ABS/doc-pointers"}'
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
