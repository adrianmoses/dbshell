use serde::{Deserialize, Serialize};

use crate::filter::Filter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorSearchRequest {
    pub collection: String,
    pub vector: Vec<f32>,
    pub limit: u64,
    pub filter: Option<Filter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredRecord {
    pub id: String,
    pub score: f32,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionSpec {
    pub name: String,
    pub dimensions: u32,
    pub distance_metric: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertResult {
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub healthy: bool,
    pub message: Option<String>,
}
