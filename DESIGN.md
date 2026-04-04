# dbshell — Design Document

> **Status:** Draft  
> **Author:** Adrian Moses 
> **Last updated:** 2026-04-04

---

## Overview


dbshell is a Rust library (with Python bindings via PyO3) that gives agents a
virtual filesystem interface to databases. Agents navigate virtual paths, run
Unix-style tools (`ls`, `cat`, `find`, `grep`), and get structured results back.

The underlying operations map to real database queries against vector, relational and graph databases. 
For v1, the following is supported: pgvector/AGE, Qdrant, and SurrealDB.

### Design goals

- [ ] Backend-agnostic tool surface. Easy to use library for any agent 
- [ ] Composable Unix tool semantics. Support for pipes and complex filtering
- [ ] Filesystem shared across calls
- [ ] Both Rust and Python native support
- [ ] Alternative to Agentic RAG architecture

### Non-goals

- [ ] Replacing MCP servers or external web data extraction
- [ ] Support for both real filesystem and virtual filesystem

---

## Crate structure

```
dbshell/
  crates/
    dbshell-core/      # Session, VirtualFS, QueryRouter, CacheLayer, ResultSet
    dbshell-drivers/   # DbDriver trait + pgvector/AGE, Qdrant, SurrealDB impls
    dbshell-tools/     # Unix tool interface (ls, cat, find, grep, wc, …)
    dbshell-mcp/       # MCP server exposing Session as tool-calls
    dbshell-py/        # PyO3 bindings (maturin build target)
```

### Dependency graph

```
dbshell-tools  →  dbshell-core  →  dbshell-drivers
dbshell-mcp    →  dbshell-tools
dbshell-py     →  dbshell-tools
```

- `dbshell-core` owns `Pipeline`, `PipelineOptimizer`, `ExecutionPlan`, and `PushdownCapability`.
- `dbshell-tools` owns individual tool implementations and `PipeStage` parsing (determining each tool's `PushdownCapability`).
- stdout + stderr formatting is a tool-level concern.

---

## Types and traits

### `VfsPath`

A parsed, validated virtual filesystem path. The lingua franca between the tool
layer and VirtualFS. All path parsing rules are defined here.

```rust
pub struct VfsPath {
    pub raw: String;
    pub kind: VfsPathKind;
}

pub enum VfsPathKind {
    DbRoot,
    VectorRoot,
    Collection { name: String },       // cat → schema+stats, ls → records
    GraphRoot,
    GraphNodeRoot,
    GraphEdgeRoot,
    GraphNode { label: String },       // cat → property schema + count
    GraphEdge { edge_type: String },   // cat → property schema + cardinality
    TableRoot,
    Table { name: String },            // cat → column schema + constraints, ls → paginated rows
    View { table: String, view: String },                       // ls → view parameter values
    ViewEntry { table: String, view: String, param: String },   // find/cat → pre-filtered results
    Symlink { name: String },          // resolved transparently to target path
    SearchRoot,                        // ls → shows collection names
    SearchCollection { collection: String },  // ls → nothing useful
    SearchQuery { collection: String, query: String },  // cat → vector search results
    Result { id: ResultId },
    Tmp { name: String },
}
```

#### Parsing rules

- **Reserved namespaces:**
  - `/db/` — database mounts (read-only structure, write-through to drivers)
  - `/results/` — result sets written back by Session
  - `/tmp/` — ephemeral scratch space within a session
  - `/search/` — vector search by filename (query is the path)
  - `/links/` — session-scoped symlinks created by the user/agent
- **Path components:**

```
/db/                          # ls → shows vectors/, graphs/, tables/
/db/vectors/                  # ls → shows collection names
/db/vectors/tracks            # cat → schema + stats for tracks collection
/db/graphs/                   # ls → shows nodes/, edges/
/db/graphs/nodes/             # ls → shows label names
/db/graphs/nodes/Artist       # cat → property schema, count, index info
/db/graphs/edges/             # ls → shows edge type names
/db/graphs/edges/WROTE        # cat → property schema, count, cardinality
/db/tables/                   # ls → shows table names
/db/tables/users              # cat → column schema, constraints, indexes
/db/tables/orders/by_customer/     # ls → shows available customer IDs (view)
/db/tables/orders/by_customer/42   # find/cat → orders where customer_id = 42
/search/                      # ls → shows searchable collection names
/search/tracks/               # the collection to search
/search/tracks/blue suede shoes   # cat → vector search results for "blue suede shoes"
/results/                     # ls → shows last, <uuid>s
/results/last                 # cat → most recent ResultSet
/tmp/                         # scratch space
/links/                       # ls → shows symlinks
/links/vip-orders             # symlink → /db/tables/orders/by_customer/42
```


#### Views (predefined filter directories)

Views expose filtered subsets of a table as navigable directory paths.
They are defined at mount time in configuration — not created dynamically
during a session. The path parameter is substituted into a filter template
to produce a `Filter` that the driver executes server-side.

```
/db/tables/orders/by_customer/42     → SELECT * FROM orders WHERE customer_id = 42
/db/tables/orders/by_status/shipped  → SELECT * FROM orders WHERE status = 'shipped'
/db/tables/users/by_role/admin       → SELECT * FROM users WHERE role = 'admin'
```

Views behave like any other path — `find`, `cat`, `sort`, `less`, pipes,
and the pipeline optimizer all work transparently. A view path is just
a `find` with a pre-baked filter.

##### `ViewMount`

```rust
pub struct ViewMount {
    /// Name of the view directory (e.g. "by_customer")
    pub name: String,
    /// The table this view filters
    pub table: String,
    /// The column the path parameter maps to
    pub filter_column: String,
    /// Expected type of the parameter (for validation/casting)
    pub param_type: ParamType,
}

pub enum ParamType {
    String,
    Integer,
    Uuid,
}
```

##### Configuration

Views are declared per-table in the mount configuration, read at session
startup.

```json
{
  "driver": "pg",
  "connection_string": "postgres://...",
  "views": [
    {
      "table": "orders",
      "name": "by_customer",
      "filter_column": "customer_id",
      "param_type": "integer"
    },
    {
      "table": "orders",
      "name": "by_status",
      "filter_column": "status",
      "param_type": "string"
    },
    {
      "table": "users",
      "name": "by_role",
      "filter_column": "role",
      "param_type": "string"
    }
  ]
}
```

##### Path resolution

When VirtualFS resolves a `ViewEntry` path:

1. Look up the `ViewMount` by table + view name
2. Cast the path parameter to the expected `ParamType`
3. Produce a `Filter::Eq { field: filter_column, value: param }`
4. Return a `DbOperation::QueryTable` with the filter pre-applied

This means `find /db/tables/orders/by_customer/42 | filter 'total > 100'`
produces a query with **both** the view filter (`customer_id = 42`) AND the
`filter` predicate (`total > 100`) — the optimizer combines them into a single
server-side query when possible.

##### `ls` on views

`ls /db/tables/orders/by_customer/` lists the view directory itself —
showing that this is a parameterized path. It does **not** enumerate all
possible parameter values (that would require a `SELECT DISTINCT` which
could be expensive on large tables).

`ls /db/tables/orders/` shows both the table's regular tools and any
configured views as subdirectories:

```
$ ls /db/tables/orders/
by_customer/
by_status/
```

> **Open: Views**
>
> - Should views support composite keys? e.g. `/db/tables/orders/by_customer_and_status/42/shipped` — two path segments mapping to two filter columns.
> - Should there be a way to list views across all tables? e.g. `ls /db/views/` as an index.
> - Can views be defined on vector collections and graph entities, or tables only?

#### Symlinks

Symlinks are session-scoped aliases created by the user or agent via `ln -s`.
They provide shortcuts to frequently accessed paths — including view paths.

```
ln -s /db/tables/orders/by_customer/42 /links/vip-orders
ln -s /db/tables/users/by_role/admin /links/admins
ln -s /db/tables/products /links/catalog
```

After creation, the symlink is usable anywhere a path is accepted:

```
find /links/vip-orders | filter 'total > 100'
cat /links/admins | wc -l
echo '{"name":"Widget"}' >> /links/catalog
```

##### Storage

Symlinks are stored in a `HashMap<String, VfsPath>` on `VirtualFS`.
They are session-scoped — created during a session, lost on disconnect.

```rust
pub struct VirtualFS {
    mounts: MountRegistry,
    results: ResultBuffer,
    symlinks: HashMap<String, VfsPath>,  // link name → target path
}
```

##### Resolution

`VirtualFS::resolve()` checks symlinks first. If the path starts with
`/links/<name>`, the target `VfsPath` is substituted before normal
resolution continues. Symlinks resolve one level only — a symlink
pointing to another symlink is an error (no chains in v1).

##### `ln` tool

| Flag | Meaning |
|------|---------|
| `-s` | Create symbolic link (required — hard links don't make sense in a VFS) |

```
ln -s <target_path> <link_path>
```

- `<link_path>` must start with `/links/` — symlinks live in a dedicated namespace.
- `<target_path>` is validated to be a resolvable VFS path at creation time.
- Creating a link with an existing name overwrites the previous link.
- `rm /links/<name>` removes the symlink, not the target.
- `ls /links/` lists all symlinks with their targets (like `ls -l`).
- `ls -l` on any directory shows symlinks inline with an arrow: `vip-orders -> /db/tables/orders/by_customer/42`

> **Open: Symlinks**
>
> - Should symlinks be persistable across sessions via a config file? This would allow teams to share common aliases.
> - Should symlinks support relative paths (e.g. `ln -s by_customer/42 /links/vip` while cwd is `/db/tables/orders/`)?
> - Can symlinks point to `/results/` paths? e.g. `ln -s /results/last /links/latest` — useful but the target is mutable.

#### `/search` — vector search as file access

The `/search/` directory makes vector similarity search feel like reading
a file. The query is the filename — `cat` embeds it and returns results.

```
cat /search/tracks/blue suede shoes
cat /search/tracks/mellow jazz piano
cat /search/documents/quarterly revenue report
```

##### How it works

1. VFS parses the path: `/search/<collection>/<query>`
2. `resolve()` produces a `DbOperation::VectorSearch` with:
   - The collection name from the path
   - The query text (everything after the collection segment)
   - The configured `Embedder` generates the vector at dispatch time
3. The driver executes the similarity search
4. Results are returned as JSON lines on stdout, ranked by score

```
$ cat /search/tracks/blue suede shoes
{"id":"t1","score":0.95,"text":"blue suede shoes","genre":"rock","artist":"Elvis Presley"}
{"id":"t2","score":0.82,"text":"blue moon","genre":"jazz","artist":"Billie Holiday"}
{"id":"t3","score":0.71,"text":"suede jacket blues","genre":"blues","artist":"BB King"}
```

##### Composable with pipes

Because the output is JSON lines, it works with the full pipeline:

```
cat /search/tracks/blue suede shoes | head -5
cat /search/documents/revenue | grep quarterly | wc -l
cat /search/tracks/sad piano music > /tmp/playlist
```

##### `ls` on `/search/`

`ls /search/` lists all vector collections that are searchable (collections
with a configured `Embedder`). `ls /search/tracks/` is a no-op — there
are no "files" to list, only queries to make.

##### Search options

Default search returns 10 results. Options can be passed as query-like
suffixes or via `find` for more control:

```
cat /search/tracks/blue suede shoes              # default: 10 results
cat /search/tracks/blue suede shoes | head -20   # explicit limit
```

> **Open: Search**
>
> - Should search support metadata filtering? e.g. `cat /search/tracks/blue suede shoes | grep rock` — combines vector similarity with text filtering. Or views: `/search/tracks/by_genre/rock/blue suede shoes`.
> - How are spaces in the query handled at the parsing level? The path parser needs to treat everything after `/search/<collection>/` as a single query string, not split on spaces.
> - Should search results include the similarity score as a field, or as metadata outside the JSON payload?

---

### `DbDriver`

The core async trait implemented by every backend driver.

```rust
#[async_trait]
pub trait DbDriver: Send + Sync {
    fn name(&self) -> &str;
    fn db_type(&self) -> DbType;

    async fn health(&self) -> Result<HealthStatus>;
    async fn list_collections(&self) -> Result<Vec<CollectionInfo>>;
    async fn create_collection(&self, spec: &CollectionSpec) -> Result<()>;
    async fn drop_collection(&self, name: &str) -> Result<()>;

    // Vector ops
    async fn upsert(&self, collection: &str, records: Vec<Record>) -> Result<UpsertResult>;
    async fn vector_search(&self, req: &VectorSearchRequest) -> Result<Vec<ScoredRecord>>;
    async fn delete(&self, collection: &str, filter: &Filter) -> Result<u64>;

    // Graph ops — default returns DbError::Unsupported
    async fn graph_query(&self, query: &GraphQuery) -> Result<GraphResult> {
        Err(DbError::Unsupported("graph queries"))
    }

    // Relational ops — default returns DbError::Unsupported
    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        Err(DbError::Unsupported("relational tables"))
    }
    async fn describe_table(&self, name: &str) -> Result<TableSchema> {
        Err(DbError::Unsupported("relational tables"))
    }
    async fn query_table(&self, req: &TableQuery) -> Result<ResultSet> {
        Err(DbError::Unsupported("relational tables"))
    }
    async fn insert_rows(&self, table: &str, rows: Vec<serde_json::Value>) -> Result<u64> {
        Err(DbError::Unsupported("relational tables"))
    }
    async fn upsert_rows(&self, table: &str, rows: Vec<serde_json::Value>) -> Result<u64> {
        Err(DbError::Unsupported("relational tables"))
    }
    async fn update_rows(&self, table: &str, filter: &Filter, set: serde_json::Value) -> Result<u64> {
        Err(DbError::Unsupported("relational tables"))
    }
    async fn delete_rows(&self, table: &str, filter: &Filter) -> Result<u64> {
        Err(DbError::Unsupported("relational tables"))
    }
    async fn merge_tables(&self, req: &MergeRequest) -> Result<ResultSet> {
        Err(DbError::Unsupported("merge"))
    }

    // Raw escape hatch — driver-specific, returns JSON
    async fn raw(&self, query: &str, params: serde_json::Value) -> Result<serde_json::Value>;
}
```

#### `DbType`

```rust
pub enum DbType {
    Vector,
    Graph,
    Relational,
    Hybrid(Vec<DbCapability>), // supports multiple (SurrealDB, PgDriver with AGE)
}

pub enum DbCapability {
    Vector,
    Graph,
    Relational,
}
```

#### Driver capability matrix

| Operation            | PgDriver (pgvector) | PgDriver (+AGE) | QdrantDriver | SurrealDriver |
|----------------------|:-------------------:|:---------------:|:------------:|:-------------:|
| `list_collections`   | ✓                   | ✓               | ✓            | ✓             |
| `create_collection`  | ✓                   | ✓               | ✓            | ✓             |
| `drop_collection`    | ✓                   | ✓               | ✓            | ✓             |
| `upsert`             | ✓                   | ✓               | ✓            | ✓             |
| `vector_search`      | ✓                   | ✓               | ✓            | ✓             |
| `delete`             | ✓                   | ✓               | ✓            | ✓             |
| `graph_query`        | —                   | ✓ (Cypher)      | —            | ✓ (SurrealQL) |
| `list_tables`        | ✓                   | ✓               | —            | ✓             |
| `describe_table`     | ✓                   | ✓               | —            | ✓             |
| `query_table`        | ✓                   | ✓               | —            | ✓             |
| `insert_rows`        | ✓                   | ✓               | —            | ✓             |
| `update_rows`        | ✓                   | ✓               | —            | ✓             |
| `delete_rows`        | ✓                   | ✓               | —            | ✓             |
| `raw`                | ✓                   | ✓               | ✓            | ✓             |

---

### `GraphQuery`

Dialect enum passed to `graph_query`. QueryRouter validates that the dialect
matches the target driver before dispatching.

```rust
pub enum GraphQuery {
    Cypher(String),     // Neo4j, AGE (openCypher)
    SurrealQL(String),  // SurrealDB graph traversal
    Raw(String),        // no dialect validation
}
```

<!-- Note on dialect mismatch: QueryRouter returns DbError::DialectMismatch
     rather than passing an incompatible query to the driver. -->

---

### `Filter`

Structured predicate type — the internal representation for row filtering.
Produced by view path resolution, `grep` pattern parsing, and `filter`
expressions. Defined independently of any driver; QueryRouter translates
to each backend's native filter format.

```rust
pub enum Filter {
    All,
    Eq { field: String, value: serde_json::Value },
    Gt { field: String, value: serde_json::Value },
    Lt { field: String, value: serde_json::Value },
    Gte { field: String, value: serde_json::Value },
    Lte { field: String, value: serde_json::Value },
    Ne { field: String, value: serde_json::Value },
    In { field: String, values: Vec<serde_json::Value> },
    Like { field: String, pattern: String },
    IsNull { field: String },
    Between { field: String, low: serde_json::Value, high: serde_json::Value },
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Not(Box<Filter>),
}
```

#### How filters are expressed (no custom DSL)

There is no JSON filter DSL. Filtering is expressed through three mechanisms,
in order of preference:

**1. Directory structure (views)** — for known, repeated access patterns.
Pre-established column filters configured at mount time. Zero syntax to learn.

```
cat /db/tables/orders/by_customer/42          # customer_id = 42
find /db/tables/users/by_role/admin           # role = 'admin'
cat /db/tables/orders/by_status/shipped       # status = 'shipped'
```

**2. Standard Unix composition** — for ad-hoc text filtering. `grep` for
pattern matching on JSON lines stdout. `sort`, `head`, `tail`, `wc`, `cut`
for shaping results. Real Unix tools operating on text.

```
find /db/tables/users | grep Alice            # text match on stdout
find /db/tables/users | grep active | sort | head -20
find /db/tables/users | wc -l                 # count
```

The pipeline optimizer can recognize `grep` patterns and push them down to
the driver as server-side LIKE clauses when possible.

**3. `filter` for structured filtering** — a narrow, purpose-built dbshell
command for field-level predicates that `grep` can't express (comparisons,
ranges, logical operators). Learnable via `man filter` or `filter --help`.

```
find /db/tables/users | filter 'age > 21'
find /db/tables/users | filter 'age >= 18 && status == "active"'
find /db/tables/orders | filter 'total > 100 || priority == "urgent"'
find /db/tables/users | filter 'name ~ /^A/'
```

The expression syntax is minimal and readable:

| Expression | Filter variant | SQL equivalent |
|---|---|---|
| `field == value` | `Eq` | `field = value` |
| `field != value` | `Ne` | `field != value` |
| `field > value` | `Gt` | `field > value` |
| `field >= value` | `Gte` | `field >= value` |
| `field < value` | `Lt` | `field < value` |
| `field <= value` | `Lte` | `field <= value` |
| `field ~ /pattern/` | `Like` | `field LIKE pattern` |
| `expr && expr` | `And` | `expr AND expr` |
| `expr \|\| expr` | `Or` | `expr OR expr` |
| `!expr` | `Not` | `NOT expr` |

String values are quoted: `status == "active"`. Numeric values are bare:
`age > 21`. The parser infers types from the quoting.

The pipeline optimizer recognizes `filter` expressions and pushes them down
to the driver as server-side WHERE clauses.

#### Driver translation

Each driver translates `Filter` to its native query format. The translation
is internal to the driver — `Filter` is the contract, not the query string.

| Driver | Translation target | Notes |
|--------|-------------------|-------|
| PgDriver | SQL `WHERE` clause (parameterized) | `~` maps to `LIKE`, multiple values to `= ANY($1)` |
| QdrantDriver | `qdrant_client::Filter` + `FieldCondition` | regex not supported — falls back to client-side |
| SurrealDriver | SurrealQL `WHERE` clause | Full operator support |

Drivers that don't support a given filter fall back to client-side
evaluation rather than failing — the optimizer marks the stage as
non-pushable and it runs over materialized stdout.

---

### `TableInfo`, `TableSchema`, and `TableQuery`

Types for relational table support.

```rust
pub struct TableInfo {
    pub name: String,
    pub driver: String,
    pub row_count: Option<u64>,  // approximate, from pg_stat or equivalent
    pub schema_name: Option<String>, // e.g. "public" in Postgres
}

pub struct TableSchema {
    pub table: String,
    pub columns: Vec<ColumnInfo>,
    pub primary_key: Option<Vec<String>>,
    pub indexes: Vec<IndexInfo>,
}

pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,       // driver-reported type string, e.g. "integer", "varchar(255)"
    pub nullable: bool,
    pub default: Option<String>,
}

pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub index_type: String,      // "btree", "hash", "gin", "hnsw", etc.
}

pub struct TableQuery {
    pub filter: Option<Filter>,
    pub columns: Option<Vec<String>>,  // SELECT projection; None = all columns
    pub order_by: Option<Vec<OrderBy>>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub cursor: Option<String>,
}

pub struct OrderBy {
    pub column: String,
    pub descending: bool,
}
```

> **Open: Relational merges**
>
> - Two-table merges are supported via the `merge` tool. Multi-way merges use chained pipes.
> - How should foreign key relationships be surfaced? `cat /db/tables/users` could show FK references, but navigating them is another question.

---

### `Record`

Unit of data flowing into `upsert`. All three drivers map from this shape.

```rust
pub struct Record {
    pub id: String,
    pub vector: Option<Vec<f32>>,       // must be finite — no NaN
    pub source_text: Option<String>,    // original text that was embedded — persisted alongside vector
    pub payload: serde_json::Value,
}
```

<!-- Note: embeddings must be finite before entering Session.
     NaN in a Vec<f32> produces non-deterministic cache keys.
     source_text is populated by the driver after calling Embedder, and stored
     in the collection's payload so it can be returned in search results and
     re-embedded if the model changes. -->

---

### `CollectionInfo`

Returned by `cat` on any entity path. There is no separate `_schema` file —
the entity *is* the metadata. `cat /db/tables/users` returns schema, stats,
and constraints in one response. `ls` returns a lightweight list of names;
`cat` on an individual entity returns the full `CollectionInfo`.

```rust
pub struct CollectionInfo {
    pub name: String,
    pub driver: String,
    pub db_type: DbType,
    pub record_count: u64,

    // Vector collections
    pub dimensions: Option<u32>,
    pub distance_metric: Option<String>,

    // Graph entities
    pub node_labels: Option<Vec<String>>,
    pub edge_types: Option<Vec<String>>,
    pub properties: Option<Vec<PropertyInfo>>,

    // Relational tables
    pub primary_key: Option<Vec<String>>,   // PK column(s) — used by upsert to match rows
    pub columns: Option<Vec<ColumnInfo>>,
    pub foreign_keys: Option<Vec<ForeignKey>>,
    pub constraints: Option<Vec<String>>,
    pub indexes: Option<Vec<String>>,
    // TODO
}

pub struct ForeignKey {
    pub column: String,
    pub references_table: String,
    pub references_column: String,
    pub on_delete: Option<String>,  // CASCADE, SET NULL, RESTRICT, NO ACTION
    pub on_update: Option<String>,
}
```

### `MergeRequest`

Produced by the `merge` tool. Describes a two-table merge that maps to a SQL
JOIN. Multi-way merges are expressed as chained pipes — the optimizer may
merge adjacent `MergeTable` operations into a single multi-table query.

```rust
pub struct MergeRequest {
    pub left: MergeSide,
    pub right: MergeSide,
    pub merge_type: MergeType,
    pub on: MergeCondition,
    pub output_fields: Option<Vec<String>>,  // --fields (projection)
}

pub struct MergeSide {
    pub table: String,
}

pub enum MergeType {
    Inner,            // default
    Left,             // --left
    Right,            // --right
    FullOuter,        // --full
    AntiLeft,         // --anti-left (unmatched from left only)
    AntiRight,        // --anti-right (unmatched from right only)
}

pub struct MergeCondition {
    pub left_col: String,   // --on left_col=right_col
    pub right_col: String,
}
```

---

### `DbOperation`

The contract between VirtualFS and QueryRouter. VirtualFS parses a `VfsPath`
and tool arguments into a `DbOperation`. QueryRouter receives it and dispatches
to the correct driver. **VirtualFS performs no I/O** — it returns only this enum.

```rust
pub enum DbOperation {
    // Structural / inspection
    ListCollections { driver: String },
    InspectCollection { driver: String, collection: String },

    // Vector ops
    VectorSearch { driver: String, collection: String, request: VectorSearchRequest },
    Upsert { driver: String, collection: String, records: Vec<Record> },
    Delete { driver: String, collection: String, filter: Filter },

    // Graph ops
    GraphQuery { driver: String, query: GraphQuery },

    // Relational ops
    ListTables { driver: String },
    DescribeTable { driver: String, table: String },
    QueryTable { driver: String, table: String, request: TableQuery },
    InsertRows { driver: String, table: String, rows: Vec<serde_json::Value> },
    UpsertRows { driver: String, table: String, rows: Vec<serde_json::Value> },
    UpdateRows { driver: String, table: String, filter: Filter, set: serde_json::Value },
    DeleteRows { driver: String, table: String, filter: Filter },
    MergeTable { driver: String, request: MergeRequest },

    // Collection management
    CreateCollection { driver: String, spec: CollectionSpec },
    DropCollection { driver: String, collection: String },

    // VFS-local — Session handles without touching QueryRouter
    ReadResult { path: VfsPath },
    ListResults,
}
```

---

### `CacheKey`

Stable hash of a `DbOperation`. Used as the key for both session-scoped and
persistent cache tiers. Path-based keying is intentionally avoided — the same
path with different filter or limit arguments produces different results.

```rust
#[derive(Hash, PartialEq, Eq, Clone)]
pub struct CacheKey(u64);

impl CacheKey {
    pub fn from_op(op: &DbOperation) -> Self {
        // FxHasher or xxhash over the derived Hash impl of DbOperation
        // TODO
    }
}
```

<!-- DbOperation must derive Hash. All fields must be deterministically
     hashable. Vec<f32> hashes correctly in Rust but NaN != NaN — see
     Record note above. -->

---

### `CacheLayer`

Two-tier: session-scoped in-memory cache (moka) with an optional persistent
backend (Redis, for long-running agents like Edgeclaw).

```rust
pub struct CacheLayer {
    session_cache: moka::future::Cache<CacheKey, CachedResult>,
    persistent: Option<Arc<dyn PersistentCache>>,
}
```

#### Cache invalidation rules

| Write operation               | Invalidates                                      |
|-------------------------------|--------------------------------------------------|
| `Upsert { driver, collection }` | All reads keyed to same driver + collection    |
| `Delete { driver, collection }` | All reads keyed to same driver + collection    |
| `CreateCollection { driver }`   | `ListCollections` for that driver              |
| `DropCollection { driver, collection }` | All reads for that driver + collection |
| `GraphQuery` (write)          | All `GraphQuery` reads for that driver           |

#### Cache policy per operation class

| Operation class          | Policy                              |
|--------------------------|-------------------------------------|
| Schema / structure       | Aggressive TTL (changes rarely)     |
| Vector search results    | Session-scoped or short TTL         |
| Graph traversals         | Session-scoped or short TTL         |
| Write results            | Never cached; triggers invalidation |

---

### `QueryRouter` and `CachedQueryRouter`

```rust
pub struct QueryRouter {
    drivers: HashMap<String, Arc<dyn DbDriver>>,
}

impl QueryRouter {
    pub async fn dispatch(&self, op: DbOperation) -> Result<ResultSet> {
        // 1. Select driver by name from op
        // 2. Validate op is supported by driver's DbType
        // 3. Validate GraphQuery dialect matches driver
        // 4. Dispatch to driver method
        // TODO
    }
}

// Wraps QueryRouter with cache read-through and write invalidation
pub struct CachedQueryRouter {
    inner: QueryRouter,
    cache: CacheLayer,
}
```

<!-- CachedQueryRouter is what Session holds. QueryRouter is testable
     independently without a cache. -->

---

### `Session`

The primary public API. Orchestrates VirtualFS, CachedQueryRouter, and
SessionMode. All tool calls enter through `exec_tool`.

```rust
pub struct Session {
    mode: SessionMode,
    vfs: VirtualFS,
    router: CachedQueryRouter,
    results: RwLock<ResultStore>,
    tx: Mutex<Option<DriverTransaction>>,  // active transaction, if any
    embedder: Option<Arc<dyn Embedder>>,
}

impl Session {
    /// Execute a single tool call (no pipes).
    pub async fn exec_tool(&self, tool: ToolCall) -> Result<ToolResult> {
        // 1. VFS resolves path → DbOperation (pure, no I/O)
        // 2. Validate op against SessionMode
        // 3. If VFS-local op, handle directly (no router)
        // 4. Cache read-through via CachedQueryRouter
        // 5. Dispatch to driver (using active tx if present)
        // 6. Write Records payload to /results/last via ResultStore
        // 7. Return ToolResult
        // TODO
    }

    /// Execute a full input line. Parses into a `CommandLine`, then
    /// dispatches pipelines sequentially or in parallel depending on
    /// separators (; vs &). See "Input parsing" section.
    pub async fn exec(&self, input: &str) -> Result<Vec<ToolResult>> {
        // 1. Parse input into CommandLine (pipelines + separators)
        // 2. For each group:
        //    - Sequential (;): exec_pipeline one at a time
        //    - Parallel (&): spawn all, join results
        // 3. Return collected ToolResults
        // TODO
    }

    /// Execute a pipe chain. Parses → optimizes → executes server-side op →
    /// runs remaining client stages over stdout.
    ///
    /// Error propagation: if any stage fails (server-side or client-side),
    /// the entire pipeline aborts immediately. The failing stage's error
    /// is propagated as the pipeline result — see "Error propagation" section.
    pub async fn exec_pipeline(&self, pipeline: Pipeline) -> Result<ToolResult> {
        // 1. PipelineOptimizer::optimize(pipeline) → ExecutionPlan
        // 2. VFS resolves lead stage path → DbOperation
        // 3. Fold pushdown stages into DbOperation
        // 4. Validate merged op against SessionMode
        // 5. Dispatch merged op via CachedQueryRouter (using active tx if present)
        //    → on error: abort, return error as ToolResult
        // 6. Materialize stdout
        // 7. Run client_stages sequentially over stdout
        //    → on error: abort, return failing stage's error as ToolResult
        // 8. Write final result to ResultStore
        // 9. Invalidate cache if mutation
        // 10. Return ToolResult
        // TODO
    }

    /// Begin an explicit transaction. Returns error if one is already active.
    pub async fn begin(&self) -> Result<()> {
        // Acquires driver transaction, stores in self.tx
        // TODO
    }

    /// Commit the active transaction. Returns error if none is active.
    pub async fn commit(&self) -> Result<()> {
        // Commits driver transaction, clears self.tx
        // TODO
    }

    /// Rollback the active transaction. Returns error if none is active.
    pub async fn rollback(&self) -> Result<()> {
        // Rolls back driver transaction, clears self.tx
        // TODO
    }
}
```

#### `SessionMode`

```rust
pub enum SessionMode {
    ReadOnly,
    ReadWrite,
}
```

<!-- ReadOnly: reject all write DbOperations with DbError::PermissionDenied
     ReadWrite: all ops permitted, transactions available via begin/commit -->

#### `ConnectOptions`

```rust
pub struct ConnectOptions {
    pub mode: SessionMode,
    pub cache: CachePolicy,
    pub max_connections: u32,
    pub connection_string: String, // e.g. "postgres://user:pass@host/db", "http://localhost:6334"
    pub tls: Option<TlsConfig>,
}

pub struct TlsConfig {
    pub ca_cert: Option<PathBuf>,
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    pub accept_invalid_certs: bool, // dev only
}

pub enum CachePolicy {
    None,
    SessionScoped,
    Ttl(Duration),
    Persistent, // uses CacheLayer::persistent backend
}
```

#### Builder example

```rust
let session = Session::builder()
    .connect("pg", pg_driver, ConnectOptions {
        mode: SessionMode::ReadWrite,
        cache: CachePolicy::Ttl(Duration::from_secs(300)),
        max_connections: 10,
    })
    .connect("qdrant", qdrant_driver, ConnectOptions::default())
    .build()
    .await?;
```

---

### `VirtualFS`

Pure namespace and state layer. Holds the mount registry and the `/results/`
buffer. Performs no I/O — `resolve` returns a `DbOperation` without calling
any driver.

```rust
pub struct VirtualFS {
    mounts: MountRegistry,
    results: ResultBuffer, // /results/last and /results/<uuid>
}

impl VirtualFS {
    /// Parse path + args → DbOperation. Pure function, no I/O.
    pub fn resolve(&self, path: &VfsPath, args: &ToolArgs) -> Result<DbOperation> {
        // TODO
    }

    /// Write a ResultSet into /results/. Called by Session after dispatch.
    pub async fn write_result(&self, result: &ResultSet) -> Result<VfsPath> {
        // TODO
    }
}
```

<!-- Only Records payloads are written to /results/. Write confirmations
     (Written, Deleted, Created, Dropped) are not written back — an agent
     cat /results/last can always assume it gets queryable data. -->

---

### `ResultSet`

Structured result returned by all read operations. Drivers produce this;
Session buffers it into VFS; tools format it as stdout.

```rust
pub struct ResultSet {
    pub rows: Vec<serde_json::Value>,
    pub schema: Option<CollectionInfo>,
    pub metadata: ResultMetadata,
}

pub struct ResultMetadata {
    pub driver: String,
    pub collection: Option<String>,
    pub total_count: Option<u64>,  // if known without full scan
    pub query_ms: u64,
    pub cache_hit: bool,
    pub next_cursor: Option<String>, // opaque token for next page, None if last page
}
```

<!-- Arrow IPC (arrow2) is the preferred serialization format for
     cross-language result passing. Gives Polars/pandas interop for free
     in dbshell-py. Decision: TODO confirm arrow2 vs native structs. -->

---

### `ToolResult` and `ToolPayload`

`ToolResult` is the envelope returned by every tool call. `stdout` is always
present. `payload` varies by operation class.

```rust
pub struct ToolResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub payload: ToolPayload,
}

pub enum ToolPayload {
    Records(ResultSet),           // find, vector search, graph query
    Info(CollectionInfo),         // cat /db/tables/users, cat /db/vectors/tracks
    Listing(Vec<CollectionInfo>), // ls /db/
    Written { count: u64 },       // upsert
    Deleted { count: u64 },       // delete
    Created { name: String },     // create collection
    Dropped { name: String },     // drop collection
    ResultRef(VfsPath),           // reference to /results/<uuid>
    Empty,                        // health, ping
}
```

---

## Error taxonomy

```rust
pub enum DbError {
    Unsupported(&'static str),      // operation not supported by this driver
    NotFound(String),               // collection, path, or result not found
    PermissionDenied(String),       // SessionMode violation
    DialectMismatch {               // GraphQuery dialect sent to wrong driver
        expected: &'static str,
        got: &'static str,
    },
    ConnectionFailed(String),       // driver could not connect
    InvalidFilter(String),          // Filter failed validation
    InvalidEmbedding(String),       // non-finite values in Vec<f32>
    DriverError(Box<dyn std::error::Error + Send + Sync>), // driver-specific
}
```

#### Exit code mapping (tool layer)

| `DbError` variant   | Exit code |
|---------------------|-----------|
| `NotFound`          | 1         |
| `PermissionDenied`  | 2         |
| `DialectMismatch`   | 3         |
| `Unsupported`       | 4         |
| `ConnectionFailed`  | 5         |
| `InvalidFilter`     | 6         |
| `InvalidEmbedding`  | 7         |
| `DriverError`       | 127       |

---

## Pipeline execution

### The problem

Unix pipes are inherently streaming — each stage runs independently and passes
stdout to the next. But naively executing database-backed tools this way is
wasteful:

```
find /db/tables/users | filter 'age > 21' | head -20
```

**Naive execution:** fetch all matching rows, take 20.  
**Optimized:** single query with `WHERE age > 21 LIMIT 20`.

The goal is to keep the Unix composability while pushing as much work as
possible into the database. This is the same problem that Polars and Spark
solve with lazy evaluation — build a logical plan, optimize, then execute.

### `Pipeline` and `PipeStage`

Session parses a full pipe chain into a `Pipeline` before executing anything.

```rust
pub struct Pipeline {
    pub stages: Vec<PipeStage>,
}

pub struct PipeStage {
    pub tool: ToolCall,
    pub pushdown: PushdownCapability,
}

/// Declares what a tool stage *could* contribute to a server-side query
/// if folded into the lead stage. Set during parsing, consumed by the optimizer.
pub enum PushdownCapability {
    /// Cannot be pushed down — always runs client-side (e.g. wc)
    None,
    /// Can become LIMIT (head -n)
    Limit { count: u64 },
    /// Can become OFFSET (tail -n +N)
    Offset { count: u64 },
    /// Can become WHERE LIKE (grep pattern → text match)
    GrepFilter { pattern: String },
    /// Can become WHERE clause (filter 'field > value' → comparison)
    FieldFilter(Filter),
}
```

### `PipelineOptimizer`

Walks the pipeline front-to-back and folds pushdown-eligible stages into the
lead stage's `DbOperation`. Stops folding at the first non-pushable stage
(the **materialization boundary**).

```rust
pub struct PipelineOptimizer;

impl PipelineOptimizer {
    /// Takes a parsed Pipeline, returns an optimized ExecutionPlan.
    pub fn optimize(pipeline: Pipeline) -> ExecutionPlan {
        // 1. Identify the lead stage (must produce a DbOperation)
        // 2. Walk subsequent stages:
        //    - If pushdown-eligible AND the driver supports it: fold into lead op
        //    - Otherwise: mark as materialization boundary, stop folding
        // 3. Everything after the boundary runs client-side over stdout
        // TODO
    }
}
```

### `ExecutionPlan`

The output of optimization. Splits the pipeline into a server-side query
and a client-side tail.

```rust
pub struct ExecutionPlan {
    /// The (potentially enriched) database operation to execute server-side.
    pub server_op: DbOperation,
    /// Remaining stages that run client-side over the materialized stdout.
    /// Empty if the entire pipeline was pushed down.
    pub client_stages: Vec<PipeStage>,
}
```

### Pushdown rules

Which tools can push down, and what they fold into:

| Tool    | Flag/usage      | Pushes down to         | Requires driver support |
|---------|-----------------|------------------------|------------------------|
| `head`  | `-n 20`         | `LIMIT 20`             | All drivers            |
| `tail`  | `-n +101`       | `OFFSET 100`           | Relational             |
| `grep`  | `pattern`       | `WHERE field LIKE '%pattern%'` | Relational       |
| `filter` | `field > value` | `WHERE field > value` | Relational             |

Tools that **never** push down (always client-side):

| Tool | Reason |
|------|--------|
| `wc` | Aggregation over stdout — needs materialized data |
| `sort` | Reordering over stdout — needs materialized data |
| `awk` | Arbitrary transformation — can't express as query |
| `uniq` | Requires sorted input, operates on text lines |
| `tee` | Side-effect (writes to file), pass-through |

### Materialization boundaries

When the optimizer hits a stage it can't push down, everything before it
(including the folded stages) executes as a single server-side query. The
result materializes to stdout, and remaining stages run client-side.

```
find /db/tables/users | filter '...' | head -20 | wc -l
                                       ^^^^^^^   ^^^^^^
                                       pushed down    boundary — can't push wc
```

Execution plan:
1. **Server:** `SELECT * FROM users WHERE ... LIMIT 20` (head folded in)
2. **Client:** `wc -l` runs over stdout

### Driver pushdown support

Not all drivers support all pushdown operations. The optimizer must check
driver capabilities before folding.

```rust
pub trait PushdownSupport {
    fn supports_order_by(&self) -> bool;
    fn supports_limit(&self) -> bool;
    fn supports_offset(&self) -> bool;
    fn supports_projection(&self) -> bool;
    fn supports_like_filter(&self) -> bool;
}
```

If a driver doesn't support a pushdown (e.g. Qdrant doesn't support ORDER BY),
the stage stays client-side even if it's before any other boundary.

> **Open: Pipeline optimization**
>
> - Should the optimizer be greedy (fold everything it can) or cost-based (estimate whether pushdown is actually faster)?
> - How does pushdown interact with `grep` on vector/graph backends? Text pattern matching may not map cleanly to vector filters.
> - Can the optimizer span multiple `DbOperation`s? e.g. `find /db/tables/users | find /db/tables/orders` — is this a merge, or an error?
> - Should the `ExecutionPlan` be inspectable? An `explain` command (like SQL EXPLAIN) that shows what pushed down and what didn't would be valuable for debugging.
> - Where does this live in the crate structure? `dbshell-core` (alongside Session) or a new `dbshell-pipeline` crate?

### Error propagation

A failing stage aborts the entire pipeline immediately. The failing stage's
error becomes the pipeline result — no partial output, no silent swallowing.
This follows the Unix `set -e` / `set -o pipefail` model.

#### Rules

1. **Server-side failure** (the lead stage's `DbOperation` fails): the
   pipeline returns immediately with the `DbError` as the `ToolResult`.
   No client stages run. stderr contains the error message, exit code is
   set per the exit code mapping table.

2. **Client-side failure** (a post-materialization stage fails): the
   pipeline aborts at the failing stage. stdout contains whatever the
   previous stages produced up to that point, but the `ToolResult`'s
   exit code and stderr reflect the failing stage. No subsequent stages
   run.

3. **Transaction interaction**: if a pipeline fails inside a
   `begin`/`commit` block, the transaction remains open — it is **not**
   auto-rolled-back. The agent (or user) must explicitly `rollback` or
   `commit`. This matches SQL behavior where a failed statement doesn't
   end the transaction.

#### `ToolResult` on failure

```rust
// Pipeline: find /db/tables/users | filter 'age >' | head -20
// Result: parse fails at the filter stage
ToolResult {
    stdout: "",
    stderr: "filter: parse error: expected value after '>'",
    exit_code: 6,           // InvalidFilter
    payload: ToolPayload::Empty,
}

// Pipeline: find /db/tables/nonexistent | head -20
// Result: table not found
ToolResult {
    stdout: "",
    stderr: "find: table 'nonexistent' not found",
    exit_code: 1,
    payload: ToolPayload::Empty,
}
```

#### `CommandLine` error propagation

For `;`-separated sequential commands, a failing pipeline **does not**
abort subsequent commands by default. Each pipeline is independent:

```
find /db/tables/nonexistent; find /db/tables/users
#     ^^^ fails (exit 1)     ^^^ still runs
```

`Session.exec()` returns a `Vec<ToolResult>` — one per pipeline. The caller
can inspect exit codes to determine overall success.

For `&`-separated parallel commands, all pipelines run to completion (or
failure) independently. A failure in one does not cancel the others.

---

## Input parsing

Session receives raw input strings. Before pipeline optimization, the input
is parsed into a `CommandLine` — a sequence of pipelines separated by control
operators.

### `CommandLine`

```rust
pub struct CommandLine {
    pub groups: Vec<CommandGroup>,
}

pub struct CommandGroup {
    pub pipeline: Pipeline,
    pub separator: Separator,
}

pub enum Separator {
    /// `;` or newline — wait for this pipeline to finish, then run the next.
    Sequential,
    /// `&` — run this pipeline in the background, continue immediately.
    Background,
    /// End of input.
    End,
}
```

### Control operators

| Operator | Meaning | Example |
|----------|---------|---------|
| `\|` | Pipe — connect stdout to next stage's stdin | `find ... \| grep Alice` |
| `;` | Sequential — run next after current completes | `echo '...' >> t1; echo '...' >> t2` |
| `&` | Background — run current in parallel, continue | `find /db/tables/users & find /db/tables/orders` |
| `begin` | Open transaction block | `begin` |
| `commit` | Commit active transaction | `commit` |
| `rollback` | Rollback active transaction | `rollback` |

### Parsing rules

- `|` binds tighter than `;` and `&` — `a | b ; c` is `(a | b) ; (c)`.
- `&` applies to the preceding pipeline, not individual commands.
- `begin`, `commit`, `rollback` are standalone statements — they cannot
  appear mid-pipeline or be piped.
- Redirect operators `>` and `>>` are parsed as part of the final stage
  in a pipeline, not as separate stages. They set the write mode on the
  pipeline's terminal operation.

### Examples

```
# Sequential: insert then query
echo '{"name":"Alice"}' >> /db/tables/users; find /db/tables/users | grep Alice

# Parallel: two independent reads
find /db/tables/users/by_status/active & find /db/tables/orders/by_status/open

# Transaction: atomic multi-table write
begin
echo '{"id":1,"user_id":5,"total":99.00}' >> /db/tables/orders
echo '{"id":2,"user_id":5,"total":50.00}' >> /db/tables/orders
commit

# Mixed: parallel reads, then sequential write
find /db/tables/users & find /db/tables/products; echo '{"report":"done"}' >> /db/tables/logs
```

---

## Transactions and concurrency

### Transactions

Transactions use explicit `begin`/`commit`/`rollback` statements, modeled
after the standard SQL transaction lifecycle. No special paths or magic files.

#### Lifecycle

```
begin                                              # Session.begin()
  echo '{"id":1}' >> /db/tables/orders             # INSERT — uses active tx
  echo '{"id":2}' >> /db/tables/orders             # INSERT — uses active tx
  find /db/tables/orders | filter 'id == 1'        # READ — sees uncommitted writes (driver-dependent)
commit                                             # Session.commit()
```

- `begin` acquires a `DriverTransaction` from the underlying driver and
  stores it in `Session.tx`. All subsequent operations dispatch through
  this handle instead of the connection pool.
- `commit` commits and clears `Session.tx`.
- `rollback` rolls back and clears `Session.tx`.
- Issuing `begin` while a transaction is already active is an error.
- If the session drops with an active transaction, the transaction is
  **rolled back** (fail-safe — never silently commit).

#### `DriverTransaction`

Drivers that support transactions implement this trait. The trait is object-safe
so Session can hold it as `dyn DriverTransaction`.

```rust
#[async_trait]
pub trait DriverTransaction: Send + Sync {
    /// Execute an operation within this transaction.
    async fn execute(&self, op: &DbOperation) -> Result<ToolPayload>;

    async fn commit(self: Box<Self>) -> Result<()>;
    async fn rollback(self: Box<Self>) -> Result<()>;
}
```

#### Driver support

Not all drivers support transactions. When `begin` is issued against a
driver that doesn't support them, Session returns `DbError::Unsupported`.

| Driver | Transaction support | Isolation level |
|--------|-------------------|-----------------|
| PgDriver | Full | READ COMMITTED (Postgres default) |
| SurrealDriver | Limited | Driver-dependent |
| QdrantDriver | None | N/A — `begin` returns error |

For pipelines targeting a non-transactional driver, writes are auto-committed
individually. The user sees a warning on stderr: `warn: driver "qdrant" does
not support transactions — writes are auto-committed`.

#### Read visibility within transactions

Whether reads see uncommitted writes from the same transaction is
**driver-dependent** and not guaranteed by dbshell:

- **Postgres**: yes — reads within a transaction see prior writes in that
  transaction (READ COMMITTED).
- **Qdrant**: N/A — no transactions.
- **SurrealDB**: depends on isolation mode.

The design does not abstract over this — each driver behaves according to
its native isolation semantics. The `describe_table` / `InspectCollection`
output can note the driver's isolation level so agents can reason about it.

### Concurrency

#### Default: sequential execution

Like a Unix shell, pipelines execute sequentially by default. `;` separates
sequential commands. No locks are needed on `ResultStore` or the cache
in the single-pipeline case.

```
find /db/tables/users; find /db/tables/orders
# users completes, then orders runs
```

#### Explicit parallelism with `&`

`&` runs the preceding pipeline in the background. Session spawns a
`tokio::spawn` task for each `&`-separated pipeline and joins them at
the next `;` or end of input.

```
find /db/tables/users & find /db/tables/orders
# both run concurrently, session waits for both to complete
```

#### Concurrency on VFS state

| VFS region | Protection | Behavior |
|---|---|---|
| Path resolution (`VirtualFS`) | None needed | Pure function — no mutable state |
| `/results/<uuid>` | None needed | Write-once with unique IDs |
| `/results/last` | `RwLock<ResultStore>` | Last-writer-wins — parallel pipelines each write their own `<uuid>`, `last` points to whichever finishes last |
| Cache (`CacheLayer`) | `RwLock<Cache>` | Reads take shared lock, mutations take exclusive lock and invalidate affected entries |
| Transaction (`Session.tx`) | `Mutex<Option<DriverTransaction>>` | Only one pipeline may hold the tx at a time — see below |

#### Transactions and parallelism

`&` inside a `begin`/`commit` block is an error. Parallel pipelines sharing
a single database transaction would require serializable isolation and
careful ordering — complexity that isn't worth it for v1. The parser rejects
this at parse time:

```
begin
  echo '...' >> /db/tables/orders & echo '...' >> /db/tables/products   # ERROR: & not allowed in transaction
commit
```

If you need concurrent writes, commit first, then parallelize:

```
commit; echo '...' >> /db/tables/orders & echo '...' >> /db/tables/products
```

> **Open: Transactions and concurrency**
>
> - Should `&` pipelines share a single `ResultStore` namespace, or should each get an isolated result space that merges on completion?
> - Should there be a `wait` builtin to block until all background pipelines complete (like bash `wait`)?
> - Is there a limit on the number of concurrent `&` pipelines? (Bounded by driver connection pool size?)
> - Should savepoints be supported within a transaction? (`savepoint sp1` / `rollback sp1`) — useful for partial rollback on error.

---

## Tool interface

All tools are async functions in `dbshell-tools` with a consistent signature.

```rust
pub struct ToolContext<'a> {
    pub session: &'a Session,
    pub cwd: &'a VfsPath,
    pub env: &'a HashMap<String, String>,
    pub stdin: Option<&'a str>,
}

pub struct ToolCall {
    pub name: String,
    pub path: VfsPath,
    pub args: ToolArgs,
    pub stdin: Option<String>,
}
```

### Built-in commands

These are handled by Session directly, not dispatched to drivers.

| Command | Behavior |
|---------|----------|
| `man <tool>` | Full manual page — description, all flags, examples, filter syntax. Designed for agents to learn the CLI. |
| `<tool> --help` | Short usage summary — one line per flag. |
| `begin` | Open transaction (see Transactions section) |
| `commit` | Commit active transaction |
| `rollback` | Rollback active transaction |

`man` pages are embedded in the binary as static strings — one per tool.
They include worked examples with realistic data so agents can learn by
pattern matching. `--help` is a short form derived from the same source.

Both `man` and `--help` write to stdout and can be piped, though there's
no practical reason to do so.

### Planned tools (v1)

| Tool   | Ops it maps to                                     |
|--------|----------------------------------------------------|
| `ls`   | `ListCollections`, `ListTables`, `InspectCollection` |
| `cat`  | `InspectCollection`, `DescribeTable`, `ReadResult`  |
| `find` | `VectorSearch`, `GraphQuery`, `QueryTable`, `Delete`/`DeleteRows` (with `-delete`) |
| `grep` | text pattern match over stdout (pushes down as LIKE) |
| `filter` | field-level predicates (pushes down as WHERE clause) |
| `wc`   | count over `find` stdout                           |
| `sort` | order results (client-side, operates on stdout)    |
| `head` | limit results (`-n`)                               |
| `tail` | offset results (`-n +N`)                           |
| `merge` | `MergeTable`                                      |
| `echo … >>` | `InsertRows` (relational), `Upsert` (vector) |
| `echo … >`  | `UpsertRows` (PK match, full row replace)     |
| `ln`   | VFS-local — creates session-scoped symlink          |
| `rm`   | `DropCollection`, or remove symlink if `/links/`   |

### Output formatting

All tools that return data write **JSON lines** to stdout — one JSON object
per line. This is the universal interchange format across pipes.

```
$ find /db/tables/users/by_status/active | head -3
{"id":1,"name":"Alice","age":30,"active":true}
{"id":3,"name":"Carol","age":25,"active":true}
{"id":7,"name":"Grace","age":42,"active":true}
```

- **`find`, `merge`, `grep`**: JSON lines (one row per line)
- **`cat` on entity**: JSON object (schema, stats, constraints)
- **`ls`**: newline-separated names (like real `ls`)
- **`wc`**: plain integer
- **`echo`**: confirmation message (`inserted 1 row`)
- **`sort`**: passes through JSON lines, reordered

JSON lines is chosen because:
- Streamable — each line is independently parseable
- Composable — `grep`, `sort`, `wc -l` all work naturally
- Agent-friendly — no custom parsing needed
- Consistent with the write path (`>>` and `>` accept JSON lines)

### `find` flags

All flags mirror real Unix `find`. No custom flags.

| Flag        | Meaning                                              |
|-------------|------------------------------------------------------|
| `-delete`   | execute `Delete`/`DeleteRows` on matches             |
| `-exec`     | run a tool on results (see below)                    |

**Replaced by Unix composition** — these operations use pipes instead of
custom flags:

| Instead of        | Use                                          |
|-------------------|----------------------------------------------|
| ~~`-dry-run`~~    | `find ... \| wc -l`                          |
| ~~`-offset`~~     | `find ... \| tail -n +101`                   |
| ~~`-label`~~      | path structure: `find /db/graphs/nodes/Artist` |

**Pagination** is handled by `head` and `tail` in pipes:

```
find /db/tables/users | head -20                          # first 20
find /db/tables/users | tail -n +101 | head -20           # rows 101-120 (skip 100)
```

#### `-exec`

Runs a dbshell tool inline on the results. Modeled after `find -exec` in
Unix — familiar to any user or agent that knows the standard toolchain.

```
find /db/tables/users -exec grep Alice \;
find /db/tables/users -exec filter 'age > 21' \;
find /db/tables/orders -exec grep shipped \; | sort -r
```

`-exec` is a dbshell builtin, not a shell-out. The optimizer has full
visibility into the tool and its arguments and can push recognized patterns
down to the driver as server-side WHERE clauses.

### `merge` flags

`merge` is a dbshell command (not a Unix tool) for combining two tables.
Takes two table paths as positional args.

```
merge --on user_id=id /db/tables/orders /db/tables/users
merge --on id /db/tables/orders /db/tables/users                # same field name in both
merge --left --on user_id=id /db/tables/orders /db/tables/users  # LEFT join
merge --on product_id=id - /db/tables/products                   # stdin as left side
```

| Flag              | Meaning                                              |
|-------------------|------------------------------------------------------|
| `--on FIELD`      | Join on FIELD (same name in both tables)             |
| `--on L=R`        | Join on field L from left table, R from right table  |
| `--left`          | Left join (include unmatched rows from left)         |
| `--right`         | Right join (include unmatched rows from right)       |
| `--full`          | Full outer join (include all unmatched rows)         |
| `--anti-left`     | Only unmatched rows from left table                  |
| `--anti-right`    | Only unmatched rows from right table                 |
| `--fields F1,F2`  | Output projection — comma-separated field list       |

**Merge types:**

| SQL equivalent | `merge` flag |
|---|---|
| `INNER JOIN` | default (no type flag) |
| `LEFT JOIN` | `--left` |
| `RIGHT JOIN` | `--right` |
| `FULL OUTER JOIN` | `--full` |

**Projection** uses `--fields`:

```
merge --on user_id=id --fields total,name /db/tables/orders /db/tables/users
```

Pre-merge filtering is done via pipes — filter before merging:

```
find /db/tables/orders/by_status/shipped | merge --on user_id=id - /db/tables/users
```

**Multi-way merges** chain through pipes. `-` (or stdin) as the left table
means "use the result of the previous stage":

```
merge --on user_id=id /db/tables/orders /db/tables/users | merge --on product_id=id - /db/tables/products
```

The pipeline optimizer can combine adjacent `MergeTable` operations into a
single multi-table SQL query when both target the same driver.

**Pipeline interaction:** `merge` is always a lead stage (it produces a
`DbOperation`), never a pushdown fold. When `merge` appears mid-pipeline
with `-` as left side, it consumes materialized stdin and initiates a new
server-side operation — this is a **materialization boundary** followed by
a new lead stage.

> **Open: Merges**
>
> - Should multi-column merge conditions be supported? e.g. `--on user_id=id,org_id=org_id`
> - For chained merges with `-`, should the optimizer attempt to combine them into one query (requires same driver), or always materialize between stages?
> - How does `merge` interact with non-relational drivers? Graph traversal already expresses joins implicitly via edges. Should `merge` on graph paths be an error or silently delegate to graph traversal?
> - What is the column naming strategy when both tables have a column with the same name? Prefix with table name (`users.name`, `orders.name`)? Or require `--fields` to disambiguate?

### Write path

All writes use JSON lines (one JSON object per line) as the interchange format.
This is streamable, works with pipes, and matches how `find` outputs results.

#### Redirect semantics

| Syntax | Operation | Behavior |
|--------|-----------|----------|
| `echo '{"name":"Alice"}' >> /db/tables/users` | INSERT | Append — always creates new rows |
| `echo '{"id":1,"name":"Alice"}' > /db/tables/users` | UPSERT | Match on primary key, full row replace |
| `cat bulk.jsonl >> /db/tables/users` | BULK INSERT | Streams JSON lines, one INSERT per line |
| `cat bulk.jsonl > /db/tables/users` | BULK UPSERT | Streams JSON lines, one UPSERT per line |

**`>` (overwrite) = upsert, not truncate.** This matches the expectation that
writing to a file replaces content at a specific location. The primary key
is resolved from `CollectionInfo` metadata (cached in-memory by Session after
first `cat` or `describe_table`). If the input JSON is missing the PK column,
the driver treats it as an INSERT (auto-generated key if the schema supports it).

**`>>` (append) = insert, always.** Never matches existing rows. Constraint
violations (duplicate PK, NOT NULL, FK violations) are reported via stderr
on the `ToolResult` — the tool exits non-zero but includes the count of
successfully inserted rows in stdout.

#### Vector writes and auto-embedding

Writing to a vector collection follows the same redirect semantics, but the
driver auto-embeds text fields before storage.

```
echo '{"text": "blue suede shoes", "genre": "rock"}' >> /db/vectors/tracks
```

The driver:
1. Receives the JSON record
2. Calls the configured `Embedder` to vectorize the text field
3. Stores both the embedding and the original text as payload

The original text is always persisted alongside the vector so that it can be
returned in search results and re-embedded if the model changes.

Which field to embed is determined by the collection's schema — either a
convention (`text` field) or an explicit config on the collection spec.

#### `Embedder`

A pluggable trait for generating embeddings. Configured per-session or
per-driver.

```rust
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embed a single text input. Returns a vector of f32.
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Batch embed for bulk writes. Default impl calls embed() in a loop.
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }

    fn dimensions(&self) -> u32;
    fn model_name(&self) -> &str;
}
```

Planned implementations:

| Struct | Backend | Notes |
|--------|---------|-------|
| `OpenAiEmbedder` | OpenAI API (`text-embedding-3-small`, etc.) | Requires API key in session env |
| `SentenceTransformerEmbedder` | Local ONNX / candle runtime | No network, slower first load |

The `Embedder` is stored on `Session` and made available to drivers via
`ToolContext`. Vector drivers call it during upsert — the tool layer does
not pre-embed.

> **Open: Write path**
>
> - For upsert (`>`), if the table has a composite primary key, does the input JSON need all PK columns present? Or can it match on a subset?
> - How should row updates (UPDATE) be expressed? The write path currently covers INSERT (`>>`) and UPSERT (`>`), but partial field updates need a tool. Options: a dedicated `update` command, piping filtered results through a transform, or `raw()`.
> - For vector writes, what happens if the collection's configured dimensions don't match the embedder's output dimensions? Error at write time, or at collection creation time?
> - Should `Embedder` be configurable per-collection (different models for different collections) or strictly per-session?
> - How is the "which field to embed" convention established? Collection-level config in `CollectionSpec`? A reserved field name? An explicit flag on `echo`?

---

## Driver implementation notes

### Connection handling

Each `DbDriver` impl owns its own connection state. There is no unified
connection struct — driver connection semantics vary too widely (connection
pools, WebSocket channels, gRPC channels) to standardize. The `DbDriver`
trait is the abstraction boundary; everything below it is driver-internal.

```rust
pub struct PgDriver {
    pool: sqlx::PgPool,          // connection pool, preconfigured
    capabilities: Vec<DbCapability>,
}

pub struct QdrantDriver {
    client: qdrant_client::Qdrant,  // gRPC channel
}

pub struct SurrealDriver {
    client: surrealdb::Surreal<surrealdb::engine::any::Any>,  // WebSocket or HTTP
}
```

`ConnectOptions` provides the connection string and TLS config at Session
construction time. Each driver's `::connect(opts: &ConnectOptions)` factory
is responsible for parsing the connection string, establishing the connection,
and returning a ready `Box<dyn DbDriver>`. Connection-specific options
(pgvector extensions, AGE graph names, Qdrant API keys) are encoded in the
connection string or as query parameters — no driver-specific flags leak
into the shared API.

```rust
impl PgDriver {
    pub async fn connect(opts: &ConnectOptions) -> Result<Self> {
        // Parses connection_string as postgres:// URL
        // Enables pgvector extension if ?pgvector=true
        // Loads AGE extension if ?age=graph_name
        // TODO
    }
}
```

### PgDriver (pgvector + AGE)

- **Connection pooling:** sqlx `PgPool` with `max_connections` from `ConnectOptions`
- **pgvector:** `<->` `<#>` `<=>` operators, `vector(N)` column type
- **AGE:** requires `LOAD 'age'` + `SET search_path` on each connection
- **agtype deserialization:** close to JSON, thin deserializer (~200 loc)
- **DbType:** `Vector` (pgvector only) or `Hybrid` (pgvector + AGE)
- **Transactions:** native Postgres transactions via sqlx, full `DriverTransaction` support

### QdrantDriver

- **Client:** official `qdrant-client` crate (gRPC, async-native)
- **Filter:** maps `Filter` to `FieldCondition` / Qdrant `Filter` structs
- **Named vectors and sparse vectors:** planned support, not v1
- **Transactions:** none — `begin` returns `DbError::Unsupported`

### SurrealDriver

- **Client:** official `surrealdb` crate (WebSocket + HTTP)
- **DbType:** `Hybrid` — supports relational, graph, and vector
- **Graph queries:** maps SurrealQL graph traversal (`->` `<-` `<->` operators). `GraphQuery::SurrealQL` only — no Cypher support
- **Vector search:** HNSW via SurrealDB's built-in index
- **Transactions:** limited support via SurrealDB's transaction API

---

## Open decisions

| # | Question | Options | Decision |
|---|----------|---------|----------|
| 1 | Result serialization format | Arrow IPC vs native Rust structs vs JSON | TODO |
| 2 | Python async bridge | pyo3-async-runtimes vs sync wrapper | TODO |
| 3 | Filtering approach | Views (directory structure) + Unix composition (grep, pipes) + `filter` command for field predicates | Decided |
| 4 | VFS schema auto-discovery | Auto on mount vs explicit declaration | TODO |
| 5 | Persistent cache backend | Redis vs sqlite vs none for v1 | TODO |
| 6 | `/results/` eviction policy | Session-scoped LRU vs TTL | TODO |
| 7 | Pagination | How does cursor-based pagination work across tool calls? `head`/`tail` push LIMIT/OFFSET to the driver, but multi-page browsing needs a cursor mechanism. | TODO |
| 8 | Sorting | `sort` runs client-side over stdout. Should there be a way to push ORDER BY to the driver, or is client-side sorting sufficient for v1? | TODO |
| 9 | Relational merge support | Dedicated `merge` tool with `MergeRequest` and `MergeTable` DbOperation | Decided |
| 10 | Schema change detection | Invalidate on TTL vs event-driven vs manual refresh | TODO |
| 11 | Pipe execution model | Lazy pipeline with pushdown optimization (see Pipeline section) | TODO — greedy vs cost-based optimizer |
| 12 | Write stdin format | JSON lines (`>>` = INSERT, `>` = UPSERT). Update mechanism TBD. | TODO |
| 13 | MCP tool surface | Single `dbshell_exec` tool, streamable HTTP transport | Decided (details deferred) |
| 14 | Testing strategy | MemoryDriver (unit) + docker-compose (integration), both required in CI | Decided |
| 15 | Transaction semantics | `begin`/`commit`/`rollback` statements, `DriverTransaction` trait, rollback on drop | Decided |
| 16 | Concurrency model | Sequential default, explicit `&` for parallelism, `&` forbidden inside transactions | Decided |

---

## MCP server design (`dbshell-mcp`)

> **Status:** Deferred — will be built last, after core and tools are stable.

Thin HTTP layer over `Session`. Exposes a single `exec` tool that accepts
raw dbshell command strings. The agent composes filesystem commands exactly
as documented — no separate MCP tool per Unix command.

### Transport

Streamable HTTP (HTTP + SSE). The server accepts JSON-RPC requests over
HTTP POST and streams responses via server-sent events. This is the MCP
Streamable HTTP transport — no stdio, no WebSocket.

### Tool surface

One tool: `dbshell_exec`.

```json
{
  "name": "dbshell_exec",
  "description": "Execute a dbshell command against the connected databases. Supports the full command syntax: pipes (|), sequential (;), parallel (&), transactions (begin/commit/rollback), and redirects (>, >>).",
  "inputSchema": {
    "type": "object",
    "properties": {
      "command": {
        "type": "string",
        "description": "The dbshell command to execute, e.g. 'find /db/tables/users/by_status/active | grep Alice | head -20'"
      }
    },
    "required": ["command"]
  }
}
```

The tool calls `Session.exec(command)` and returns the `ToolResult`(s) as
the MCP tool response. For long-running commands, progress is streamed
via SSE notifications.

### Session lifecycle

One `Session` per MCP connection. The connection's `initialize` handshake
creates a `Session` with the configured `ConnectOptions`. The session lives
for the duration of the connection — transaction state, result history, and
cache persist across tool calls within the same connection.

### Configuration

Connection credentials are passed via MCP server configuration (environment
variables or a config file read at startup). The MCP server does not accept
credentials in tool call arguments.

```json
{
  "mcpServers": {
    "dbshell": {
      "url": "http://localhost:3001/mcp",
      "env": {
        "DBSHELL_PG_URL": "postgres://user:pass@host/db?pgvector=true",
        "DBSHELL_QDRANT_URL": "http://localhost:6334",
        "DBSHELL_MODE": "read_write"
      }
    }
  }
}
```

> **Open: MCP design (deferred)**
>
> - Should MCP resources expose the VFS tree (e.g. `/db/tables/` as a listable resource) for agent discovery, or is `ls` via `dbshell_exec` sufficient?
> - Should there be a second tool for structured output (returning `ResultSet` as JSON) vs the text-formatted `stdout`?
> - How should authentication work for multi-tenant deployments?
> - Rate limiting and connection pooling for the HTTP server.

---

## Observability

Uses the `tracing` crate throughout — the de facto standard for async Rust.
All spans and events use structured fields so they can be filtered, exported
to JSON, or consumed by OpenTelemetry collectors.

### Why observability matters here

dbshell sits between an AI agent and a database. When the agent produces
unexpected results, the first question is always: **what queries did it
actually run?** Tracing every operation that flows through the VFS makes it
possible to distinguish between a hallucinating agent (asked for data that
doesn't exist) and a correct agent hitting a driver bug.

### Span hierarchy

```
session.exec                          # top-level input line
  ├─ parse                            # CommandLine parsing
  ├─ pipeline.exec                    # one per pipeline in the CommandLine
  │   ├─ pipeline.optimize            # PipelineOptimizer pass
  │   ├─ vfs.resolve                  # VfsPath → DbOperation (pure, fast)
  │   ├─ session.validate             # SessionMode check
  │   ├─ cache.lookup                 # CacheKey hit/miss
  │   ├─ driver.execute               # actual DB call
  │   │   ├─ driver.query_sql         # logged SQL / Cypher / SurrealQL
  │   │   └─ driver.embed             # Embedder call (vector writes)
  │   ├─ client_stage.exec            # per client-side stage (sort, wc, etc.)
  │   └─ results.write                # write to ResultStore
  └─ transaction.commit / .rollback   # if applicable
```

### Structured fields on every span

| Field | Type | Example | Purpose |
|-------|------|---------|---------|
| `driver` | string | `"pg"` | Which driver handled the op |
| `operation` | string | `"QueryTable"` | DbOperation variant name |
| `collection` | string | `"users"` | Target entity |
| `cache_hit` | bool | `true` | Was the result served from cache |
| `duration_ms` | u64 | `12` | Wall-clock time |
| `rows_affected` | u64 | `3` | For writes — count of affected rows |
| `rows_returned` | u64 | `50` | For reads — count of returned rows |
| `query_text` | string | `"SELECT * FROM users WHERE ..."` | The actual query sent to the DB |
| `pushdown` | string | `"ORDER BY name LIMIT 20"` | What the optimizer folded in |

### Query logging

Every query dispatched to a driver is logged at `DEBUG` level with the full
query text. This is the single most useful signal for debugging agent behavior.

```rust
#[instrument(skip(self), fields(driver = self.name(), operation = %op, collection))]
async fn dispatch(&self, op: &DbOperation) -> Result<ToolPayload> {
    tracing::debug!(query_text = %sql, "executing query");
    // ...
}
```

At `INFO` level, queries are logged without the full text (operation +
collection + duration only). At `TRACE` level, full request/response
payloads are logged.

### Agent audit trail

For AI agent use cases, the span tree forms an **audit trail** — every
action the agent took, what it asked the database, and what it got back.
This can be exported as JSON for post-hoc analysis:

```json
{"span":"driver.execute","driver":"pg","operation":"QueryTable","collection":"users","query_text":"SELECT * FROM users WHERE age > 21 ORDER BY name LIMIT 20","cache_hit":false,"duration_ms":12,"rows_returned":20}
```

This is essential for:
- **Hallucination detection** — the agent claims data exists, but the query
  returned zero rows
- **Performance debugging** — identifying slow queries or cache misses
- **Security auditing** — what data the agent accessed

> **Open: Observability**
>
> - Should `query_text` logging be opt-in (off by default) to avoid leaking sensitive data in production? Or always-on with a redaction layer?
> - Are metrics (query latency histograms, cache hit rate, connection pool utilization) in scope for v1, or deferred?
> - How does observability surface in the Python bindings — Python `logging` integration via `pyo3-log`, or opaque?
> - Should there be a built-in `history` command that replays the span tree for the current session?

---

## Testing strategy

Two tiers are required: unit tests with `MemoryDriver` and integration tests
with real databases. Both must pass in CI before merge.

### Tier 1: `MemoryDriver` (unit tests)

`MemoryDriver` is a full `DbDriver` implementation backed by in-memory data
structures. It supports all operations including transactions, and is
deterministic (no network, no I/O, no timing variance).

```rust
pub struct MemoryDriver {
    collections: RwLock<HashMap<String, MemoryCollection>>,
    tables: RwLock<HashMap<String, MemoryTable>>,
}

impl DbDriver for MemoryDriver { /* ... */ }
impl MemoryDriver {
    /// Seed the driver with test data.
    pub fn with_table(self, name: &str, schema: TableSchema, rows: Vec<serde_json::Value>) -> Self;
    pub fn with_collection(self, name: &str, records: Vec<Record>) -> Self;
}
```

**What it tests:**
- VirtualFS path resolution and all VfsPathKind variants
- Pipeline parsing, optimization, and execution plans
- `CommandLine` parsing (`;`, `&`, `begin`/`commit`/`rollback`)
- Session lifecycle (mode checks, result storage, cache behavior)
- QueryRouter dispatch logic
- Tool implementations (all flags, all redirect modes)
- `PipelineOptimizer` pushdown rules
- Transaction semantics (begin/commit/rollback/drop-rollback)
- Write path (`>>`, `>`, `find -delete`)
- Error paths (constraint violations, FK violations, type mismatches)

**What it doesn't test:**
- Real SQL/Cypher/SurrealQL generation
- Connection handling, pooling, TLS
- Driver-specific behavior (pgvector operators, AGE graph traversal)
- Actual embedder API calls

### Tier 2: Integration tests (real databases)

Docker Compose with real Postgres (pgvector + AGE), Qdrant, and SurrealDB.
Each driver gets a dedicated test suite that runs the same test cases against
a live database.

```yaml
# docker-compose.test.yml
services:
  postgres:
    image: pgvector/pgvector:pg16
    # AGE extension installed via init script
  qdrant:
    image: qdrant/qdrant:latest
  surrealdb:
    image: surrealdb/surrealdb:latest
```

**What it tests:**
- Real query generation and execution (SQL, Cypher, SurrealQL)
- Connection lifecycle (pool creation, health checks, reconnection)
- Driver-specific transaction isolation behavior
- Constraint enforcement by the real database
- Embedder integration (with a mock HTTP server for OpenAI, or a small
  local model for SentenceTransformer)
- End-to-end pipeline execution from raw input string to `ToolResult`

**Test structure:**

```rust
/// Shared trait for driver-agnostic integration tests.
/// Each driver implements this to provide a connected Session.
#[async_trait]
trait DriverTestHarness {
    async fn setup() -> Session;
    async fn teardown(session: Session);
}

/// Common test cases — run against every driver.
async fn test_insert_and_query(session: &Session) { /* ... */ }
async fn test_upsert_replaces_row(session: &Session) { /* ... */ }
async fn test_sed_partial_update(session: &Session) { /* ... */ }
async fn test_find_delete(session: &Session) { /* ... */ }
async fn test_pipeline_pushdown(session: &Session) { /* ... */ }
async fn test_transaction_commit(session: &Session) { /* ... */ }
async fn test_transaction_rollback_on_drop(session: &Session) { /* ... */ }
```

### CI requirements

- **Unit tests (tier 1):** run on every PR, no external dependencies.
- **Integration tests (tier 2):** run on every PR via docker-compose.
  Services start before the test suite and are torn down after.
- **Both tiers must pass before merge.**

### Python binding tests

pytest with `MemoryDriver` for unit tests, same docker-compose for
integration. The Python test suite should mirror the Rust integration
tests to ensure the pyo3 bridge doesn't introduce behavioral differences.

> **Open: Testing**
>
> - Should `MemoryDriver` simulate latency (configurable delay) to catch timing-sensitive bugs?
> - Are benchmark targets in scope for v1? (e.g. "VFS overhead < 5ms vs direct driver call")
> - Should integration tests use `testcontainers-rs` instead of docker-compose for better per-test isolation?

---

## Schema evolution and external changes

> **Status:** Not yet designed

> **Open: Schema evolution**
>
> - The cache stores `CollectionInfo` with aggressive TTL. What happens when the underlying schema changes externally (e.g. a column is added, a collection is re-indexed)?
> - Should there be an explicit `refresh` command to invalidate cached schema? (e.g. `cat --refresh /db/tables/users`)
> - Should `Session` detect schema drift on query errors and auto-invalidate, or surface the error to the agent?
> - For relational tables, `ALTER TABLE` can happen at any time. How stale can `describe_table` results be before they cause problems?

---
