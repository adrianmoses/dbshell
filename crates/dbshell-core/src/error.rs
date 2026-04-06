use std::fmt;

#[derive(Debug)]
pub enum DbError {
    Unsupported(&'static str),
    NotFound(String),
    PermissionDenied(String),
    DialectMismatch {
        expected: &'static str,
        got: &'static str,
    },
    ConnectionFailed(String),
    InvalidFilter(String),
    InvalidEmbedding(String),
    InvalidPath(String),
    InvalidState(&'static str),
    ParseError(String),
    DriverError(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Unsupported(op) => write!(f, "unsupported: {op}"),
            DbError::NotFound(what) => write!(f, "not found: {what}"),
            DbError::PermissionDenied(msg) => write!(f, "permission denied: {msg}"),
            DbError::DialectMismatch { expected, got } => {
                write!(f, "dialect mismatch: expected {expected}, got {got}")
            }
            DbError::ConnectionFailed(msg) => write!(f, "connection failed: {msg}"),
            DbError::InvalidFilter(msg) => write!(f, "invalid filter: {msg}"),
            DbError::InvalidEmbedding(msg) => write!(f, "invalid embedding: {msg}"),
            DbError::InvalidPath(msg) => write!(f, "invalid path: {msg}"),
            DbError::InvalidState(msg) => write!(f, "invalid state: {msg}"),
            DbError::ParseError(msg) => write!(f, "parse error: {msg}"),
            DbError::DriverError(err) => write!(f, "driver error: {err}"),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DbError::DriverError(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

impl DbError {
    pub fn exit_code(&self) -> i32 {
        match self {
            DbError::NotFound(_) => 1,
            DbError::PermissionDenied(_) => 2,
            DbError::DialectMismatch { .. } => 3,
            DbError::Unsupported(_) => 4,
            DbError::ConnectionFailed(_) => 5,
            DbError::InvalidFilter(_) => 6,
            DbError::InvalidEmbedding(_) => 7,
            DbError::InvalidPath(_) => 8,
            DbError::InvalidState(_) => 9,
            DbError::ParseError(_) => 10,
            DbError::DriverError(_) => 127,
        }
    }
}

pub type Result<T> = std::result::Result<T, DbError>;
