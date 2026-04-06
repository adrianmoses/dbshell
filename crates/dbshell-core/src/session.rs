use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::command_line::{CommandLine, Separator};
use crate::driver::{DbDriver, DriverTransaction};
use crate::embedder::Embedder;
use crate::error::{DbError, Result};
use crate::operation::DbOperation;
use crate::pipeline::{Pipeline, PipelineOptimizer};
use crate::result::{ToolPayload, ToolResult};
use crate::result_store::ResultStore;
use crate::router::CachedQueryRouter;
use crate::tool_kind::{ToolCall, ToolKind};
use crate::vfs::VirtualFS;
use crate::view::ViewMount;

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

pub struct Session {
    mode: SessionMode,
    vfs: VirtualFS,
    router: CachedQueryRouter,
    results: RwLock<ResultStore>,
    tx: Mutex<Option<Box<dyn DriverTransaction>>>,
    #[allow(dead_code)]
    embedder: Option<Arc<dyn Embedder>>,
}

pub struct SessionBuilder {
    drivers: HashMap<String, Arc<dyn DbDriver>>,
    views: Vec<ViewMount>,
    mode: SessionMode,
    cache_policy: CachePolicy,
    embedder: Option<Arc<dyn Embedder>>,
}

impl SessionBuilder {
    pub fn new() -> Self {
        SessionBuilder {
            drivers: HashMap::new(),
            views: Vec::new(),
            mode: SessionMode::ReadWrite,
            cache_policy: CachePolicy::SessionScoped,
            embedder: None,
        }
    }

    pub fn connect(mut self, name: &str, driver: Arc<dyn DbDriver>) -> Self {
        self.drivers.insert(name.to_string(), driver);
        self
    }

    pub fn mode(mut self, mode: SessionMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn cache_policy(mut self, policy: CachePolicy) -> Self {
        self.cache_policy = policy;
        self
    }

    pub fn with_views(mut self, views: Vec<ViewMount>) -> Self {
        self.views = views;
        self
    }

    pub fn with_embedder(mut self, embedder: Arc<dyn Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    pub fn build(self) -> Session {
        let router = CachedQueryRouter::new(self.drivers, &self.cache_policy);
        let vfs = VirtualFS::new().with_views(self.views);

        Session {
            mode: self.mode,
            vfs,
            router,
            results: RwLock::new(ResultStore::new()),
            tx: Mutex::new(None),
            embedder: self.embedder,
        }
    }
}

impl Default for SessionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn builder() -> SessionBuilder {
        SessionBuilder::new()
    }

    /// Execute a single tool call (no pipes).
    pub async fn exec_tool(&self, tool: ToolCall) -> Result<ToolResult> {
        let driver_name = self.driver_name_from_tool(&tool)?;
        let path = tool
            .path
            .as_ref()
            .ok_or(DbError::InvalidPath("tool call has no path".into()))?;
        let op = self.vfs.resolve(path, &driver_name, &tool.kind)?;

        if self.mode == SessionMode::ReadOnly && op.is_write() {
            return Err(DbError::PermissionDenied("session is read-only".into()));
        }

        // VFS-local operations bypass the router
        match &op {
            DbOperation::ReadResult { .. } => {
                let store = self.results.read().await;
                let id = path.raw.rsplit('/').next().unwrap_or("last");
                if let Some(rs) = store.get(id) {
                    return Ok(ToolResult {
                        stdout: serde_json::to_string(&rs.rows).unwrap_or_default(),
                        stderr: String::new(),
                        exit_code: 0,
                        payload: ToolPayload::Records(rs.clone()),
                    });
                } else {
                    return Err(DbError::NotFound(format!("result: {id}")));
                }
            }
            DbOperation::ListResults => {
                let store = self.results.read().await;
                let ids = store.list_ids();
                return Ok(ToolResult {
                    stdout: ids.join("\n"),
                    stderr: String::new(),
                    exit_code: 0,
                    payload: ToolPayload::Empty,
                });
            }
            _ => {}
        }

        let payload = self.dispatch_op(&op).await?;
        self.store_if_records(&payload).await;
        Ok(self.payload_to_result(payload))
    }

    /// Execute a full input line. Parses into a CommandLine, then dispatches
    /// pipelines sequentially or in parallel.
    pub async fn exec(&self, input: &str) -> Result<Vec<ToolResult>> {
        let command_line = CommandLine::parse(input)?;
        let mut results = Vec::new();

        for group in command_line.groups {
            let stages = &group.pipeline.stages;

            if stages.len() == 1 && stages[0].tool.kind.is_transaction_control() {
                match stages[0].tool.kind {
                    ToolKind::Begin => self.begin().await?,
                    ToolKind::Commit => self.commit().await?,
                    ToolKind::Rollback => self.rollback().await?,
                    _ => unreachable!(),
                }
                results.push(ToolResult::empty());
                continue;
            }

            if matches!(group.separator, Separator::Background) {
                let has_tx = self.tx.lock().await.is_some();
                if has_tx {
                    return Err(DbError::ParseError(
                        "& not allowed inside a transaction".into(),
                    ));
                }
            }

            match self.exec_pipeline(group.pipeline).await {
                Ok(r) => results.push(r),
                Err(e) => results.push(ToolResult::from_error(&e)),
            }
        }

        Ok(results)
    }

    /// Execute a pipe chain: optimize -> dispatch server op -> run client stages.
    pub async fn exec_pipeline(&self, pipeline: Pipeline) -> Result<ToolResult> {
        if pipeline.stages.is_empty() {
            return Err(DbError::ParseError("empty pipeline".into()));
        }

        let lead = &pipeline.stages[0];
        let driver_name = self.driver_name_from_tool(&lead.tool)?;
        let path = lead
            .tool
            .path
            .as_ref()
            .ok_or(DbError::InvalidPath("lead stage has no path".into()))?;
        let base_op = self.vfs.resolve(path, &driver_name, &lead.tool.kind)?;

        if self.mode == SessionMode::ReadOnly && base_op.is_write() {
            return Err(DbError::PermissionDenied("session is read-only".into()));
        }

        let plan = PipelineOptimizer::optimize(pipeline, base_op);
        let payload = self.dispatch_op(&plan.server_op).await?;

        let final_payload = if plan.client_stages.is_empty() {
            payload
        } else {
            self.run_client_stages(payload, &plan.client_stages)?
        };

        self.store_if_records(&final_payload).await;
        Ok(self.payload_to_result(final_payload))
    }

    /// Dispatch an operation through the active transaction or router.
    async fn dispatch_op(&self, op: &DbOperation) -> Result<ToolPayload> {
        let tx_guard = self.tx.lock().await;
        if let Some(ref tx) = *tx_guard {
            tx.execute(op).await
        } else {
            drop(tx_guard);
            self.router.dispatch(op).await
        }
    }

    /// Store a Records payload in the ResultStore.
    async fn store_if_records(&self, payload: &ToolPayload) {
        if let ToolPayload::Records(ref rs) = payload {
            let mut store = self.results.write().await;
            store.store(rs.clone());
        }
    }

    /// Run client-side pipeline stages over a ToolPayload.
    fn run_client_stages(
        &self,
        payload: ToolPayload,
        stages: &[crate::pipeline::PipeStage],
    ) -> Result<ToolPayload> {
        let mut current = payload;

        for stage in stages {
            current = match &stage.tool.kind {
                ToolKind::Wc => {
                    let count = match &current {
                        ToolPayload::Records(rs) => rs.rows.len() as u64,
                        ToolPayload::Listing(list) => list.len() as u64,
                        _ => 0,
                    };
                    ToolPayload::Written { count }
                }
                ToolKind::Head => {
                    let count = match &stage.pushdown {
                        crate::pipeline::PushdownCapability::Limit { count } => *count as usize,
                        _ => 10,
                    };
                    match current {
                        ToolPayload::Records(mut rs) => {
                            rs.rows.truncate(count);
                            ToolPayload::Records(rs)
                        }
                        other => other,
                    }
                }
                ToolKind::Tail => {
                    let count = match &stage.pushdown {
                        crate::pipeline::PushdownCapability::Offset { count } => *count as usize,
                        _ => 10,
                    };
                    match current {
                        ToolPayload::Records(mut rs) => {
                            if count < rs.rows.len() {
                                rs.rows = rs.rows.split_off(count);
                            } else {
                                rs.rows.clear();
                            }
                            ToolPayload::Records(rs)
                        }
                        other => other,
                    }
                }
                ToolKind::Grep => {
                    let pattern = stage
                        .tool
                        .args
                        .positional
                        .first()
                        .cloned()
                        .unwrap_or_default();
                    match current {
                        ToolPayload::Records(mut rs) => {
                            rs.rows.retain(|row| {
                                // Search all string values in the row for the pattern
                                if let Some(obj) = row.as_object() {
                                    obj.values().any(|v| {
                                        v.as_str().map(|s| s.contains(&pattern)).unwrap_or(false)
                                    })
                                } else {
                                    row.as_str().map(|s| s.contains(&pattern)).unwrap_or(false)
                                }
                            });
                            ToolPayload::Records(rs)
                        }
                        other => other,
                    }
                }
                ToolKind::Sort => {
                    let key = stage.tool.args.positional.first().cloned();
                    let reverse = stage.tool.args.flags.contains_key("-r");
                    match current {
                        ToolPayload::Records(mut rs) => {
                            rs.rows.sort_by(|a, b| {
                                let ord = if let Some(ref k) = key {
                                    let va = a.get(k).cloned().unwrap_or(serde_json::Value::Null);
                                    let vb = b.get(k).cloned().unwrap_or(serde_json::Value::Null);
                                    crate::filter::cmp_json_values(&va, &vb)
                                } else {
                                    let sa = serde_json::to_string(a).unwrap_or_default();
                                    let sb = serde_json::to_string(b).unwrap_or_default();
                                    sa.cmp(&sb)
                                };
                                if reverse {
                                    ord.reverse()
                                } else {
                                    ord
                                }
                            });
                            ToolPayload::Records(rs)
                        }
                        other => other,
                    }
                }
                _ => current,
            };
        }

        Ok(current)
    }

    pub async fn begin(&self) -> Result<()> {
        let tx_guard = self.tx.lock().await;
        if tx_guard.is_some() {
            return Err(DbError::InvalidState("transaction already active"));
        }
        drop(tx_guard);

        Err(DbError::Unsupported(
            "transactions not yet supported (Phase 3)",
        ))
    }

    pub async fn commit(&self) -> Result<()> {
        let mut tx_guard = self.tx.lock().await;
        let tx = tx_guard
            .take()
            .ok_or(DbError::InvalidState("no active transaction"))?;
        tx.commit().await
    }

    pub async fn rollback(&self) -> Result<()> {
        let mut tx_guard = self.tx.lock().await;
        let tx = tx_guard
            .take()
            .ok_or(DbError::InvalidState("no active transaction"))?;
        tx.rollback().await
    }

    fn driver_name_from_tool(&self, _tool: &ToolCall) -> Result<String> {
        // Phase 3+: derive from path structure (e.g., /db@pg/tables/...)
        Ok("default".into())
    }

    fn payload_to_result(&self, payload: ToolPayload) -> ToolResult {
        let stdout = match &payload {
            ToolPayload::Records(rs) => rs
                .rows
                .iter()
                .map(|r| serde_json::to_string(r).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n"),
            ToolPayload::Info(info) => serde_json::to_string_pretty(info).unwrap_or_default(),
            ToolPayload::Listing(list) => list
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
                .join("\n"),
            ToolPayload::Written { count } => format!("{count} written"),
            ToolPayload::Deleted { count } => format!("{count} deleted"),
            ToolPayload::Created { name } => format!("created: {name}"),
            ToolPayload::Dropped { name } => format!("dropped: {name}"),
            ToolPayload::ResultRef(path) => path.raw.clone(),
            ToolPayload::Empty => String::new(),
        };

        ToolResult {
            stdout,
            stderr: String::new(),
            exit_code: 0,
            payload,
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.tx.get_mut().is_some() {
            tracing::warn!("session dropped with active transaction -- rolling back");
        }
    }
}
