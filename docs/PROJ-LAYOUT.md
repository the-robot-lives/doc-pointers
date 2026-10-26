# Project Layout — doc-pointers

Elixir Mix app (`:doc_pointers`) that mints **UUIDv5-derived 4-character hieroglyph
tokens** (Egyptian / Meroitic / Anatolian blocks) as durable cross-document code
references. Programmatic API + MCP tools; persistence in `.meta/pointers.yaml`
(per project root and detected git submodules). Portfolio submodule:
`Portfolio/Apps/Developer/doc-pointers`.

Plain tree: [`PROJ-LAYOUT.summary.md`](PROJ-LAYOUT.summary.md).

```
doc-pointers/
├── mix.exs · mix.lock            # ★ Mix project :doc_pointers (Elixir ~> 1.18)
├── Makefile · bin/doc-pointers   # Rust CLI: cli-build · cli-test · cli-install; source-run wrapper
├── rust/                         # Rust scanner CLI → [CLI.md](CLI.md)
│   ├── Cargo.toml · Cargo.lock
│   └── src/bin/doc-pointers.rs   #   uuid5 · build · annotate · hook; JSON → mix doc_pointers.cli
├── README.md                     # Front door: MCP + library API, storage, config
├── merge-notes.md                # Working notes from branch merges
├── CLAUDE.md                     # Agent instructions (Claude Code): stack, worktree rules
├── AGENTS.md · AGENT.md          # Mirrored agent instructions (Grok / Codex)
│
├── .formatter.exs                # mix format rules (standard Elixir)
├── .tool-versions                # mise/asdf tool pins (Elixir/Erlang)
├── .gitignore                    # excludes _build/, deps/, .DS_Store
├── .github/workflows/ci.yml      # CI workflow (test, compile, format)
│
├── lib/
│   ├── doc_pointers.ex           # Public API: DocPointers.generate/3,4
│   ├── doc_pointers/
│   │   ├── application.ex        # OTP app; starts Store (DOC_POINTERS_ROOT / cwd)
│   │   ├── uuid5.ex              # UUIDv5 (custom namespace) + name builders
│   │   ├── hieroglyph.ex         # 128-bit UUID → 4-glyph token; legacy ⟦token⟧ alias
│   │   ├── pointer.ex            # %Pointer{} struct + YAML map codec
│   │   ├── store.ex              # GenServer: load/save YAML, token index, submodule split
│   │   ├── mcp.ex                # Noizu.MCP.Server "doc_pointers" tool registry
│   │   └── mcp/
│   │       ├── writes.ex         #   --write / confirm gate
│   │       ├── connection_help.ex #   client registration banner (stdio / HTTP)
│   │       ├── runtime.ex        #   stdio + loopback HTTP boot
│   │       └── tools/
│   │           ├── generate.ex   #   doc-pointer/generate (hidden unless --write)
│   │           ├── generate_batch.ex # doc-pointer/generate-batch; verified paths
│   │           ├── lookup.ex     #   doc-pointer/lookup
│   │           ├── list.ex       #   doc-pointer/list
│   │           └── update.ex     #   doc-pointer/update (hidden unless --write)
│   └── mix/tasks/
│       ├── doc_pointers.mcp.stdio.ex   # mix doc_pointers.mcp.stdio (preferred)
│       ├── doc_pointers.mcp.server.ex  # mix doc_pointers.mcp.server (127.0.0.1 HTTP)
│       ├── doc_pointers.cli.ex         # mix doc_pointers.cli (JSON-lines backend, Rust CLI)
│       └── doc_pointers.migrate_stores.ex # re-split pointers into owning submodule stores
│
├── test/
│   ├── test_helper.exs
│   ├── doc_pointers_test.exs           # E2E golden: UUID5 → hieroglyph
│   └── doc_pointers/
│       ├── uuid5_test.exs · hieroglyph_test.exs · marker_test.exs
│       ├── store_test.exs · store_reconcile_test.exs
│       ├── mcp_test.exs                # MCP tool registry / surface
│       └── mcp/
│           ├── writes_test.exs · connection_help_test.exs
│           ├── runtime_test.exs        #   stdio + HTTP boot
│           └── tools/
│               ├── generate_test.exs · generate_batch_test.exs
│               └── lookup_test.exs
│
└── docs/
    ├── PROJ-LAYOUT.md                  # This file
    ├── PROJ-LAYOUT.summary.md          # Tree-only companion
    ├── PROJ-ARCH.md                    # Design, mint pipeline, data model
    ├── PROJ-ARCH.summary.md            # Short architecture digest
    ├── PROJ-SCHEMA.md                  # Data formats (.meta/pointers.yaml, legacy JSON) + config knobs
    ├── PROJ-SCHEMA.summary.md          # Schema quick-reference
    ├── THREAT-MODEL.md                 # Attack surface, STRIDE register, mitigations
    ├── THREAT-MODEL.summary.md         # Threat-model quick-reference
    ├── CLI.md                          # Rust CLI grammar + workflow reference
    ├── DOCMOST-SYNC.md                 # Docmost wiki mirroring (retired script) reference
    └── TEST-HEALTH.md                  # CI / test-suite health notes
```

Tree is small — no `docs/layout/*` extracts.

## Layer ownership

| Path | Role |
|------|------|
| `lib/doc_pointers.ex` | Library entry: generate + collision-retry |
| `uuid5` / `hieroglyph` / `pointer` | Pure encode/model (no I/O) |
| `store` | In-memory index + YAML at `{root\|submodule}/.meta/pointers.yaml` |
| `mcp` + `mcp/tools/*` | MCP surface over Store / generate logic |
| `mix/tasks/...mcp.stdio` | Preferred local stdio transport |
| `mix/tasks/...mcp.server` | Loopback Streamable HTTP (Bandit + Plug, 127.0.0.1) |
| `rust/` · `bin/doc-pointers` · `Makefile` | Rust scanner CLI (uuid5 · build · annotate · hook); `make cli-install` |
| `mix/tasks/...cli` | JSON-lines store backend (status · reconcile) for the Rust CLI |
| `mix/tasks/...migrate_stores` | Re-split pointers into owning submodule stores |

## Runtime storage (outside this repo tree)

| Path | Role |
|------|------|
| `{DOC_POINTERS_ROOT\|cwd}/.meta/pointers.yaml` | Canonical store (UUID-keyed map) |
| `{root}/{submodule}/.meta/pointers.yaml` | Per-submodule store when root has `.gitmodules` |
| `{root}/docs/doc-pointer-db.json` | Legacy JSON import if YAML empty (e.g. monorepo `docs/doc-pointer-db.json`) |

## Key setup

| Item | Action |
|------|--------|
| Elixir | `~> 1.18`; `mix deps.get` then `mix test` |
| MCP stdio | `mix doc_pointers.mcp.stdio` (`--write` for generate/update) |
| MCP HTTP | `mix doc_pointers.mcp.server` (127.0.0.1, port **4242** or `DOC_POINTERS_PORT`) |
| Root | `--root PATH` or `DOC_POINTERS_ROOT` (default: cwd) |
| Client | `claude mcp add doc-pointers -- mix doc_pointers.mcp.stdio` |
| Rust CLI | `make cli-install` (cargo); `DOC_POINTERS_HOME` pins the Elixir backend checkout |

**Deps (runtime):** `noizu_mcp`, `yaml_elixir`, `ymlr`, `jason`, `bandit`, `plug`.

## Generated / excluded

- `_build/`, `deps/` — Mix artifacts (often present locally; not source)
- Target-project `.meta/` — written at configured root, not packaged here

## Conventions

- Token length fixed at **4** codepoints; uniqueness via attempt suffix on UUIDv5 name
- Markers: typed `〚emoji:UUIDv5〛` (📁 file · 📦 module · 🔌 interface · 🧩 component · 🔧 function · 🔀 logic · 📐 diagram); declaration line appends `Name :: Description`; legacy `⟦TOKEN⟧` forms read for migration only
- Store keys by full UUID; secondary index by token
- Submodule-aware: file paths under a submodule path strip that prefix and write into that submodule’s `.meta/`
