use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub enum GraphQuery {
    Cypher(String),
    SurrealQL(String),
    Raw(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphResult {
    pub rows: Vec<serde_json::Value>,
}
