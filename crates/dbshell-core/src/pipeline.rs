use crate::filter::Filter;
use crate::operation::DbOperation;
use crate::tool_kind::ToolCall;

#[derive(Debug)]
pub struct Pipeline {
    pub stages: Vec<PipeStage>,
}

#[derive(Debug)]
pub struct PipeStage {
    pub tool: ToolCall,
    pub pushdown: PushdownCapability,
}

/// Declares what a tool stage could contribute to a server-side query
/// if folded into the lead stage. Set during parsing, consumed by the optimizer.
#[derive(Debug, Clone, PartialEq)]
pub enum PushdownCapability {
    /// Cannot be pushed down — always runs client-side (e.g. wc, sort).
    None,
    /// Can become LIMIT (head -n).
    Limit { count: u64 },
    /// Can become OFFSET (tail -n +N).
    Offset { count: u64 },
    /// Can become WHERE LIKE (grep pattern -> text match).
    GrepFilter { pattern: String },
    /// Can become WHERE clause (filter 'field > value' -> comparison).
    FieldFilter(Filter),
}

/// The output of optimization. Splits the pipeline into a server-side query
/// and a client-side tail.
#[derive(Debug)]
pub struct ExecutionPlan {
    /// The (potentially enriched) database operation to execute server-side.
    pub server_op: DbOperation,
    /// Remaining stages that run client-side over the materialized stdout.
    /// Empty if the entire pipeline was pushed down.
    pub client_stages: Vec<PipeStage>,
}

pub struct PipelineOptimizer;

impl PipelineOptimizer {
    /// Takes a parsed Pipeline and the base DbOperation from VFS resolution,
    /// returns an optimized ExecutionPlan.
    ///
    /// Walks stages front-to-back. Folds pushdown-eligible stages into the
    /// lead stage's DbOperation. Stops at the first non-pushable stage
    /// (the materialization boundary).
    pub fn optimize(pipeline: Pipeline, base_op: DbOperation) -> ExecutionPlan {
        let mut server_op = base_op;
        let mut stages = pipeline.stages.into_iter();

        // Skip lead stage (already resolved to base_op)
        let _lead = stages.next();

        let mut client_stages = Vec::new();
        let mut hit_boundary = false;

        for stage in stages {
            if hit_boundary {
                client_stages.push(stage);
                continue;
            }

            match &stage.pushdown {
                PushdownCapability::None => {
                    hit_boundary = true;
                    client_stages.push(stage);
                }
                PushdownCapability::Limit { count } => {
                    if !Self::fold_limit(&mut server_op, *count) {
                        hit_boundary = true;
                        client_stages.push(stage);
                    }
                }
                PushdownCapability::Offset { count } => {
                    if !Self::fold_offset(&mut server_op, *count) {
                        hit_boundary = true;
                        client_stages.push(stage);
                    }
                }
                PushdownCapability::FieldFilter(filter) => {
                    if !Self::fold_filter(&mut server_op, filter.clone()) {
                        hit_boundary = true;
                        client_stages.push(stage);
                    }
                }
                PushdownCapability::GrepFilter { pattern } => {
                    if !Self::fold_grep(&mut server_op, pattern.clone()) {
                        hit_boundary = true;
                        client_stages.push(stage);
                    }
                }
            }
        }

        ExecutionPlan {
            server_op,
            client_stages,
        }
    }

    /// Fold a LIMIT into the operation. Returns true if successful.
    fn fold_limit(op: &mut DbOperation, count: u64) -> bool {
        match op {
            DbOperation::QueryTable { request, .. } => {
                request.limit = Some(count);
                true
            }
            DbOperation::VectorSearch { request, .. } => {
                request.limit = count;
                true
            }
            _ => false,
        }
    }

    /// Fold an OFFSET into the operation. Returns true if successful.
    fn fold_offset(op: &mut DbOperation, count: u64) -> bool {
        match op {
            DbOperation::QueryTable { request, .. } => {
                request.offset = Some(count);
                true
            }
            _ => false,
        }
    }

    /// Fold a structured filter into the operation. Returns true if successful.
    fn fold_filter(op: &mut DbOperation, filter: Filter) -> bool {
        match op {
            DbOperation::QueryTable { request, .. } => {
                request.add_filter(filter);
                true
            }
            _ => false,
        }
    }

    /// Fold a grep pattern as a LIKE filter. Returns true if successful.
    fn fold_grep(op: &mut DbOperation, pattern: String) -> bool {
        match op {
            DbOperation::QueryTable { request, .. } => {
                request.add_filter(Filter::Like {
                    field: "*".to_string(),
                    pattern,
                });
                true
            }
            _ => false,
        }
    }
}
