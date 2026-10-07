# Test Health — doc-pointers
_Last measured: 2026-10-07 · branch develop@218a391_

Before this change the repo had **no CI workflow at all**: no test, compile, or format check ran on
PRs or pushes, and nothing was released from GitHub (the MCP server runs locally from a checkout).

| Metric | Before | After |
|---|---|---|
| CI PR wall-clock — critical path (warm / cold) | none (no CI) | see PR / report |
| Main release build (warm / cold) | n.a. (no image) | n.a. |
| CI acceptance test job (warm / cold) | none | see PR / report |
| Local full-suite runtime (uptime load) | 4.2s (load 31) | 4.3s (load 31) |
| Docker build (warm / cold) | n.a. (no Dockerfile) | n.a. |
| Tests in acceptance / slow tier | 56 / 0 (never run in CI) | 56 / 0 |
| Async modules / total | 2 / 8 | 2 / 8 |
| Coverage — acceptance pass | 66.10% (local, unenforced) | 66.10% |
| Coverage — full pass | same suite | same suite |
| Coverage gate | — | 61% (`mix.exs` `test_coverage`, enforced by `mix test --cover` in CI) |

## Caching status
- GitHub Actions: deps ✅ · build (`_build/test`) ✅ · npm n.a. · .next/cache n.a. · PLT n.a. (no dialyzer) · develop-ref seeding ✅ (`push: develop`)
- Cache key re-saves per sha; restore/save are split so a red test step still saves the compile.
- Docker: n.a. (no Dockerfile, no image) · release-cache warmer n.a.

## Slow tests (tier: nightly)
None. Whole suite is ~4s; slowest test is the git-submodule superproject test (~0.6s, shells out to `git`).

## Test debt
| Item | Kind | Notes |
|---|---|---|
| `DocPointers.StoreTest`, `MCPTest`, `MCP.RuntimeTest`, `MCP.WritesTest`, `MCP.Tools.GenerateTest`, `DocPointersTest` | sync-only | `DocPointers.Store` is a single named GenServer started by the application; tests reset it per test. Sync phase is ~4s, so async flips aren't worth a store-per-test seam yet. |
| `DocPointers.MCP.Tools.List`, `MCP.Tools.Lookup` | coverage gap | 0% — read-only MCP tools have no direct tests. |
| `Mix.Tasks.DocPointers.Mcp.Server/Stdio/MigrateStores` | coverage gap | 0% — mix task entrypoints untested. |
| `DocPointers.MCP.Runtime` | coverage gap | 34.78%. |
| Elixir/OTP pin | toolchain | CI uses `.tool-versions` (Elixir 1.20.1 / OTP 29.0.2); `mix.exs` allows `~> 1.18`. |

## Nightly
Not needed — no slow tier, no Docker image to keep warm.
