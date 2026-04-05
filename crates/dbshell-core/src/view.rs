use serde::{Deserialize, Serialize};

use crate::error::{DbError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewMount {
    pub name: String,
    pub table: String,
    pub filter_column: String,
    pub param_type: ParamType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParamType {
    String,
    Integer,
    Uuid,
}

impl ViewMount {
    pub fn cast_param(&self, param: &str) -> Result<serde_json::Value> {
        match self.param_type {
            ParamType::String => Ok(serde_json::Value::String(param.to_string())),
            ParamType::Integer => {
                let n: i64 = param.parse().map_err(|_| {
                    DbError::InvalidPath(format!("expected integer, got '{param}'"))
                })?;
                Ok(serde_json::Value::Number(n.into()))
            }
            ParamType::Uuid => {
                // Basic UUID format validation (8-4-4-4-12 hex chars)
                let valid = param.len() == 36
                    && param.chars().enumerate().all(|(i, c)| match i {
                        8 | 13 | 18 | 23 => c == '-',
                        _ => c.is_ascii_hexdigit(),
                    });
                if !valid {
                    return Err(DbError::InvalidPath(format!(
                        "expected UUID, got '{param}'"
                    )));
                }
                Ok(serde_json::Value::String(param.to_string()))
            }
        }
    }
}
