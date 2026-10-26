# Architecture Summary — doc-pointers

UUIDv5 → 4-glyph hieroglyph tokens for durable cross-doc code refs.
Path: `Portfolio/Apps/Developer/doc-pointers`. Full: [`PROJ-ARCH.md`](PROJ-ARCH.md).

## What it is

- Elixir OTP app (`:doc_pointers`) + MCP server (stdio preferred; optional Bandit on **127.0.0.1:4242**)
- Deterministic tokens: `doc-pointers:{name}[:salt][:attempt]` under fixed UUIDv5 NS
- Markers typed `〚emoji:UUIDv5〛` (legacy `⟦TOKEN⟧` readable); store `.meta/pointers.yaml` (per root + git submodules)
- Legacy import: `docs/doc-pointer-db.json` if YAML empty

## Components (one line each)

| Piece | Role |
|-------|------|
| `DocPointers` | Public generate + collision-retry |
| `UUID5` / `Hieroglyph` / `Pointer` | Pure encode + model |
| `Store` | GenServer YAML index + submodule split |
| `MCP` + tools | lookup · list (default); generate · update · generate-batch (`--write`) |
| `Marker` | Typed `〚emoji:UUID〛` grammar + canonical kinds |
| Mix tasks | `mcp.stdio` (preferred); `mcp.server` loopback HTTP; `cli` JSON backend; `migrate_stores` |
| Rust CLI | `rust/` + `bin/doc-pointers`: scan · annotate · `build --check` hook |

## Flows

```
generate: name → UUIDv5 → 4 glyphs → retry if token taken → Store.put → YAML
put path: resolve .gitmodules longest prefix → strip path → write that store’s YAML
MCP:     client → stdio or 127.0.0.1 Bandit → DocPointers.MCP tools → Store / generate
CLI:     scan/annotate → JSON → mix doc_pointers.cli → Store reconcile → YAML
```

## Design bullets

- UUID primary key; token secondary index
- Collision = attempt suffix (max 10k)
- Glyphs from Egyptian / Meroitic / Anatolian ranges
- Update mutates metadata only (not token/uuid)
- Dual surface: library embed + agent MCP

## Stack

Elixir 1.18 · noizu_mcp · bandit/plug · yaml_elixir/ymlr · jason · :crypto · Rust scanner CLI

No SQL/DB — YAML + legacy-JSON data formats and config knobs: [`PROJ-SCHEMA.md`](PROJ-SCHEMA.md)

## Ops

```
mix doc_pointers.mcp.stdio [--root PATH] [--write]
mix doc_pointers.mcp.server [--port 4242] [--root PATH] [--write]
# DOC_POINTERS_ROOT, DOC_POINTERS_PORT, DOC_POINTERS_MCP_WRITES
```

Non-goals: multi-node Store, MCP auth.
