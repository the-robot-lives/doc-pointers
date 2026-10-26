# Project Layout Summary — doc-pointers

Elixir MCP/library for UUIDv5 → 4-glyph doc-pointer tokens
(`Portfolio/Apps/Developer/doc-pointers`).

```
doc-pointers/
├── mix.exs · mix.lock · README.md · merge-notes.md
├── Makefile · bin/ · rust/             # Rust scanner CLI (cargo) + launcher
├── CLAUDE.md · AGENTS.md · AGENT.md      # agent instructions (Claude / Grok / Codex)
├── .formatter.exs · .tool-versions · .gitignore · .github/
├── lib/
│   ├── doc_pointers.ex                 # public generate API
│   ├── doc_pointers/
│   │   ├── application.ex              # OTP + Store root
│   │   ├── uuid5.ex · hieroglyph.ex · marker.ex · pointer.ex
│   │   ├── store.ex                    # YAML GenServer + submodule split
│   │   ├── mcp.ex                      # MCP server registry
│   │   └── mcp/                        # writes gate, runtime, connection help, tools
│   └── mix/tasks/                      # mcp.stdio (preferred), mcp.server, cli backend, migrate_stores
├── test/                               # uuid5, hieroglyph, marker, store (+reconcile), MCP tools, golden
└── docs/
    ├── PROJ-LAYOUT.md · PROJ-LAYOUT.summary.md
    ├── PROJ-ARCH.md · PROJ-ARCH.summary.md
    ├── PROJ-SCHEMA.md · PROJ-SCHEMA.summary.md
    ├── THREAT-MODEL.md · THREAT-MODEL.summary.md
    └── CLI.md · DOCMOST-SYNC.md · TEST-HEALTH.md
```

Runtime data (at target root): `.meta/pointers.yaml` (+ optional legacy
`docs/doc-pointer-db.json` import).

See [PROJ-LAYOUT.md](PROJ-LAYOUT.md) for annotated tree and setup.
