# Threat Model Summary — doc-pointers

Local single-user tool: Elixir library + MCP server (stdio preferred; optional
unauthenticated loopback HTTP `127.0.0.1:4242`). No secrets, no network
service, no multi-tenant data. Full model: [THREAT-MODEL.md](THREAT-MODEL.md).

**Assets:** integrity of target `.meta/pointers.yaml`; writes contained to
configured root; token uniqueness.

**Trust boundaries:** MCP-client ↔ server (LLM-influenced args) · local host ↔
loopback HTTP · server ↔ target filesystem (`--root`/env/cwd + `.gitmodules`
paths).

**Register:** 7 entries — 2 mitigated, 1 partial, 2 open (accepted), 2
informational.

- T-001 (Med, partial): loopback HTTP has no auth; any local process can be a
  client. Control = loopback bind + stdio preferred.
- T-002 (Low, mitigated): prompt-injected tool args; write tools hidden unless
  `--write` / `confirm: true`.
- T-003 (Low, open/accepted): `.gitmodules` store paths trusted verbatim
  (`../` could escape root).
- T-004 (Low, mitigated): legacy `doc-pointer-db.json` import — one-shot,
  empty-stores only, re-encoded to YAML.
- T-005 (Low, mitigated): DoS — retry cap 10 000, list cap 500/page.
- T-006/T-007 (Info, accepted): no write audit log (git history suffices);
  pointer content is low-sensitivity.

**Residual risk:** loopback-only unauthenticated HTTP and `.gitmodules` path
trust accepted in the local single-user threat context.
