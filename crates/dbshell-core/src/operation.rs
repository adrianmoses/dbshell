use crate::filter::Filter;
use crate::graph::GraphQuery;
use crate::merge::MergeRequest;
use crate::record::{Record, TableQuery};
use crate::search::{CollectionSpec, VectorSearchRequest};
use crate::vfs_path::VfsPath;

#[derive(Debug)]
pub enum DbOperation {
    // Structural / inspection
    ListCollections {
        driver: String,
    },
    InspectCollection {
        driver: String,
        collection: String,
    },

    // Vector ops
    VectorSearch {
        driver: String,
        collection: String,
        request: VectorSearchRequest,
    },
    Upsert {
        driver: String,
        collection: String,
        records: Vec<Record>,
    },
    Delete {
        driver: String,
        collection: String,
        filter: Filter,
    },

    // Graph ops
    GraphQuery {
        driver: String,
        query: GraphQuery,
    },

    // Relational ops
    ListTables {
        driver: String,
    },
    DescribeTable {
        driver: String,
        table: String,
    },
    QueryTable {
        driver: String,
        table: String,
        request: TableQuery,
    },
    InsertRows {
        driver: String,
        table: String,
        rows: Vec<serde_json::Value>,
    },
    UpsertRows {
        driver: String,
        table: String,
        rows: Vec<serde_json::Value>,
    },
    UpdateRows {
        driver: String,
        table: String,
        filter: Filter,
        set: serde_json::Value,
    },
    DeleteRows {
        driver: String,
        table: String,
        filter: Filter,
    },
    MergeTable {
        driver: String,
        request: MergeRequest,
    },

    // Collection management
    CreateCollection {
        driver: String,
        spec: CollectionSpec,
    },
    DropCollection {
        driver: String,
        collection: String,
    },

    // VFS-local (Session handles without touching QueryRouter)
    ReadResult {
        path: VfsPath,
    },
    ListResults,
}
