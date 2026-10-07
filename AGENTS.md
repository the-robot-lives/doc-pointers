# AGENTS.md — doc-pointers

Guidance for **Codex**, **Grok**, **Cursor**, and other `AGENTS.md` / `AGENT.md` tools.

Claude Code loads [CLAUDE.md](./CLAUDE.md). Same policy; this file is the harness-shaped sibling (numbered MUST first, markdown headings). If both this file and a parent `AGENTS.md` load, **this file wins on conflict**.

## MUST (every turn)

1. **Trinity Protocol (REQUIRED)**: substantive responses follow Orientation (assumption table, minds-eye, mermaid plan) → Friction (WEDGE/SHADOW/CRITIC) → Response + meta-review. Full text: monorepo `protocols/the-trinity-protocol.md`.
2. **No shell in main thread** — delegate lookups/builds/greps to tasker subagents; batch and summarize.
3. **Worktrees (REQUIRED)**: canonical placement `.claude/worktrees/<name>/`, created from this repo's own `.git` off `develop`; `.claude/worktrees/` is gitignored — see **Worktrees — Canonical Convention** below.
4. **PRs target `develop`.** Never merge or push `main` (CI/CD-only release path).
5. **PR / CI monitoring:** use `gh-wait` with `--snapshot` for one-shot checks; do not write polling loops. See **Monitoring** below.

## Identity

Mint durable, code-stable citations for a source locus: UUIDv5 identity + 4-glyph hieroglyph token pasteable into docs. Monorepo role: Developer MCP/tooling; NPL-tooling adjacent.

## Stack & commands

Elixir. `mix deps.get` · `mix test` · `mix format`.

## Branch & PR Policy

- Submodules sit on **`develop`** — keep your checkout on `develop`.
- All PRs target **`develop`** (feature/bug/task branches fork from `develop`).
- **`main` is CI/CD-only**: CI/CD automation performs all merges into `main` (release path). Never merge to or push `main` by hand.

## Worktrees — Canonical Convention (REQUIRED)

All work happens on git worktrees, created from **this repo's own `.git`** — never work directly on a shared checkout of `develop`/`main`.

- **Placement (fixed):** every worktree lives inside this repo's checkout at **`.claude/worktrees/<name>/`** — never siblings (`<repo>.worktrees/`), never ad-hoc paths. Matches Claude Code's native worktree tooling, so harness-created and manual worktrees coexist.
- **Naming:** `<name>` = branch name with `/` → `-` (branch `feature/vfs-wave1` → `.claude/worktrees/feature-vfs-wave1`).
- **Creation** — from this repo's own `.git`, based on `develop` (never `main`):

  ```bash
  git -C <this-repo> worktree add .claude/worktrees/<name> -b <branch> develop
  ```

- **Hygiene:** `.claude/worktrees/` is gitignored in this repo; never commit its contents. One worktree per task; remove it when the work lands (`git worktree remove .claude/worktrees/<name>` — keep the branch).
- **Addressing:** `git -C <this-repo>/.claude/worktrees/<name> …`; verify branch + clean index before any git write; no `git stash`.
- **Elixir projects:** the MAIN checkout owns `deps/` + `_build/`; each worktree symlinks `deps` (and `_build` where needed) to the canonical checkout by **absolute path** — no per-worktree re-fetch/recompile.
- **Legacy placements** (`.worktrees/`, `.wt/`, `<repo>.worktrees/` siblings, `staging/`) are grandfathered — do not create new ones; migrate opportunistically. `staging/` remains local-only experiments (never pushed/submoduled).

## Monitoring PRs and CI — `gh-wait`

Use the monorepo's `gh-wait` (`~/.local/bin`) for PR, review, checks, workflow-run, and deploy status. It replaces ad-hoc `gh`/`kubectl` sleep loops. `-R owner/repo` selects a repository (otherwise inferred from the current remote); `--interval 30s` and `--timeout 30m` are defaults. `--json` emits one result object; `--quiet` suppresses progress. Exit codes are `0` success, `1` terminal failure, `2` timeout or pending with `--snapshot`, and `3` usage/tool error. It is read-only except the opt-in `run --rerun-cancelled` action.

```bash
gh-wait status 48 -R the-robot-lives/doc-pointers
gh-wait pr-review 48 --bot robot --since now --any-comment
gh-wait pr-checks 48 --snapshot --json
gh-wait pr-state --head feature/x --until merged --snapshot
```

**Waiting style (main thread and subagents):** Prefer a Monitor or scheduled wake-up followed by `gh-wait <command> --snapshot`; pending exits `2`, so check again on the next wake. If the next step truly depends on a terminal state, use a foreground `gh-wait` call with `--timeout 3m`, then `--timeout 5m` on further pending results. Stop on exits `0`, `1`, or `3`, or after about 60 minutes total unless the task sets another limit. Keep the enclosing tool timeout longer than the `gh-wait` timeout. Do not use `tail -f`, log-file polling, `gh run watch`, or background-and-follow loops.

Full reference: monorepo `Portfolio/Utilities/source/github-utils/docs/gh-wait.md` and its root `CLAUDE.md` monitoring section.

## Pointers

- Claude Code baseline: [CLAUDE.md](./CLAUDE.md)
