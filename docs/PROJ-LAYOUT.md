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
├── README.md                     # Hex-style scaffold (description still TODO)
│
├── lib/
│   ├── doc_pointers.ex           # Public API: DocPointers.generate/3,4
│   ├── doc_pointers/
│   │   ├── application.ex        # OTP app; starts Store (DOC_POINTERS_ROOT / cwd)
│   │   ├── uuid5.ex              # UUIDv5 (custom namespace) + name builders
│   │   ├── hieroglyph.ex         # 128-bit UUID → 4-glyph token; ⟦token⟧ markers
│   │   ├── pointer.ex            # %Pointer{} struct + YAML map codec
│   │   ├── store.ex              # GenServer: load/save YAML, token index, submodule split
│   │   ├── mcp.ex                # Noizu.MCP.Server "doc_pointers" tool registry
│   │   └── mcp/tools/
│   │       ├── generate.ex       #   doc-pointer/generate
│   │       ├── lookup.ex         #   doc-pointer/lookup
│   │       ├── list.ex           #   doc-pointer/list
│   │       └── update.ex         #   doc-pointer/update
│   └── mix/tasks/
│       └── doc_pointers.mcp.server.ex  # mix doc_pointers.mcp.server (Bandit HTTP)
│
├── test/
│   ├── test_helper.exs
│   ├── doc_pointers_test.exs     # E2E golden: UUID5 → hieroglyph
│   └── doc_pointers/
│       ├── uuid5_test.exs
│       ├── hieroglyph_test.exs
│       ├── store_test.exs
│       └── mcp/tools/generate_test.exs
│
└── docs/
    ├── PROJ-LAYOUT.md            # This file
    └── PROJ-LAYOUT.summary.md    # Tree-only companion
```

Tree is small — no `docs/layout/*` extracts.

## Layer ownership

| Path | Role |
|------|------|
| `lib/doc_pointers.ex` | Library entry: generate + collision-retry |
| `uuid5` / `hieroglyph` / `pointer` | Pure encode/model (no I/O) |
| `store` | In-memory index + YAML at `{root\|submodule}/.meta/pointers.yaml` |
| `mcp` + `mcp/tools/*` | MCP surface over Store / generate logic |
| `mix/tasks/...mcp.server` | Streamable HTTP server (Bandit + Plug) |

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
| MCP HTTP | `mix doc_pointers.mcp.server` (port **4242** or `DOC_POINTERS_PORT`) |
| Root | `--root PATH` or `DOC_POINTERS_ROOT` (default: cwd) |
| Client | `claude mcp add doc-pointers --transport http http://localhost:4242/mcp` |

**Deps (runtime):** `noizu_mcp`, `yaml_elixir`, `ymlr`, `jason`, `bandit`, `plug`.

## Generated / excluded

- `_build/`, `deps/` — Mix artifacts (often present locally; not source)
- Target-project `.meta/` — written at configured root, not packaged here

## Conventions

- Token length fixed at **4** codepoints; uniqueness via attempt suffix on UUIDv5 name
- Markers: `⟦TOKEN⟧`; declaration line: `⟦TOKEN⟧ Name :: Description`
- Store keys by full UUID; secondary index by token
- Submodule-aware: file paths under a submodule path strip that prefix and write into that submodule’s `.meta/`
