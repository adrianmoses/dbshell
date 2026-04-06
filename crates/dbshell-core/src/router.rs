use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::cache::{CacheKey, CacheLayer, CachedResult};
use crate::driver::DbDriver;
use crate::error::{DbError, Result};
use crate::operation::DbOperation;
use crate::result::{ResultMetadata, ResultSet, ToolPayload};
use crate::session::CachePolicy;

pub struct QueryRouter {
    drivers: HashMap<String, Arc<dyn DbDriver>>,
}

impl QueryRouter {
    pub fn new(drivers: HashMap<String, Arc<dyn DbDriver>>) -> Self {
        QueryRouter { drivers }
    }

    pub fn get_driver(&self, name: &str) -> Result<&Arc<dyn DbDriver>> {
        self.drivers
            .get(name)
            .ok_or_else(|| DbError::NotFound(format!("driver: {name}")))
    }

    /// Dispatch a DbOperation to the correct driver and wrap the result
    /// in a ToolPayload.
    pub async fn dispatch(&self, op: &DbOperation) -> Result<ToolPayload> {
        let driver_name = op
            .driver_name()
            .ok_or(DbError::InvalidState("operation has no driver"))?;
        let driver = self.get_driver(driver_name)?;

        let start = Instant::now();

        match op {
            DbOperation::ListCollections { .. } => {
                let collections = driver.list_collections().await?;
                Ok(ToolPayload::Listing(collections))
            }

            DbOperation::InspectCollection { collection, .. } => {
                let collections = driver.list_collections().await?;
                let info = collections
                    .into_iter()
                    .find(|c| c.name == *collection)
                    .ok_or_else(|| DbError::NotFound(format!("collection: {collection}")))?;
                Ok(ToolPayload::Info(info))
            }

            DbOperation::VectorSearch {
                collection,
                request,
                ..
            } => {
                let scored = driver.vector_search(request).await?;
                let rows = scored
                    .into_iter()
                    .map(|s| {
                        serde_json::json!({
                            "id": s.id,
                            "score": s.score,
                            "payload": s.payload,
                        })
                    })
                    .collect();
                Ok(ToolPayload::Records(ResultSet {
                    rows,
                    schema: None,
                    metadata: ResultMetadata::for_query(
                        driver_name,
                        Some(collection.clone()),
                        start,
                    ),
                }))
            }

            DbOperation::Upsert {
                collection,
                records,
                ..
            } => {
                let result = driver.upsert(collection, records.clone()).await?;
                Ok(ToolPayload::Written {
                    count: result.count,
                })
            }

            DbOperation::Delete {
                collection, filter, ..
            } => {
                let count = driver.delete(collection, filter).await?;
                Ok(ToolPayload::Deleted { count })
            }

            DbOperation::GraphQuery { query, .. } => {
                let result = driver.graph_query(query).await?;
                let rows = result.rows;
                Ok(ToolPayload::Records(ResultSet {
                    rows,
                    schema: None,
                    metadata: ResultMetadata::for_query(driver_name, None, start),
                }))
            }

            DbOperation::ListTables { .. } => {
                let tables = driver.list_tables().await?;
                let rows = tables
                    .into_iter()
                    .map(|t| serde_json::to_value(t).unwrap_or_default())
                    .collect();
                Ok(ToolPayload::Records(ResultSet {
                    rows,
                    schema: None,
                    metadata: ResultMetadata::for_query(driver_name, None, start),
                }))
            }

            DbOperation::DescribeTable { table, .. } => {
                let schema = driver.describe_table(table).await?;
                let row = serde_json::to_value(&schema).unwrap_or_default();
                Ok(ToolPayload::Records(ResultSet {
                    rows: vec![row],
                    schema: None,
                    metadata: ResultMetadata::for_query(driver_name, Some(table.clone()), start),
                }))
            }

            DbOperation::QueryTable { table, request, .. } => {
                let result = driver.query_table(table, request).await?;
                Ok(ToolPayload::Records(result))
            }

            DbOperation::InsertRows { table, rows, .. } => {
                let count = driver.insert_rows(table, rows.clone()).await?;
                Ok(ToolPayload::Written { count })
            }

            DbOperation::UpsertRows { table, rows, .. } => {
                let count = driver.upsert_rows(table, rows.clone()).await?;
                Ok(ToolPayload::Written { count })
            }

            DbOperation::UpdateRows {
                table, filter, set, ..
            } => {
                let count = driver.update_rows(table, filter, set.clone()).await?;
                Ok(ToolPayload::Written { count })
            }

            DbOperation::DeleteRows { table, filter, .. } => {
                let count = driver.delete_rows(table, filter).await?;
                Ok(ToolPayload::Deleted { count })
            }

            DbOperation::MergeTable { request, .. } => {
                let result = driver.merge_tables(request).await?;
                Ok(ToolPayload::Records(result))
            }

            DbOperation::CreateCollection { spec, .. } => {
                driver.create_collection(spec).await?;
                Ok(ToolPayload::Created {
                    name: spec.name.clone(),
                })
            }

            DbOperation::DropCollection { collection, .. } => {
                driver.drop_collection(collection).await?;
                Ok(ToolPayload::Dropped {
                    name: collection.clone(),
                })
            }

            DbOperation::ReadResult { .. } | DbOperation::ListResults => Err(
                DbError::InvalidState("VFS-local operations should not reach the router"),
            ),
        }
    }
}

pub struct CachedQueryRouter {
    inner: QueryRouter,
    cache: CacheLayer,
}

impl CachedQueryRouter {
    pub fn new(drivers: HashMap<String, Arc<dyn DbDriver>>, policy: &CachePolicy) -> Self {
        CachedQueryRouter {
            inner: QueryRouter::new(drivers),
            cache: CacheLayer::new(policy),
        }
    }

    pub fn get_driver(&self, name: &str) -> Result<&Arc<dyn DbDriver>> {
        self.inner.get_driver(name)
    }

    pub async fn dispatch(&self, op: &DbOperation) -> Result<ToolPayload> {
        if op.is_write() {
            let result = self.inner.dispatch(op).await?;
            self.cache.invalidate_for_write(op);
            return Ok(result);
        }

        let key = CacheKey::from_op(op);
        if let Some(cached) = self.cache.get(&key).await {
            return Ok(cached.payload);
        }

        let result = self.inner.dispatch(op).await?;
        self.cache
            .put(
                key,
                CachedResult {
                    payload: result.clone(),
                    cached_at: Instant::now(),
                },
            )
            .await;
        Ok(result)
    }
}
