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

impl DbOperation {
    /// Whether this operation mutates data (writes, deletes, creates, drops).
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            DbOperation::Upsert { .. }
                | DbOperation::Delete { .. }
                | DbOperation::InsertRows { .. }
                | DbOperation::UpsertRows { .. }
                | DbOperation::UpdateRows { .. }
                | DbOperation::DeleteRows { .. }
                | DbOperation::CreateCollection { .. }
                | DbOperation::DropCollection { .. }
        )
    }

    /// Extract the driver name from the operation, if present.
    pub fn driver_name(&self) -> Option<&str> {
        match self {
            DbOperation::ListCollections { driver }
            | DbOperation::InspectCollection { driver, .. }
            | DbOperation::VectorSearch { driver, .. }
            | DbOperation::Upsert { driver, .. }
            | DbOperation::Delete { driver, .. }
            | DbOperation::GraphQuery { driver, .. }
            | DbOperation::ListTables { driver }
            | DbOperation::DescribeTable { driver, .. }
            | DbOperation::QueryTable { driver, .. }
            | DbOperation::InsertRows { driver, .. }
            | DbOperation::UpsertRows { driver, .. }
            | DbOperation::UpdateRows { driver, .. }
            | DbOperation::DeleteRows { driver, .. }
            | DbOperation::MergeTable { driver, .. }
            | DbOperation::CreateCollection { driver, .. }
            | DbOperation::DropCollection { driver, .. } => Some(driver),
            DbOperation::ReadResult { .. } | DbOperation::ListResults => None,
        }
    }

    /// Extract the collection or table name targeted by this operation.
    pub fn collection_or_table(&self) -> Option<&str> {
        match self {
            DbOperation::InspectCollection { collection, .. }
            | DbOperation::VectorSearch { collection, .. }
            | DbOperation::Upsert { collection, .. }
            | DbOperation::Delete { collection, .. }
            | DbOperation::DropCollection { collection, .. } => Some(collection),
            DbOperation::DescribeTable { table, .. }
            | DbOperation::QueryTable { table, .. }
            | DbOperation::InsertRows { table, .. }
            | DbOperation::UpsertRows { table, .. }
            | DbOperation::UpdateRows { table, .. }
            | DbOperation::DeleteRows { table, .. } => Some(table),
            _ => None,
        }
    }
}
