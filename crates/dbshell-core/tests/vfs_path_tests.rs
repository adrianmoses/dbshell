use dbshell_core::vfs_path::{ResultId, VfsPath, VfsPathKind};

fn parse(input: &str) -> VfsPath {
    VfsPath::parse(input).unwrap_or_else(|e| panic!("failed to parse '{input}': {e}"))
}

fn parse_err(input: &str) {
    assert!(
        VfsPath::parse(input).is_err(),
        "expected error for '{input}'"
    );
}

// ── /db/ paths ─────────────────────────────────────────────────────

#[test]
fn test_parse_db_root() {
    assert_eq!(parse("/db").kind, VfsPathKind::DbRoot);
    assert_eq!(parse("/db/").kind, VfsPathKind::DbRoot);
}

#[test]
fn test_parse_vector_root() {
    assert_eq!(parse("/db/vectors").kind, VfsPathKind::VectorRoot);
    assert_eq!(parse("/db/vectors/").kind, VfsPathKind::VectorRoot);
}

#[test]
fn test_parse_collection() {
    assert_eq!(
        parse("/db/vectors/tracks").kind,
        VfsPathKind::Collection {
            name: "tracks".into()
        }
    );
}

#[test]
fn test_parse_graph_root() {
    assert_eq!(parse("/db/graphs").kind, VfsPathKind::GraphRoot);
    assert_eq!(parse("/db/graphs/").kind, VfsPathKind::GraphRoot);
}

#[test]
fn test_parse_graph_node_root() {
    assert_eq!(parse("/db/graphs/nodes").kind, VfsPathKind::GraphNodeRoot);
    assert_eq!(parse("/db/graphs/nodes/").kind, VfsPathKind::GraphNodeRoot);
}

#[test]
fn test_parse_graph_edge_root() {
    assert_eq!(parse("/db/graphs/edges").kind, VfsPathKind::GraphEdgeRoot);
}

#[test]
fn test_parse_graph_node() {
    assert_eq!(
        parse("/db/graphs/nodes/Artist").kind,
        VfsPathKind::GraphNode {
            label: "Artist".into()
        }
    );
}

#[test]
fn test_parse_graph_edge() {
    assert_eq!(
        parse("/db/graphs/edges/WROTE").kind,
        VfsPathKind::GraphEdge {
            edge_type: "WROTE".into()
        }
    );
}

#[test]
fn test_parse_table_root() {
    assert_eq!(parse("/db/tables").kind, VfsPathKind::TableRoot);
    assert_eq!(parse("/db/tables/").kind, VfsPathKind::TableRoot);
}

#[test]
fn test_parse_table() {
    assert_eq!(
        parse("/db/tables/users").kind,
        VfsPathKind::Table {
            name: "users".into()
        }
    );
}

#[test]
fn test_parse_view() {
    assert_eq!(
        parse("/db/tables/orders/by_customer").kind,
        VfsPathKind::View {
            table: "orders".into(),
            view: "by_customer".into()
        }
    );
}

#[test]
fn test_parse_view_entry() {
    assert_eq!(
        parse("/db/tables/orders/by_customer/42").kind,
        VfsPathKind::ViewEntry {
            table: "orders".into(),
            view: "by_customer".into(),
            param: "42".into()
        }
    );
}

#[test]
fn test_parse_view_entry_string_param() {
    assert_eq!(
        parse("/db/tables/orders/by_status/shipped").kind,
        VfsPathKind::ViewEntry {
            table: "orders".into(),
            view: "by_status".into(),
            param: "shipped".into()
        }
    );
}

// ── /links/ paths ──────────────────────────────────────────────────

#[test]
fn test_parse_symlink() {
    assert_eq!(
        parse("/links/vip-orders").kind,
        VfsPathKind::Symlink {
            name: "vip-orders".into()
        }
    );
}

// ── /search/ paths ─────────────────────────────────────────────────

#[test]
fn test_parse_search_root() {
    assert_eq!(parse("/search").kind, VfsPathKind::SearchRoot);
    assert_eq!(parse("/search/").kind, VfsPathKind::SearchRoot);
}

#[test]
fn test_parse_search_collection() {
    assert_eq!(
        parse("/search/tracks").kind,
        VfsPathKind::SearchCollection {
            collection: "tracks".into()
        }
    );
}

#[test]
fn test_parse_search_query() {
    assert_eq!(
        parse("/search/tracks/blue suede shoes").kind,
        VfsPathKind::SearchQuery {
            collection: "tracks".into(),
            query: "blue suede shoes".into()
        }
    );
}

#[test]
fn test_parse_search_query_with_slashes() {
    assert_eq!(
        parse("/search/docs/quarterly/revenue").kind,
        VfsPathKind::SearchQuery {
            collection: "docs".into(),
            query: "quarterly/revenue".into()
        }
    );
}

// ── /results/ paths ────────────────────────────────────────────────

#[test]
fn test_parse_result() {
    assert_eq!(
        parse("/results/last").kind,
        VfsPathKind::Result {
            id: ResultId("last".into())
        }
    );
}

#[test]
fn test_parse_result_uuid() {
    assert_eq!(
        parse("/results/abc-123-def").kind,
        VfsPathKind::Result {
            id: ResultId("abc-123-def".into())
        }
    );
}

// ── /tmp/ paths ────────────────────────────────────────────────────

#[test]
fn test_parse_tmp() {
    assert_eq!(
        parse("/tmp/scratch").kind,
        VfsPathKind::Tmp {
            name: "scratch".into()
        }
    );
}

// ── Normalization ──────────────────────────────────────────────────

#[test]
fn test_parse_trailing_slash_normalization() {
    assert_eq!(
        parse("/db/tables/users/").kind,
        parse("/db/tables/users").kind
    );
}

// ── Error cases ────────────────────────────────────────────────────

#[test]
fn test_parse_empty_error() {
    parse_err("");
}

#[test]
fn test_parse_no_leading_slash_error() {
    parse_err("db/tables");
}

#[test]
fn test_parse_unknown_namespace_error() {
    parse_err("/foo/bar");
}

#[test]
fn test_parse_bare_slash_error() {
    parse_err("/");
}
