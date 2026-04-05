# dbshell — Implementation Roadmap

Four phases from foundation to a usable tool surface. Each phase builds on the previous and is independently testable.

### Key architectural concept: tool + path = operation

The same path resolves to different `DbOperation`s depending on which tool is invoked. The **tool** determines intent, the **path** determines target:

| Command | Path | Operation |
|---------|------|-----------|
| `ls /db/tables/users` | Table | `ListTables` (show views/subdirectories) |
| `cat /db/tables/users` | Table | `DescribeTable` (show schema + constraints) |
| `find /db/tables/users` | Table | `QueryTable` (return rows) |
| `ls /db/vectors/tracks` | Collection | `ListCollections` (show collection contents) |
| `cat /db/vectors/tracks` | Collection | `InspectCollection` (show schema + stats) |
| `find /db/vectors/tracks` | Collection | `VectorSearch` or record listing |

This means `VirtualFS::resolve()` must accept both the parsed path **and** the tool context to produce the correct `DbOperation`. In Phase 1, `resolve()` only handles path-based resolution (the default/`cat` behavior). Phase 4 extends it to be tool-aware — the tool layer in `dbshell-tools` is responsible for combining tool semantics with VFS path resolution to produce the final operation.

---

## Phase 1 — Foundation (complete)

Crate skeletons, all core types, VfsPath parsing, VirtualFS resolve, DbDriver trait, MemoryDriver.

### Scope

- Workspace scaffold with 5 crates (`dbshell-core`, `dbshell-drivers`, `dbshell-tools`, `dbshell-mcp`, `dbshell-py`)
- All core types from DESIGN.md: `VfsPath`, `VfsPathKind`, `Filter`, `DbOperation`, `Record`, `CollectionInfo`, `TableInfo`, `TableSchema`, `TableQuery`, `MergeRequest`, `GraphQuery`, `ResultSet`, `ToolResult`, `ToolPayload`, `DbError`, `CacheKey`, `ViewMount`, `SessionMode`
- `DbDriver` trait with all methods and defaults
- `Embedder` trait, `DriverTransaction` trait
- `VfsPath::parse()` — all 18 path variants with trailing-slash normalization
- `VirtualFS::resolve()` — pure path-to-`DbOperation` mapping with symlink and view resolution
- `MemoryDriver` — full in-memory `DbDriver` implementation with filter evaluation, vector search, table CRUD
- `matches_filter`, `cmp_json_values`, `like_match` as reusable utilities in `dbshell-core::filter`

### Tests

- 26 VfsPath parsing tests (all path patterns + error cases)
- 12 VirtualFS resolve tests (views, symlinks, operations)
- 30 MemoryDriver tests (collections, vector search, table queries, writes)

---

## Phase 2 — Session + Pipeline Execution

Wire up the runtime: session lifecycle, command parsing, pipeline optimization, caching, and transactions.

### Scope

- `Session` struct with `exec_tool()` and `exec()` methods
- Extend `VirtualFS::resolve()` to accept a `ToolKind` parameter — the same path must produce different `DbOperation`s depending on the tool (e.g., `ls` vs `cat` vs `find` on `/db/tables/users`)
- `CommandLine` parser: pipes (`|`), sequential (`;`), parallel (`&`), transaction control (`begin`/`commit`/`rollback`)
- `Pipeline`, `PipeStage`, `PushdownCapability` types
- `PipelineOptimizer` — greedy pushdown of `head`/`tail`/`grep`/`filter` into server-side `DbOperation`
- `ExecutionPlan` — split into server op + remaining client stages
- `QueryRouter` — dispatch `DbOperation` to correct driver by name, validate capabilities
- `CachedQueryRouter` — session-scoped cache (moka) with invalidation rules per write operation
- `ResultStore` for `/results/last` and `/results/<uuid>` (write-once, last-writer-wins)
- Transaction lifecycle: `begin` acquires `DriverTransaction`, ops dispatch through tx, `commit`/`rollback` clears
- `SessionMode` enforcement (read-only blocks writes)
- Concurrency: `;` = sequential, `&` = parallel via `tokio::spawn`, `&` inside transactions = parse error

### Tests

- CommandLine parsing: pipes, semicolons, ampersands, transaction keywords, mixed operators
- Pipeline optimization: pushdown folding, materialization boundary detection
- Session lifecycle: builder pattern, connect, mode enforcement
- Transaction semantics: begin/commit/rollback, auto-rollback on drop, nested begin = error
- Cache invalidation: reads cached, writes invalidate matching keys

---

## Phase 3 — PgDriver End-to-End (Docker)

Real database drivers and integration tests against live services.

### Scope

- `PgDriver` implementation (sqlx + pgvector + AGE)
  - Relational: `list_tables`, `describe_table`, `query_table`, `insert_rows`, `upsert_rows`, `update_rows`, `delete_rows`
  - Vector: `upsert` (with pgvector `vector(N)` type), `vector_search` (cosine/euclidean/dot operators)
  - Graph: `graph_query` via AGE — `LOAD 'age'`, Cypher queries through `GraphQuery::Cypher`
  - Transactions: full Postgres transaction support, `READ COMMITTED` isolation
- Docker Compose (`docker-compose.test.yml`):
  - `pgvector/pgvector:pg16` with AGE extension init script
  - `qdrant/qdrant:latest`
  - `surrealdb/surrealdb:latest`
- `QueryRouter` dispatch with real drivers — driver selection by name, capability validation, dialect mismatch detection
- `CachedQueryRouter` integration — verify cache invalidation with real writes
- Mock `Embedder` for vector search tests (deterministic vectors)
- `Filter` translation: `Filter` enum to SQL `WHERE` clauses (parameterized queries)

### Integration Tests

- PgDriver: insert/query roundtrip, upsert by PK, delete with filters, schema introspection
- pgvector: upsert with vectors, similarity search, score ordering
- AGE: Cypher query execution, node/edge creation and traversal
- Transactions: commit persists, rollback reverts, isolation between sessions
- Pipeline pushdown: verify `head -n` becomes `LIMIT`, `grep` becomes `WHERE LIKE`
- Cross-driver dispatch: route operations to correct driver by name

---

## Phase 4 — Unix Tools + Pipeline Composition

The user-facing tool layer that makes dbshell feel like a filesystem.

### Scope

- Tool parsing layer: each tool parses its own argv (flags + path args), then combines tool semantics with `VirtualFS::resolve()` to produce the correct `DbOperation`. The tool is what gives a path its meaning — `cat /db/tables/users` describes the table, `find /db/tables/users` queries its rows, `ls /db/tables/users` lists its views.
- Tool implementations (all flags mirror real Unix commands — no custom flags per CLAUDE.md):
  - `ls` — list collections, tables, views, symlinks
  - `cat` — inspect collection/table schema, read results, execute vector search
  - `find` — query tables/collections with `-delete` and `-exec` flags
  - `grep` — text pattern matching on JSON lines stdout (pushes down as `WHERE LIKE`)
  - `filter` — structured field predicates (`field > value`, `&&`, `||`, `!`)
  - `head -n N` — limit results (pushes down as `LIMIT`)
  - `tail -n +N` — offset results (pushes down as `OFFSET`)
  - `wc -l` — count rows
  - `sort -r -n` — order results client-side
  - `merge --on --left --right --full --anti-left --anti-right --fields` — SQL JOIN
  - `ln -s` — create symlinks in `/links/`
  - `echo >>`/`echo >` — insert (append) and upsert (match on PK) via redirect
  - `rm` — drop collections or remove symlinks
- `filter` expression parser: `field == value`, `field > value`, `field ~ /pattern/`, `&&`, `||`, `!`
- `ToolCall`, `ToolArgs`, `ToolContext` types
- Output formatting: JSON lines (`find`, `merge`, `grep`), newline-separated names (`ls`), JSON object (`cat` on entity), plain integer (`wc`)
- `man <tool>` — embedded static manual pages
- `<tool> --help` — short usage summary

### Tests

- End-to-end: raw input string -> parsed -> executed -> formatted output
- Pipeline composition: `find /db/tables/users | grep Alice | wc -l`
- Pushdown verification: `find ... | head -5` produces `LIMIT 5` in the query, not client-side truncation
- Filter expressions: all operators, type inference from quoting, logical combinators
- Write path: `echo '{"name":"Alice"}' >> /db/tables/users` inserts, `>` upserts
- Merge: inner/left/right/full/anti joins with `--on` condition
- Symlinks: create, use in paths, remove
- Error cases: invalid flags, missing required args, type mismatches
