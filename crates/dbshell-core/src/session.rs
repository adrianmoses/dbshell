use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionMode {
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Clone)]
pub enum CachePolicy {
    None,
    SessionScoped,
    Ttl(Duration),
    Persistent,
}

#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub mode: SessionMode,
    pub cache: CachePolicy,
    pub max_connections: u32,
    pub connection_string: String,
    pub tls: Option<TlsConfig>,
}

#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub ca_cert: Option<PathBuf>,
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    pub accept_invalid_certs: bool,
}
