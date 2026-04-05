use serde_json::json;

use dbshell_core::driver::DbDriver;
use dbshell_core::filter::Filter;
use dbshell_core::record::{ColumnInfo, OrderBy, Record, TableQuery, TableSchema};
use dbshell_core::search::{CollectionSpec, VectorSearchRequest};
use dbshell_drivers::memory::MemoryDriver;

fn simple_schema(pk: &str) -> TableSchema {
    TableSchema {
        table: "test".into(),
        columns: vec![
            ColumnInfo {
                name: pk.into(),
                data_type: "integer".into(),
                nullable: false,
                default: None,
            },
            ColumnInfo {
                name: "name".into(),
                data_type: "text".into(),
                nullable: true,
                default: None,
            },
            ColumnInfo {
                name: "age".into(),
                data_type: "integer".into(),
                nullable: true,
                default: None,
            },
        ],
        primary_key: Some(vec![pk.into()]),
        indexes: vec![],
    }
}

fn test_rows() -> Vec<serde_json::Value> {
    vec![
        json!({"id": 1, "name": "Alice", "age": 30}),
        json!({"id": 2, "name": "Bob", "age": 25}),
        json!({"id": 3, "name": "Charlie", "age": 35}),
        json!({"id": 4, "name": "Diana", "age": 28}),
    ]
}

async fn seeded_driver() -> MemoryDriver {
    MemoryDriver::new()
        .with_table("users", simple_schema("id"), test_rows())
        .await
        .with_collection(
            "tracks",
            CollectionSpec {
                name: "tracks".into(),
                dimensions: 3,
                distance_metric: "cosine".into(),
            },
            vec![
                Record {
                    id: "t1".into(),
                    vector: Some(vec![1.0, 0.0, 0.0]),
                    source_text: Some("rock".into()),
                    payload: json!({"genre": "rock"}),
                },
                Record {
                    id: "t2".into(),
                    vector: Some(vec![0.0, 1.0, 0.0]),
                    source_text: Some("jazz".into()),
                    payload: json!({"genre": "jazz"}),
                },
                Record {
                    id: "t3".into(),
                    vector: Some(vec![0.7, 0.7, 0.0]),
                    source_text: Some("fusion".into()),
                    payload: json!({"genre": "fusion"}),
                },
            ],
        )
        .await
}

// ── Health ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_health() {
    let d = MemoryDriver::new();
    let h = d.health().await.unwrap();
    assert!(h.healthy);
}

// ── Collections ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_collections_empty() {
    let d = MemoryDriver::new();
    let colls = d.list_collections().await.unwrap();
    assert!(colls.is_empty());
}

#[tokio::test]
async fn test_list_collections_seeded() {
    let d = seeded_driver().await;
    let colls = d.list_collections().await.unwrap();
    assert_eq!(colls.len(), 1);
    assert_eq!(colls[0].name, "tracks");
}

#[tokio::test]
async fn test_create_drop_collection() {
    let d = MemoryDriver::new();
    let spec = CollectionSpec {
        name: "new_coll".into(),
        dimensions: 128,
        distance_metric: "cosine".into(),
    };
    d.create_collection(&spec).await.unwrap();
    assert_eq!(d.list_collections().await.unwrap().len(), 1);

    d.drop_collection("new_coll").await.unwrap();
    assert!(d.list_collections().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_create_duplicate_collection_error() {
    let d = seeded_driver().await;
    let spec = CollectionSpec {
        name: "tracks".into(),
        dimensions: 3,
        distance_metric: "cosine".into(),
    };
    assert!(d.create_collection(&spec).await.is_err());
}

#[tokio::test]
async fn test_drop_nonexistent_error() {
    let d = MemoryDriver::new();
    assert!(d.drop_collection("nope").await.is_err());
}

// ── Vector ops ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_upsert_insert() {
    let d = seeded_driver().await;
    let result = d
        .upsert(
            "tracks",
            vec![Record {
                id: "t4".into(),
                vector: Some(vec![0.0, 0.0, 1.0]),
                source_text: None,
                payload: json!({"genre": "blues"}),
            }],
        )
        .await
        .unwrap();
    assert_eq!(result.count, 1);
    let colls = d.list_collections().await.unwrap();
    assert_eq!(colls[0].record_count, 4);
}

#[tokio::test]
async fn test_upsert_update() {
    let d = seeded_driver().await;
    d.upsert(
        "tracks",
        vec![Record {
            id: "t1".into(),
            vector: Some(vec![0.5, 0.5, 0.0]),
            source_text: Some("updated".into()),
            payload: json!({"genre": "updated"}),
        }],
    )
    .await
    .unwrap();

    let results = d
        .vector_search(&VectorSearchRequest {
            collection: "tracks".into(),
            vector: vec![0.5, 0.5, 0.0],
            limit: 1,
            filter: None,
        })
        .await
        .unwrap();
    assert_eq!(results[0].id, "t1");
    assert_eq!(results[0].payload["genre"], "updated");
}

#[tokio::test]
async fn test_vector_search_ranked() {
    let d = seeded_driver().await;
    let results = d
        .vector_search(&VectorSearchRequest {
            collection: "tracks".into(),
            vector: vec![1.0, 0.0, 0.0],
            limit: 10,
            filter: None,
        })
        .await
        .unwrap();

    assert_eq!(results[0].id, "t1"); // exact match
    assert!(results[0].score > results[1].score);
}

#[tokio::test]
async fn test_vector_search_limit() {
    let d = seeded_driver().await;
    let results = d
        .vector_search(&VectorSearchRequest {
            collection: "tracks".into(),
            vector: vec![1.0, 0.0, 0.0],
            limit: 1,
            filter: None,
        })
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_delete_with_filter() {
    let d = seeded_driver().await;
    let count = d
        .delete(
            "tracks",
            &Filter::Eq {
                field: "genre".into(),
                value: json!("rock"),
            },
        )
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(d.list_collections().await.unwrap()[0].record_count, 2);
}

// ── Table ops ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_tables() {
    let d = seeded_driver().await;
    let tables = d.list_tables().await.unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "users");
}

#[tokio::test]
async fn test_describe_table() {
    let d = seeded_driver().await;
    let schema = d.describe_table("users").await.unwrap();
    assert_eq!(schema.columns.len(), 3);
    assert_eq!(schema.primary_key, Some(vec!["id".into()]));
}

#[tokio::test]
async fn test_describe_nonexistent_table() {
    let d = MemoryDriver::new();
    assert!(d.describe_table("nope").await.is_err());
}

#[tokio::test]
async fn test_query_table_no_filter() {
    let d = seeded_driver().await;
    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: None,
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs.rows.len(), 4);
}

#[tokio::test]
async fn test_query_table_eq() {
    let d = seeded_driver().await;
    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: Some(Filter::Eq {
                    field: "name".into(),
                    value: json!("Alice"),
                }),
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs.rows.len(), 1);
    assert_eq!(rs.rows[0]["name"], "Alice");
}

#[tokio::test]
async fn test_query_table_gt_lt() {
    let d = seeded_driver().await;
    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: Some(Filter::Gt {
                    field: "age".into(),
                    value: json!(28),
                }),
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    // Alice (30) and Charlie (35)
    assert_eq!(rs.rows.len(), 2);

    let rs2 = d
        .query_table(
            "users",
            &TableQuery {
                filter: Some(Filter::Lt {
                    field: "age".into(),
                    value: json!(28),
                }),
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    // Bob (25)
    assert_eq!(rs2.rows.len(), 1);
}

#[tokio::test]
async fn test_query_table_and_or_not() {
    let d = seeded_driver().await;

    // AND: age > 25 AND age < 35
    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: Some(Filter::And(vec![
                    Filter::Gt {
                        field: "age".into(),
                        value: json!(25),
                    },
                    Filter::Lt {
                        field: "age".into(),
                        value: json!(35),
                    },
                ])),
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    // Alice (30), Diana (28)
    assert_eq!(rs.rows.len(), 2);

    // OR: name == "Alice" OR name == "Bob"
    let rs2 = d
        .query_table(
            "users",
            &TableQuery {
                filter: Some(Filter::Or(vec![
                    Filter::Eq {
                        field: "name".into(),
                        value: json!("Alice"),
                    },
                    Filter::Eq {
                        field: "name".into(),
                        value: json!("Bob"),
                    },
                ])),
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs2.rows.len(), 2);

    // NOT: not Alice
    let rs3 = d
        .query_table(
            "users",
            &TableQuery {
                filter: Some(Filter::Not(Box::new(Filter::Eq {
                    field: "name".into(),
                    value: json!("Alice"),
                }))),
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs3.rows.len(), 3);
}

#[tokio::test]
async fn test_query_table_like() {
    let d = seeded_driver().await;
    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: Some(Filter::Like {
                    field: "name".into(),
                    pattern: "Al%".into(),
                }),
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs.rows.len(), 1);
    assert_eq!(rs.rows[0]["name"], "Alice");
}

#[tokio::test]
async fn test_query_table_in() {
    let d = seeded_driver().await;
    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: Some(Filter::In {
                    field: "name".into(),
                    values: vec![json!("Alice"), json!("Charlie")],
                }),
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs.rows.len(), 2);
}

#[tokio::test]
async fn test_query_table_between() {
    let d = seeded_driver().await;
    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: Some(Filter::Between {
                    field: "age".into(),
                    low: json!(28),
                    high: json!(32),
                }),
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    // Alice (30), Diana (28)
    assert_eq!(rs.rows.len(), 2);
}

#[tokio::test]
async fn test_query_table_is_null() {
    let d = MemoryDriver::new()
        .with_table(
            "test",
            simple_schema("id"),
            vec![
                json!({"id": 1, "name": "Alice", "age": 30}),
                json!({"id": 2, "name": null, "age": 25}),
                json!({"id": 3}),
            ],
        )
        .await;

    let rs = d
        .query_table(
            "test",
            &TableQuery {
                filter: Some(Filter::IsNull {
                    field: "name".into(),
                }),
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    // id=2 (null) and id=3 (missing field treated as null)
    assert_eq!(rs.rows.len(), 2);
}

#[tokio::test]
async fn test_query_table_limit_offset() {
    let d = seeded_driver().await;
    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: None,
                columns: None,
                order_by: None,
                limit: Some(2),
                offset: Some(1),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs.rows.len(), 2);
    assert_eq!(rs.rows[0]["name"], "Bob"); // skipped Alice
}

#[tokio::test]
async fn test_query_table_order_by() {
    let d = seeded_driver().await;

    // Ascending
    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: None,
                columns: None,
                order_by: Some(vec![OrderBy {
                    column: "age".into(),
                    descending: false,
                }]),
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs.rows[0]["name"], "Bob"); // 25
    assert_eq!(rs.rows[3]["name"], "Charlie"); // 35

    // Descending
    let rs2 = d
        .query_table(
            "users",
            &TableQuery {
                filter: None,
                columns: None,
                order_by: Some(vec![OrderBy {
                    column: "age".into(),
                    descending: true,
                }]),
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs2.rows[0]["name"], "Charlie"); // 35
}

#[tokio::test]
async fn test_query_table_projection() {
    let d = seeded_driver().await;
    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: None,
                columns: Some(vec!["name".into()]),
                order_by: None,
                limit: Some(1),
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs.rows[0].as_object().unwrap().len(), 1);
    assert!(rs.rows[0].get("name").is_some());
    assert!(rs.rows[0].get("age").is_none());
}

// ── Table write ops ─────────────────────────────────────────────────

#[tokio::test]
async fn test_insert_rows() {
    let d = seeded_driver().await;
    let count = d
        .insert_rows("users", vec![json!({"id": 5, "name": "Eve", "age": 22})])
        .await
        .unwrap();
    assert_eq!(count, 1);

    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: None,
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs.rows.len(), 5);
}

#[tokio::test]
async fn test_upsert_rows() {
    let d = seeded_driver().await;

    // Upsert existing (id=1) + new (id=10)
    d.upsert_rows(
        "users",
        vec![
            json!({"id": 1, "name": "Alice Updated", "age": 31}),
            json!({"id": 10, "name": "New Person", "age": 40}),
        ],
    )
    .await
    .unwrap();

    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: Some(Filter::Eq {
                    field: "id".into(),
                    value: json!(1),
                }),
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs.rows[0]["name"], "Alice Updated");

    let all = d
        .query_table(
            "users",
            &TableQuery {
                filter: None,
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(all.rows.len(), 5); // 4 original + 1 new (1 was updated in place)
}

#[tokio::test]
async fn test_update_rows() {
    let d = seeded_driver().await;
    let count = d
        .update_rows(
            "users",
            &Filter::Eq {
                field: "name".into(),
                value: json!("Alice"),
            },
            json!({"age": 99}),
        )
        .await
        .unwrap();
    assert_eq!(count, 1);

    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: Some(Filter::Eq {
                    field: "name".into(),
                    value: json!("Alice"),
                }),
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs.rows[0]["age"], 99);
}

#[tokio::test]
async fn test_delete_rows() {
    let d = seeded_driver().await;
    let count = d
        .delete_rows(
            "users",
            &Filter::Eq {
                field: "name".into(),
                value: json!("Bob"),
            },
        )
        .await
        .unwrap();
    assert_eq!(count, 1);

    let rs = d
        .query_table(
            "users",
            &TableQuery {
                filter: None,
                columns: None,
                order_by: None,
                limit: None,
                offset: None,
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(rs.rows.len(), 3);
}

#[tokio::test]
async fn test_raw_returns_unsupported() {
    let d = MemoryDriver::new();
    assert!(d.raw("SELECT 1", json!({})).await.is_err());
}
