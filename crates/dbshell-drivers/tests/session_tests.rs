use std::sync::Arc;

use dbshell_core::record::TableSchema;
use dbshell_core::result::ToolPayload;
use dbshell_core::session::{Session, SessionMode};
use dbshell_core::tool_kind::{ToolArgs, ToolCall, ToolKind};
use dbshell_core::vfs_path::VfsPath;

use dbshell_drivers::memory::MemoryDriver;

async fn make_session_with_table() -> Session {
    let driver = MemoryDriver::new()
        .with_table(
            "users",
            TableSchema {
                table: "users".into(),
                columns: vec![],
                primary_key: None,
                indexes: vec![],
            },
            vec![
                serde_json::json!({"id": 1, "name": "Alice", "age": 30}),
                serde_json::json!({"id": 2, "name": "Bob", "age": 25}),
                serde_json::json!({"id": 3, "name": "Charlie", "age": 35}),
            ],
        )
        .await;

    Session::builder()
        .connect("default", Arc::new(driver))
        .build()
}

async fn make_readonly_session() -> Session {
    let driver = MemoryDriver::new()
        .with_table(
            "users",
            TableSchema {
                table: "users".into(),
                columns: vec![],
                primary_key: None,
                indexes: vec![],
            },
            vec![],
        )
        .await;

    Session::builder()
        .connect("default", Arc::new(driver))
        .mode(SessionMode::ReadOnly)
        .build()
}

#[tokio::test]
async fn test_builder_creates_session() {
    let _session = make_session_with_table().await;
}

#[tokio::test]
async fn test_exec_tool_query_table() {
    let session = make_session_with_table().await;
    let tool = ToolCall {
        name: "find".into(),
        kind: ToolKind::Find,
        path: Some(VfsPath::parse("/db/tables/users").unwrap()),
        args: ToolArgs::default(),
        stdin: None,
    };
    let result = session.exec_tool(tool).await;
    assert!(result.is_ok(), "exec_tool failed: {:?}", result.err());
    let result = result.unwrap();
    assert_eq!(result.exit_code, 0);
    // Should have 3 rows
    match &result.payload {
        ToolPayload::Records(rs) => assert_eq!(rs.rows.len(), 3),
        other => panic!("expected Records, got {other:?}"),
    }
}

#[tokio::test]
async fn test_exec_tool_describe_table() {
    let session = make_session_with_table().await;
    let tool = ToolCall {
        name: "cat".into(),
        kind: ToolKind::Cat,
        path: Some(VfsPath::parse("/db/tables/users").unwrap()),
        args: ToolArgs::default(),
        stdin: None,
    };
    let result = session.exec_tool(tool).await;
    assert!(result.is_ok(), "exec_tool failed: {:?}", result.err());
}

#[tokio::test]
async fn test_exec_tool_list_tables() {
    let session = make_session_with_table().await;
    let tool = ToolCall {
        name: "ls".into(),
        kind: ToolKind::Ls,
        path: Some(VfsPath::parse("/db/tables").unwrap()),
        args: ToolArgs::default(),
        stdin: None,
    };
    let result = session.exec_tool(tool).await;
    assert!(result.is_ok(), "exec_tool failed: {:?}", result.err());
}

#[tokio::test]
async fn test_readonly_allows_reads() {
    let session = make_readonly_session().await;
    let tool = ToolCall {
        name: "cat".into(),
        kind: ToolKind::Cat,
        path: Some(VfsPath::parse("/db/tables/users").unwrap()),
        args: ToolArgs::default(),
        stdin: None,
    };
    let result = session.exec_tool(tool).await;
    assert!(result.is_ok(), "readonly read failed: {:?}", result.err());
}

#[tokio::test]
async fn test_begin_unsupported_for_memory_driver() {
    let session = make_session_with_table().await;
    let result = session.begin().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_commit_without_begin_errors() {
    let session = make_session_with_table().await;
    let result = session.commit().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_rollback_without_begin_errors() {
    let session = make_session_with_table().await;
    let result = session.rollback().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_exec_simple_command() {
    let session = make_session_with_table().await;
    let results = session.exec("ls /db/tables").await;
    assert!(results.is_ok(), "exec failed: {:?}", results.err());
    let results = results.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].exit_code, 0);
}

#[tokio::test]
async fn test_exec_sequential_commands() {
    let session = make_session_with_table().await;
    let results = session.exec("ls /db/tables ; ls /db/vectors").await;
    assert!(results.is_ok(), "exec failed: {:?}", results.err());
    let results = results.unwrap();
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn test_exec_pipeline_find_with_head() {
    let session = make_session_with_table().await;
    let results = session.exec("find /db/tables/users | head -n 2").await;
    assert!(results.is_ok(), "exec failed: {:?}", results.err());
    let results = results.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].exit_code, 0);
    // head -n 2 should limit to 2 rows (pushed down)
    match &results[0].payload {
        ToolPayload::Records(rs) => assert_eq!(rs.rows.len(), 2),
        other => panic!("expected Records, got {other:?}"),
    }
}

#[tokio::test]
async fn test_exec_pipeline_find_with_wc() {
    let session = make_session_with_table().await;
    let results = session.exec("find /db/tables/users | wc -l").await;
    assert!(results.is_ok(), "exec failed: {:?}", results.err());
    let results = results.unwrap();
    assert_eq!(results.len(), 1);
    // wc should count 3 rows
    assert!(
        results[0].stdout.contains("3"),
        "stdout was: {}",
        results[0].stdout
    );
}

#[tokio::test]
async fn test_exec_pipeline_find_grep_wc() {
    let session = make_session_with_table().await;
    // find | grep Alice | wc — grep pushes down as LIKE, wc is client-side
    let results = session
        .exec("find /db/tables/users | grep Alice | wc -l")
        .await;
    assert!(results.is_ok(), "exec failed: {:?}", results.err());
    let results = results.unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_result_store_populated_after_query() {
    let session = make_session_with_table().await;
    // Execute a find to populate result store
    let _ = session.exec("find /db/tables/users").await;

    // Read from /results/last
    let tool = ToolCall {
        name: "cat".into(),
        kind: ToolKind::Cat,
        path: Some(VfsPath::parse("/results/last").unwrap()),
        args: ToolArgs::default(),
        stdin: None,
    };
    let result = session.exec_tool(tool).await;
    assert!(
        result.is_ok(),
        "result store read failed: {:?}",
        result.err()
    );
    match &result.unwrap().payload {
        ToolPayload::Records(rs) => assert_eq!(rs.rows.len(), 3),
        other => panic!("expected Records from result store, got {other:?}"),
    }
}

#[tokio::test]
async fn test_exec_list_collections() {
    let session = make_session_with_table().await;
    let tool = ToolCall {
        name: "ls".into(),
        kind: ToolKind::Ls,
        path: Some(VfsPath::parse("/db/vectors").unwrap()),
        args: ToolArgs::default(),
        stdin: None,
    };
    let result = session.exec_tool(tool).await;
    assert!(
        result.is_ok(),
        "list collections failed: {:?}",
        result.err()
    );
}
