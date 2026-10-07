# Docmost Sync Reference (retired script)

The `doc-pointers-docmost-sync` script (util-misc, branch
`feature/doc-pointers-docmost-sync`, commit `662d90a`) mirrored doc-pointer
entries to Docmost wiki pages. It was **retired**: it read the legacy flat JSON
db (`docs/doc-pointer-db.json`), which the per-repo `.meta/pointers.yaml`
stores replace. The full script remains in util-misc git history at that
commit. This page records the parts worth reusing.

## Docmost REST contract (as exercised by the script)

- Base: `<DOCMOST_API_URL>/api/…` (e.g. `https://docmost.noizu.com/api`), auth
  `authorization: Bearer $DOCMOST_API_KEY`, `content-type: application/json`.
- **Spaces:** `GET /spaces` — paginated, cursor in `meta.nextCursor`, items in
  `data[]` (`id`, `slug`, `name`).
- **Space-root pages:** `GET /spaces/{sid}/pages?limit=100` — same pagination;
  used to find/create the parent page.
- **Create page:** `POST /spaces/{sid}/pages` with
  `{title, parentId, content}` → `{id}`. There is **no bulk endpoint** — one
  call per page.
- **Child pages:** `GET /pages/{id}/children?limit=100` (cursor pagination).
- **Read content:** `GET /pages/{id}` → `.content` (markdown).
- **Update content:** `PUT /pages/{id}/content` with
  `{content, operation: "replace"}`.

## Patterns worth keeping

- **Write gating:** every write requires `DOCMOST_MCP_WRITES=1` (mirrors
  docmost-mcp); without it the tool is read-only and prints what it *would*
  do. `--dry-run` forces the read-only path even with the flag set.
- **Upsert by title:** locate an existing child page by exact title match
  (titles embed the pointer code, e.g. `Name ⟦CODE⟧`); compare fetched
  content against the desired markdown and only PUT when they differ
  (unchanged pages are skipped, no write churn).
- **Parent auto-create:** the parent page under the space root is created on
  first run, then reused.
- **Incremental sidecar map:** a JSON sidecar (default
  `docs/doc-pointer-docmost.json`) records `{code: {space, pageId}}` after
  each run so re-runs can address pages directly without re-discovery.

## Follow-up idea (not ported)

The same upsert flow adapts directly to `.meta/pointers.yaml` stores: iterate
pointer entries (code + description + location) instead of JSON db keys, and
key the sidecar by full UUID as the yaml stores do. Trivial port if a docmost
mirror is ever wanted again.
