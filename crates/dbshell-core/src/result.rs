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

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub payload: ToolPayload,
}

impl ToolResult {
    pub fn empty() -> Self {
        ToolResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
            payload: ToolPayload::Empty,
        }
    }

    pub fn from_error(e: &crate::error::DbError) -> Self {
        ToolResult {
            stdout: String::new(),
            stderr: e.to_string(),
            exit_code: e.exit_code(),
            payload: ToolPayload::Empty,
        }
    }
}

impl ResultMetadata {
    pub fn for_query(driver: &str, collection: Option<String>, start: std::time::Instant) -> Self {
        ResultMetadata {
            driver: driver.to_string(),
            collection,
            total_count: None,
            query_ms: start.elapsed().as_millis() as u64,
            cache_hit: false,
            next_cursor: None,
        }
    }
}

#[derive(Debug, Clone)]
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
