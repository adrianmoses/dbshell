use async_trait::async_trait;

use crate::db_type::DbType;
use crate::error::{DbError, Result};
use crate::filter::Filter;
use crate::graph::{GraphQuery, GraphResult};
use crate::merge::MergeRequest;
use crate::record::CollectionInfo;
use crate::record::{Record, TableInfo, TableQuery, TableSchema};
use crate::result::ResultSet;
use crate::search::{
    CollectionSpec, HealthStatus, ScoredRecord, UpsertResult, VectorSearchRequest,
};

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

    // Graph ops
    async fn graph_query(&self, query: &GraphQuery) -> Result<GraphResult> {
        let _ = query;
        Err(DbError::Unsupported("graph queries"))
    }

    // Relational ops
    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        Err(DbError::Unsupported("relational tables"))
    }

    async fn describe_table(&self, name: &str) -> Result<TableSchema> {
        let _ = name;
        Err(DbError::Unsupported("relational tables"))
    }

    async fn query_table(&self, table: &str, req: &TableQuery) -> Result<ResultSet> {
        let _ = (table, req);
        Err(DbError::Unsupported("relational tables"))
    }

    async fn insert_rows(&self, table: &str, rows: Vec<serde_json::Value>) -> Result<u64> {
        let _ = (table, rows);
        Err(DbError::Unsupported("relational tables"))
    }

    async fn upsert_rows(&self, table: &str, rows: Vec<serde_json::Value>) -> Result<u64> {
        let _ = (table, rows);
        Err(DbError::Unsupported("relational tables"))
    }

    async fn update_rows(
        &self,
        table: &str,
        filter: &Filter,
        set: serde_json::Value,
    ) -> Result<u64> {
        let _ = (table, filter, set);
        Err(DbError::Unsupported("relational tables"))
    }

    async fn delete_rows(&self, table: &str, filter: &Filter) -> Result<u64> {
        let _ = (table, filter);
        Err(DbError::Unsupported("relational tables"))
    }

    async fn merge_tables(&self, req: &MergeRequest) -> Result<ResultSet> {
        let _ = req;
        Err(DbError::Unsupported("merge"))
    }

    // Raw escape hatch
    async fn raw(&self, query: &str, params: serde_json::Value) -> Result<serde_json::Value>;
}

#[async_trait]
pub trait DriverTransaction: Send + Sync {
    async fn execute(&self, query: &str, params: serde_json::Value) -> Result<serde_json::Value>;
    async fn commit(self: Box<Self>) -> Result<()>;
    async fn rollback(self: Box<Self>) -> Result<()>;
}
