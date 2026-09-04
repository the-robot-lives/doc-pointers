# AGENTS.md — doc-pointers

Mint durable, code-stable citations for a source locus: UUIDv5 identity + 4-glyph hieroglyph token pasteable into docs. Monorepo role: Developer MCP/tooling; NPL-tooling adjacent.

## Stack & commands

Elixir. `mix deps.get` · `mix test` · `mix format`.

## Universal rules (monorepo policy)

- **Trinity Protocol (REQUIRED)**: substantive responses follow Orientation (assumption table, minds-eye, mermaid plan) → Friction (WEDGE/SHADOW/CRITIC) → Response + meta-review. Full text: monorepo `protocols/the-trinity-protocol.md`.
- **No shell in main thread** — delegate lookups/builds/greps to tasker subagents; batch and summarize.
- **Worktree workflow (REQUIRED)**: all work on worktrees; integration-testing consolidation branches `epic.<group>` fork from active `develop`; feature branches merge into their parent epic via squash-PR (provenance); a fully-passing epic becomes one PR for the group. See monorepo CLAUDE.md "Git Trees — Worktree Workflow".

Monorepo-wide ops (secrets/dc, terraform, submodules, tiers): see `../../../CLAUDE.md` at the trl-infra root.
