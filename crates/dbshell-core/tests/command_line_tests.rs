use dbshell_core::command_line::{CommandLine, Separator};
use dbshell_core::pipeline::PushdownCapability;
use dbshell_core::tool_kind::ToolKind;

#[test]
fn test_simple_pipe() {
    let cl = CommandLine::parse("find /db/tables/users | head -n 5").unwrap();
    assert_eq!(cl.groups.len(), 1);
    let stages = &cl.groups[0].pipeline.stages;
    assert_eq!(stages.len(), 2);
    assert_eq!(stages[0].tool.kind, ToolKind::Find);
    assert_eq!(stages[1].tool.kind, ToolKind::Head);
    assert_eq!(stages[1].pushdown, PushdownCapability::Limit { count: 5 });
    assert_eq!(cl.groups[0].separator, Separator::End);
}

#[test]
fn test_sequential_commands() {
    let cl = CommandLine::parse("ls /db/tables ; ls /db/vectors").unwrap();
    assert_eq!(cl.groups.len(), 2);
    assert_eq!(cl.groups[0].separator, Separator::Sequential);
    assert_eq!(cl.groups[1].separator, Separator::End);
    assert_eq!(cl.groups[0].pipeline.stages[0].tool.kind, ToolKind::Ls);
    assert_eq!(cl.groups[1].pipeline.stages[0].tool.kind, ToolKind::Ls);
}

#[test]
fn test_parallel_commands() {
    let cl = CommandLine::parse("find /db/tables/users & find /db/tables/orders").unwrap();
    assert_eq!(cl.groups.len(), 2);
    assert_eq!(cl.groups[0].separator, Separator::Background);
    assert_eq!(cl.groups[1].separator, Separator::End);
}

#[test]
fn test_mixed_separators() {
    let cl =
        CommandLine::parse("find /db/tables/a | head -n 5 ; ls /db & cat /db/tables/b").unwrap();
    assert_eq!(cl.groups.len(), 3);
    // First group: pipe of find | head, separated by ;
    assert_eq!(cl.groups[0].pipeline.stages.len(), 2);
    assert_eq!(cl.groups[0].separator, Separator::Sequential);
    // Second group: ls, separated by &
    assert_eq!(cl.groups[1].pipeline.stages.len(), 1);
    assert_eq!(cl.groups[1].separator, Separator::Background);
    // Third group: cat, end
    assert_eq!(cl.groups[2].pipeline.stages.len(), 1);
    assert_eq!(cl.groups[2].separator, Separator::End);
}

#[test]
fn test_transaction_keywords_standalone() {
    let cl = CommandLine::parse("begin").unwrap();
    assert_eq!(cl.groups.len(), 1);
    assert_eq!(cl.groups[0].pipeline.stages[0].tool.kind, ToolKind::Begin);

    let cl = CommandLine::parse("commit").unwrap();
    assert_eq!(cl.groups[0].pipeline.stages[0].tool.kind, ToolKind::Commit);

    let cl = CommandLine::parse("rollback").unwrap();
    assert_eq!(
        cl.groups[0].pipeline.stages[0].tool.kind,
        ToolKind::Rollback
    );
}

#[test]
fn test_transaction_keyword_in_pipe_is_error() {
    assert!(CommandLine::parse("begin | find /db/tables/users").is_err());
    assert!(CommandLine::parse("find /db/tables/users | commit").is_err());
}

#[test]
fn test_quoted_string_preserved() {
    let cl = CommandLine::parse("echo '{\"name\":\"Alice\"}' /db/tables/users").unwrap();
    let stage = &cl.groups[0].pipeline.stages[0];
    assert_eq!(stage.tool.kind, ToolKind::Echo);
    assert!(stage
        .tool
        .args
        .positional
        .contains(&"{\"name\":\"Alice\"}".to_string()));
}

#[test]
fn test_grep_pushdown() {
    let cl = CommandLine::parse("find /db/tables/users | grep Alice").unwrap();
    let stages = &cl.groups[0].pipeline.stages;
    assert_eq!(stages.len(), 2);
    assert_eq!(
        stages[1].pushdown,
        PushdownCapability::GrepFilter {
            pattern: "Alice".into()
        }
    );
}

#[test]
fn test_head_default_limit() {
    let cl = CommandLine::parse("find /db/tables/users | head").unwrap();
    let stages = &cl.groups[0].pipeline.stages;
    assert_eq!(stages[1].pushdown, PushdownCapability::Limit { count: 10 });
}

#[test]
fn test_empty_input_error() {
    assert!(CommandLine::parse("").is_err());
    assert!(CommandLine::parse("   ").is_err());
}

#[test]
fn test_unterminated_quote_error() {
    assert!(CommandLine::parse("echo 'unclosed").is_err());
}

#[test]
fn test_unknown_tool_error() {
    assert!(CommandLine::parse("unknown_cmd /db/tables").is_err());
}

#[test]
fn test_pipe_three_stages() {
    let cl = CommandLine::parse("find /db/tables/users | grep Alice | wc -l").unwrap();
    let stages = &cl.groups[0].pipeline.stages;
    assert_eq!(stages.len(), 3);
    assert_eq!(stages[0].tool.kind, ToolKind::Find);
    assert_eq!(stages[1].tool.kind, ToolKind::Grep);
    assert_eq!(stages[2].tool.kind, ToolKind::Wc);
}

#[test]
fn test_redirect_append() {
    let cl = CommandLine::parse("echo '{\"id\":1}' >> /db/tables/orders").unwrap();
    let stage = &cl.groups[0].pipeline.stages[0];
    assert_eq!(stage.tool.kind, ToolKind::Echo);
}

#[test]
fn test_sequential_with_begin_commit() {
    let cl = CommandLine::parse("begin ; echo '{\"id\":1}' >> /db/tables/orders ; commit").unwrap();
    assert_eq!(cl.groups.len(), 3);
    assert_eq!(cl.groups[0].pipeline.stages[0].tool.kind, ToolKind::Begin);
    assert_eq!(cl.groups[1].pipeline.stages[0].tool.kind, ToolKind::Echo);
    assert_eq!(cl.groups[2].pipeline.stages[0].tool.kind, ToolKind::Commit);
}
