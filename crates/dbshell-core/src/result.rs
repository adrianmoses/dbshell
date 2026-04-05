use serde::{Deserialize, Serialize};

use crate::record::CollectionInfo;
use crate::vfs_path::VfsPath;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultSet {
    pub rows: Vec<serde_json::Value>,
    pub schema: Option<CollectionInfo>,
    pub metadata: ResultMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultMetadata {
    pub driver: String,
    pub collection: Option<String>,
    pub total_count: Option<u64>,
    pub query_ms: u64,
    pub cache_hit: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug)]
pub struct ToolResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub payload: ToolPayload,
}

#[derive(Debug)]
pub enum ToolPayload {
    Records(ResultSet),
    Info(CollectionInfo),
    Listing(Vec<CollectionInfo>),
    Written { count: u64 },
    Deleted { count: u64 },
    Created { name: String },
    Dropped { name: String },
    ResultRef(VfsPath),
    Empty,
}
