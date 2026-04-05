use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergeRequest {
    pub left: MergeSide,
    pub right: MergeSide,
    pub merge_type: MergeType,
    pub on: MergeCondition,
    pub output_fields: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergeSide {
    pub table: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MergeType {
    Inner,
    Left,
    Right,
    FullOuter,
    AntiLeft,
    AntiRight,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergeCondition {
    pub left_col: String,
    pub right_col: String,
}
