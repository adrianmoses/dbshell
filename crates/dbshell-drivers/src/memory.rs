use std::collections::HashMap;

use async_trait::async_trait;
use tokio::sync::RwLock;

use dbshell_core::db_type::{DbCapability, DbType};
use dbshell_core::driver::DbDriver;
use dbshell_core::error::{DbError, Result};
use dbshell_core::filter::{cmp_json_values, matches_filter, Filter};
use dbshell_core::record::{CollectionInfo, Record, TableInfo, TableQuery, TableSchema};
use dbshell_core::result::{ResultMetadata, ResultSet};
use dbshell_core::search::{
    CollectionSpec, HealthStatus, ScoredRecord, UpsertResult, VectorSearchRequest,
};

struct MemoryCollection {
    spec: CollectionSpec,
    records: Vec<Record>,
}

struct MemoryTable {
    schema: TableSchema,
    rows: Vec<serde_json::Value>,
}

pub struct MemoryDriver {
    collections: RwLock<HashMap<String, MemoryCollection>>,
    tables: RwLock<HashMap<String, MemoryTable>>,
}

impl MemoryDriver {
    pub fn new() -> Self {
        MemoryDriver {
            collections: RwLock::new(HashMap::new()),
            tables: RwLock::new(HashMap::new()),
        }
    }

    pub async fn with_table(
        self,
        name: &str,
        schema: TableSchema,
        rows: Vec<serde_json::Value>,
    ) -> Self {
        self.tables
            .write()
            .await
            .insert(name.to_string(), MemoryTable { schema, rows });
        self
    }

    pub async fn with_collection(
        self,
        name: &str,
        spec: CollectionSpec,
        records: Vec<Record>,
    ) -> Self {
        self.collections
            .write()
            .await
            .insert(name.to_string(), MemoryCollection { spec, records });
        self
    }
}

impl Default for MemoryDriver {
    fn default() -> Self {
        Self::new()
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (dot, norm_a_sq, norm_b_sq) = a
        .iter()
        .zip(b.iter())
        .fold((0.0f32, 0.0f32, 0.0f32), |(d, na, nb), (x, y)| {
            (d + x * y, na + x * x, nb + y * y)
        });
    let norm_a = norm_a_sq.sqrt();
    let norm_b = norm_b_sq.sqrt();
    if norm_a < f32::EPSILON || norm_b < f32::EPSILON {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[async_trait]
impl DbDriver for MemoryDriver {
    fn name(&self) -> &str {
        "memory"
    }

    fn db_type(&self) -> DbType {
        DbType::Hybrid(vec![DbCapability::Vector, DbCapability::Relational])
    }

    async fn health(&self) -> Result<HealthStatus> {
        Ok(HealthStatus {
            healthy: true,
            message: None,
        })
    }

    async fn list_collections(&self) -> Result<Vec<CollectionInfo>> {
        let colls = self.collections.read().await;
        Ok(colls
            .iter()
            .map(|(name, c)| CollectionInfo {
                name: name.clone(),
                driver: "memory".into(),
                db_type: DbType::Vector,
                record_count: c.records.len() as u64,
                dimensions: Some(c.spec.dimensions),
                distance_metric: Some(c.spec.distance_metric.clone()),
                node_labels: None,
                edge_types: None,
                properties: None,
                primary_key: None,
                columns: None,
                foreign_keys: None,
                constraints: None,
                indexes: None,
            })
            .collect())
    }

    async fn create_collection(&self, spec: &CollectionSpec) -> Result<()> {
        let mut colls = self.collections.write().await;
        if colls.contains_key(&spec.name) {
            return Err(DbError::DriverError(
                format!("collection '{}' already exists", spec.name).into(),
            ));
        }
        colls.insert(
            spec.name.clone(),
            MemoryCollection {
                spec: spec.clone(),
                records: Vec::new(),
            },
        );
        Ok(())
    }

    async fn drop_collection(&self, name: &str) -> Result<()> {
        let mut colls = self.collections.write().await;
        colls
            .remove(name)
            .ok_or_else(|| DbError::NotFound(format!("collection: {name}")))?;
        Ok(())
    }

    async fn upsert(&self, collection: &str, records: Vec<Record>) -> Result<UpsertResult> {
        let mut colls = self.collections.write().await;
        let coll = colls
            .get_mut(collection)
            .ok_or_else(|| DbError::NotFound(format!("collection: {collection}")))?;

        let count = records.len() as u64;
        for rec in records {
            if let Some(existing) = coll.records.iter_mut().find(|r| r.id == rec.id) {
                *existing = rec;
            } else {
                coll.records.push(rec);
            }
        }
        Ok(UpsertResult { count })
    }

    async fn vector_search(&self, req: &VectorSearchRequest) -> Result<Vec<ScoredRecord>> {
        let colls = self.collections.read().await;
        let coll = colls
            .get(&req.collection)
            .ok_or_else(|| DbError::NotFound(format!("collection: {}", req.collection)))?;

        let limit = req.limit as usize;

        // Score all records but only keep top-k by maintaining a sorted vec
        // of (score, index) pairs, avoiding cloning payloads for non-winners.
        let mut top: Vec<(f32, usize)> = Vec::with_capacity(limit + 1);

        for (idx, r) in coll.records.iter().enumerate() {
            if let Some(ref f) = req.filter {
                if !matches_filter(&r.payload, f) {
                    continue;
                }
            }
            let score = match r.vector.as_ref() {
                Some(vec) => cosine_similarity(vec, &req.vector),
                None => continue,
            };

            top.push((score, idx));
            // Keep sorted descending; evict the lowest when over limit
            if top.len() > limit {
                top.sort_unstable_by(|a, b| {
                    b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)
                });
                top.truncate(limit);
            }
        }

        top.sort_unstable_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        Ok(top
            .into_iter()
            .map(|(score, idx)| {
                let r = &coll.records[idx];
                ScoredRecord {
                    id: r.id.clone(),
                    score,
                    payload: r.payload.clone(),
                }
            })
            .collect())
    }

    async fn delete(&self, collection: &str, filter: &Filter) -> Result<u64> {
        let mut colls = self.collections.write().await;
        let coll = colls
            .get_mut(collection)
            .ok_or_else(|| DbError::NotFound(format!("collection: {collection}")))?;

        let before = coll.records.len();
        coll.records.retain(|r| !matches_filter(&r.payload, filter));
        Ok((before - coll.records.len()) as u64)
    }

    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        let tables = self.tables.read().await;
        Ok(tables
            .iter()
            .map(|(name, t)| TableInfo {
                name: name.clone(),
                driver: "memory".into(),
                row_count: Some(t.rows.len() as u64),
                schema_name: None,
            })
            .collect())
    }

    async fn describe_table(&self, name: &str) -> Result<TableSchema> {
        let tables = self.tables.read().await;
        let table = tables
            .get(name)
            .ok_or_else(|| DbError::NotFound(format!("table: {name}")))?;
        Ok(table.schema.clone())
    }

    async fn query_table(&self, table: &str, req: &TableQuery) -> Result<ResultSet> {
        let tables = self.tables.read().await;
        let t = tables
            .get(table)
            .ok_or_else(|| DbError::NotFound(format!("table: {table}")))?;

        let mut rows: Vec<serde_json::Value> = t
            .rows
            .iter()
            .filter(|r| req.filter.as_ref().is_none_or(|f| matches_filter(r, f)))
            .cloned()
            .collect();

        if let Some(ref order_by) = req.order_by {
            for ob in order_by.iter().rev() {
                rows.sort_by(|a, b| {
                    let cmp = cmp_values(a.get(&ob.column), b.get(&ob.column));
                    if ob.descending {
                        cmp.reverse()
                    } else {
                        cmp
                    }
                });
            }
        }

        let total_count = rows.len() as u64;

        if let Some(offset) = req.offset {
            let skip = offset as usize;
            if skip < rows.len() {
                rows.drain(..skip);
            } else {
                rows.clear();
            }
        }

        if let Some(limit) = req.limit {
            rows.truncate(limit as usize);
        }

        if let Some(ref cols) = req.columns {
            rows = rows
                .into_iter()
                .map(|row| project_row(&row, cols))
                .collect();
        }

        Ok(ResultSet {
            rows,
            schema: None,
            metadata: ResultMetadata {
                driver: "memory".into(),
                collection: Some(table.into()),
                total_count: Some(total_count),
                query_ms: 0,
                cache_hit: false,
                next_cursor: None,
            },
        })
    }

    async fn insert_rows(&self, table: &str, rows: Vec<serde_json::Value>) -> Result<u64> {
        let mut tables = self.tables.write().await;
        let t = tables
            .get_mut(table)
            .ok_or_else(|| DbError::NotFound(format!("table: {table}")))?;
        let count = rows.len() as u64;
        t.rows.extend(rows);
        Ok(count)
    }

    async fn upsert_rows(&self, table: &str, rows: Vec<serde_json::Value>) -> Result<u64> {
        let mut tables = self.tables.write().await;
        let t = tables
            .get_mut(table)
            .ok_or_else(|| DbError::NotFound(format!("table: {table}")))?;
        let count = rows.len() as u64;
        let pk = t.schema.primary_key.clone().unwrap_or_default();

        for new_row in rows {
            if let Some(existing) = find_row_by_pk(&mut t.rows, &pk, &new_row) {
                *existing = new_row;
            } else {
                t.rows.push(new_row);
            }
        }
        Ok(count)
    }

    async fn update_rows(
        &self,
        table: &str,
        filter: &Filter,
        set: serde_json::Value,
    ) -> Result<u64> {
        let mut tables = self.tables.write().await;
        let t = tables
            .get_mut(table)
            .ok_or_else(|| DbError::NotFound(format!("table: {table}")))?;

        let mut count = 0u64;
        for row in &mut t.rows {
            if matches_filter(row, filter) {
                if let (Some(row_obj), Some(set_obj)) = (row.as_object_mut(), set.as_object()) {
                    for (k, v) in set_obj {
                        row_obj.insert(k.clone(), v.clone());
                    }
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    async fn delete_rows(&self, table: &str, filter: &Filter) -> Result<u64> {
        let mut tables = self.tables.write().await;
        let t = tables
            .get_mut(table)
            .ok_or_else(|| DbError::NotFound(format!("table: {table}")))?;

        let before = t.rows.len();
        t.rows.retain(|r| !matches_filter(r, filter));
        Ok((before - t.rows.len()) as u64)
    }

    async fn raw(&self, _query: &str, _params: serde_json::Value) -> Result<serde_json::Value> {
        Err(DbError::Unsupported("raw queries on memory driver"))
    }
}

fn project_row(row: &serde_json::Value, cols: &[String]) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    if let Some(row_obj) = row.as_object() {
        for col in cols {
            if let Some(v) = row_obj.get(col) {
                obj.insert(col.clone(), v.clone());
            }
        }
    }
    serde_json::Value::Object(obj)
}

fn cmp_values(a: Option<&serde_json::Value>, b: Option<&serde_json::Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a), Some(b)) => cmp_json_values(a, b),
    }
}

fn find_row_by_pk<'a>(
    rows: &'a mut [serde_json::Value],
    pk_cols: &[String],
    new_row: &serde_json::Value,
) -> Option<&'a mut serde_json::Value> {
    if pk_cols.is_empty() {
        return None;
    }
    rows.iter_mut().find(|existing| {
        pk_cols.iter().all(|col| {
            let a = existing.get(col);
            let b = new_row.get(col);
            a.is_some() && a == b
        })
    })
}
