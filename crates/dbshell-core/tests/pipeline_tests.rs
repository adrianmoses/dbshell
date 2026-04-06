use dbshell_core::filter::Filter;
use dbshell_core::operation::DbOperation;
use dbshell_core::pipeline::{PipeStage, Pipeline, PipelineOptimizer, PushdownCapability};
use dbshell_core::record::TableQuery;
use dbshell_core::tool_kind::{ToolArgs, ToolCall, ToolKind};
use dbshell_core::vfs_path::VfsPath;

fn make_tool(kind: ToolKind) -> ToolCall {
    ToolCall {
        name: format!("{kind:?}").to_lowercase(),
        kind,
        path: None,
        args: ToolArgs::default(),
        stdin: None,
    }
}

fn make_lead_stage() -> PipeStage {
    PipeStage {
        tool: ToolCall {
            name: "find".into(),
            kind: ToolKind::Find,
            path: Some(VfsPath::parse("/db/tables/users").unwrap()),
            args: ToolArgs::default(),
            stdin: None,
        },
        pushdown: PushdownCapability::None,
    }
}

fn base_query_op() -> DbOperation {
    DbOperation::QueryTable {
        driver: "pg".into(),
        table: "users".into(),
        request: TableQuery::default(),
    }
}

#[test]
fn test_pushdown_limit() {
    let pipeline = Pipeline {
        stages: vec![
            make_lead_stage(),
            PipeStage {
                tool: make_tool(ToolKind::Head),
                pushdown: PushdownCapability::Limit { count: 5 },
            },
        ],
    };

    let plan = PipelineOptimizer::optimize(pipeline, base_query_op());
    assert!(plan.client_stages.is_empty());
    match &plan.server_op {
        DbOperation::QueryTable { request, .. } => {
            assert_eq!(request.limit, Some(5));
        }
        other => panic!("expected QueryTable, got {other:?}"),
    }
}

#[test]
fn test_pushdown_offset() {
    let pipeline = Pipeline {
        stages: vec![
            make_lead_stage(),
            PipeStage {
                tool: make_tool(ToolKind::Tail),
                pushdown: PushdownCapability::Offset { count: 100 },
            },
        ],
    };

    let plan = PipelineOptimizer::optimize(pipeline, base_query_op());
    assert!(plan.client_stages.is_empty());
    match &plan.server_op {
        DbOperation::QueryTable { request, .. } => {
            assert_eq!(request.offset, Some(100));
        }
        other => panic!("expected QueryTable, got {other:?}"),
    }
}

#[test]
fn test_pushdown_field_filter() {
    let filter = Filter::Gt {
        field: "age".into(),
        value: serde_json::json!(21),
    };
    let pipeline = Pipeline {
        stages: vec![
            make_lead_stage(),
            PipeStage {
                tool: make_tool(ToolKind::Filter),
                pushdown: PushdownCapability::FieldFilter(filter.clone()),
            },
        ],
    };

    let plan = PipelineOptimizer::optimize(pipeline, base_query_op());
    assert!(plan.client_stages.is_empty());
    match &plan.server_op {
        DbOperation::QueryTable { request, .. } => {
            assert_eq!(request.filter, Some(filter));
        }
        other => panic!("expected QueryTable, got {other:?}"),
    }
}

#[test]
fn test_pushdown_filter_then_limit() {
    let filter = Filter::Gt {
        field: "age".into(),
        value: serde_json::json!(21),
    };
    let pipeline = Pipeline {
        stages: vec![
            make_lead_stage(),
            PipeStage {
                tool: make_tool(ToolKind::Filter),
                pushdown: PushdownCapability::FieldFilter(filter.clone()),
            },
            PipeStage {
                tool: make_tool(ToolKind::Head),
                pushdown: PushdownCapability::Limit { count: 20 },
            },
        ],
    };

    let plan = PipelineOptimizer::optimize(pipeline, base_query_op());
    assert!(plan.client_stages.is_empty());
    match &plan.server_op {
        DbOperation::QueryTable { request, .. } => {
            assert_eq!(request.filter, Some(filter));
            assert_eq!(request.limit, Some(20));
        }
        other => panic!("expected QueryTable, got {other:?}"),
    }
}

#[test]
fn test_materialization_boundary() {
    let pipeline = Pipeline {
        stages: vec![
            make_lead_stage(),
            PipeStage {
                tool: make_tool(ToolKind::Filter),
                pushdown: PushdownCapability::FieldFilter(Filter::Gt {
                    field: "age".into(),
                    value: serde_json::json!(21),
                }),
            },
            PipeStage {
                tool: make_tool(ToolKind::Wc),
                pushdown: PushdownCapability::None,
            },
        ],
    };

    let plan = PipelineOptimizer::optimize(pipeline, base_query_op());
    // filter pushed down, wc is client-side
    assert_eq!(plan.client_stages.len(), 1);
    assert!(matches!(
        plan.client_stages[0].pushdown,
        PushdownCapability::None
    ));
}

#[test]
fn test_stages_after_boundary_stay_client_side() {
    let pipeline = Pipeline {
        stages: vec![
            make_lead_stage(),
            PipeStage {
                tool: make_tool(ToolKind::Wc),
                pushdown: PushdownCapability::None,
            },
            PipeStage {
                tool: make_tool(ToolKind::Head),
                pushdown: PushdownCapability::Limit { count: 5 },
            },
        ],
    };

    let plan = PipelineOptimizer::optimize(pipeline, base_query_op());
    // Both wc and head stay client-side (head can't fold after boundary)
    assert_eq!(plan.client_stages.len(), 2);
}

#[test]
fn test_no_pushdown_on_non_query_op() {
    let op = DbOperation::ListTables {
        driver: "pg".into(),
    };
    let pipeline = Pipeline {
        stages: vec![
            make_lead_stage(),
            PipeStage {
                tool: make_tool(ToolKind::Head),
                pushdown: PushdownCapability::Limit { count: 5 },
            },
        ],
    };

    let plan = PipelineOptimizer::optimize(pipeline, op);
    // Can't fold limit into ListTables, so it stays client-side
    assert_eq!(plan.client_stages.len(), 1);
}

#[test]
fn test_grep_pushdown_on_query_table() {
    let pipeline = Pipeline {
        stages: vec![
            make_lead_stage(),
            PipeStage {
                tool: make_tool(ToolKind::Grep),
                pushdown: PushdownCapability::GrepFilter {
                    pattern: "Alice".into(),
                },
            },
        ],
    };

    let plan = PipelineOptimizer::optimize(pipeline, base_query_op());
    assert!(plan.client_stages.is_empty());
    match &plan.server_op {
        DbOperation::QueryTable { request, .. } => {
            assert!(
                matches!(&request.filter, Some(Filter::Like { pattern, .. }) if pattern == "Alice")
            );
        }
        other => panic!("expected QueryTable, got {other:?}"),
    }
}

#[test]
fn test_single_stage_pipeline() {
    let pipeline = Pipeline {
        stages: vec![make_lead_stage()],
    };

    let plan = PipelineOptimizer::optimize(pipeline, base_query_op());
    assert!(plan.client_stages.is_empty());
    assert!(matches!(plan.server_op, DbOperation::QueryTable { .. }));
}
