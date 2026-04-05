use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DbType {
    Vector,
    Graph,
    Relational,
    Hybrid(Vec<DbCapability>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DbCapability {
    Vector,
    Graph,
    Relational,
}
