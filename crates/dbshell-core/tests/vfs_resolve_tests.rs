use dbshell_core::filter::Filter;
use dbshell_core::operation::DbOperation;
use dbshell_core::tool_kind::ToolKind;
use dbshell_core::vfs::VirtualFS;
use dbshell_core::vfs_path::{VfsPath, VfsPathKind};
use dbshell_core::view::{ParamType, ViewMount};

fn vfs_with_views() -> VirtualFS {
    VirtualFS::new().with_views(vec![
        ViewMount {
            name: "by_customer".into(),
            table: "orders".into(),
            filter_column: "customer_id".into(),
            param_type: ParamType::Integer,
        },
        ViewMount {
            name: "by_status".into(),
            table: "orders".into(),
            filter_column: "status".into(),
            param_type: ParamType::String,
        },
    ])
}

#[test]
fn test_resolve_db_root() {
    let vfs = VirtualFS::new();
    let path = VfsPath::parse("/db").unwrap();
    let op = vfs.resolve_default(&path, "pg").unwrap();
    assert!(matches!(op, DbOperation::ListCollections { driver } if driver == "pg"));
}

#[test]
fn test_resolve_table_root() {
    let vfs = VirtualFS::new();
    let path = VfsPath::parse("/db/tables").unwrap();
    let op = vfs.resolve_default(&path, "pg").unwrap();
    assert!(matches!(op, DbOperation::ListTables { driver } if driver == "pg"));
}

#[test]
fn test_resolve_table() {
    let vfs = VirtualFS::new();
    let path = VfsPath::parse("/db/tables/users").unwrap();
    let op = vfs.resolve_default(&path, "pg").unwrap();
    assert!(
        matches!(op, DbOperation::DescribeTable { driver, table } if driver == "pg" && table == "users")
    );
}

#[test]
fn test_resolve_collection() {
    let vfs = VirtualFS::new();
    let path = VfsPath::parse("/db/vectors/tracks").unwrap();
    let op = vfs.resolve_default(&path, "pg").unwrap();
    assert!(
        matches!(op, DbOperation::InspectCollection { driver, collection } if driver == "pg" && collection == "tracks")
    );
}

#[test]
fn test_resolve_view_entry_integer() {
    let vfs = vfs_with_views();
    let path = VfsPath::parse("/db/tables/orders/by_customer/42").unwrap();
    let op = vfs.resolve_default(&path, "pg").unwrap();

    match op {
        DbOperation::QueryTable {
            driver,
            table,
            request,
        } => {
            assert_eq!(driver, "pg");
            assert_eq!(table, "orders");
            assert_eq!(
                request.filter,
                Some(Filter::Eq {
                    field: "customer_id".into(),
                    value: serde_json::Value::Number(42.into()),
                })
            );
        }
        other => panic!("expected QueryTable, got {other:?}"),
    }
}

#[test]
fn test_resolve_view_entry_string() {
    let vfs = vfs_with_views();
    let path = VfsPath::parse("/db/tables/orders/by_status/shipped").unwrap();
    let op = vfs.resolve_default(&path, "pg").unwrap();

    match op {
        DbOperation::QueryTable {
            driver,
            table,
            request,
        } => {
            assert_eq!(driver, "pg");
            assert_eq!(table, "orders");
            assert_eq!(
                request.filter,
                Some(Filter::Eq {
                    field: "status".into(),
                    value: serde_json::Value::String("shipped".into()),
                })
            );
        }
        other => panic!("expected QueryTable, got {other:?}"),
    }
}

#[test]
fn test_resolve_view_entry_bad_cast() {
    let vfs = vfs_with_views();
    let path = VfsPath::parse("/db/tables/orders/by_customer/abc").unwrap();
    assert!(vfs.resolve_default(&path, "pg").is_err());
}

#[test]
fn test_resolve_view_not_found() {
    let vfs = VirtualFS::new(); // no views configured
    let path = VfsPath::parse("/db/tables/orders/by_customer/42").unwrap();
    assert!(vfs.resolve_default(&path, "pg").is_err());
}

#[test]
fn test_resolve_symlink() {
    let mut vfs = vfs_with_views();
    let target = VfsPath::parse("/db/tables/users").unwrap();
    vfs.add_symlink("my-users".into(), target).unwrap();

    let link_path = VfsPath {
        raw: "/links/my-users".into(),
        kind: VfsPathKind::Symlink {
            name: "my-users".into(),
        },
    };
    let op = vfs.resolve_default(&link_path, "pg").unwrap();
    assert!(
        matches!(op, DbOperation::DescribeTable { driver, table } if driver == "pg" && table == "users")
    );
}

#[test]
fn test_resolve_symlink_not_found() {
    let vfs = VirtualFS::new();
    let path = VfsPath {
        raw: "/links/nope".into(),
        kind: VfsPathKind::Symlink {
            name: "nope".into(),
        },
    };
    assert!(vfs.resolve_default(&path, "pg").is_err());
}

#[test]
fn test_resolve_symlink_chain_error() {
    let mut vfs = VirtualFS::new();
    // Manually insert a symlink that points to another symlink path
    let target = VfsPath {
        raw: "/links/other".into(),
        kind: VfsPathKind::Symlink {
            name: "other".into(),
        },
    };
    // add_symlink should reject this
    assert!(vfs.add_symlink("chained".into(), target).is_err());
}

#[test]
fn test_resolve_search_query() {
    let vfs = VirtualFS::new();
    let path = VfsPath::parse("/search/tracks/blue suede shoes").unwrap();
    let op = vfs.resolve_default(&path, "pg").unwrap();
    assert!(matches!(op, DbOperation::VectorSearch { collection, .. } if collection == "tracks"));
}

// --- Tool-aware resolution tests ---

#[test]
fn test_resolve_table_ls_describes() {
    let vfs = VirtualFS::new();
    let path = VfsPath::parse("/db/tables/users").unwrap();
    let op = vfs.resolve(&path, "pg", &ToolKind::Ls).unwrap();
    assert!(
        matches!(op, DbOperation::DescribeTable { driver, table } if driver == "pg" && table == "users")
    );
}

#[test]
fn test_resolve_table_find_queries() {
    let vfs = VirtualFS::new();
    let path = VfsPath::parse("/db/tables/users").unwrap();
    let op = vfs.resolve(&path, "pg", &ToolKind::Find).unwrap();
    assert!(
        matches!(op, DbOperation::QueryTable { driver, table, .. } if driver == "pg" && table == "users")
    );
}

#[test]
fn test_resolve_table_cat_describes() {
    let vfs = VirtualFS::new();
    let path = VfsPath::parse("/db/tables/users").unwrap();
    let op = vfs.resolve(&path, "pg", &ToolKind::Cat).unwrap();
    assert!(
        matches!(op, DbOperation::DescribeTable { driver, table } if driver == "pg" && table == "users")
    );
}

#[test]
fn test_resolve_collection_ls_lists() {
    let vfs = VirtualFS::new();
    let path = VfsPath::parse("/db/vectors/tracks").unwrap();
    let op = vfs.resolve(&path, "pg", &ToolKind::Ls).unwrap();
    assert!(matches!(op, DbOperation::ListCollections { driver } if driver == "pg"));
}

#[test]
fn test_resolve_collection_find_searches() {
    let vfs = VirtualFS::new();
    let path = VfsPath::parse("/db/vectors/tracks").unwrap();
    let op = vfs.resolve(&path, "pg", &ToolKind::Find).unwrap();
    assert!(matches!(op, DbOperation::VectorSearch { collection, .. } if collection == "tracks"));
}

#[test]
fn test_resolve_collection_cat_inspects() {
    let vfs = VirtualFS::new();
    let path = VfsPath::parse("/db/vectors/tracks").unwrap();
    let op = vfs.resolve(&path, "pg", &ToolKind::Cat).unwrap();
    assert!(
        matches!(op, DbOperation::InspectCollection { collection, .. } if collection == "tracks")
    );
}
