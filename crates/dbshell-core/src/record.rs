use serde::{Deserialize, Serialize};

use crate::db_type::DbType;
use crate::filter::Filter;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub vector: Option<Vec<f32>>,
    pub source_text: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub primary_key: Option<Vec<String>>,
    pub columns: Option<Vec<ColumnInfo>>,
    pub foreign_keys: Option<Vec<ForeignKey>>,
    pub constraints: Option<Vec<String>>,
    pub indexes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyInfo {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    pub driver: String,
    pub row_count: Option<u64>,
    pub schema_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSchema {
    pub table: String,
    pub columns: Vec<ColumnInfo>,
    pub primary_key: Option<Vec<String>>,
    pub indexes: Vec<IndexInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub index_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKey {
    pub column: String,
    pub references_table: String,
    pub references_column: String,
    pub on_delete: Option<String>,
    pub on_update: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TableQuery {
    pub filter: Option<Filter>,
    pub columns: Option<Vec<String>>,
    pub order_by: Option<Vec<OrderBy>>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderBy {
    pub column: String,
    pub descending: bool,
}
