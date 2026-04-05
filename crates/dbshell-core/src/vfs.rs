use std::collections::HashMap;

use crate::error::{DbError, Result};
use crate::filter::Filter;
use crate::operation::DbOperation;
use crate::record::TableQuery;
use crate::search::VectorSearchRequest;
use crate::vfs_path::{VfsPath, VfsPathKind};
use crate::view::ViewMount;

pub struct VirtualFS {
    views: Vec<ViewMount>,
    symlinks: HashMap<String, VfsPath>,
}

impl VirtualFS {
    pub fn new() -> Self {
        VirtualFS {
            views: Vec::new(),
            symlinks: HashMap::new(),
        }
    }

    pub fn with_views(mut self, views: Vec<ViewMount>) -> Self {
        self.views = views;
        self
    }

    pub fn add_symlink(&mut self, name: String, target: VfsPath) -> Result<()> {
        if matches!(target.kind, VfsPathKind::Symlink { .. }) {
            return Err(DbError::InvalidPath("symlink chains not supported".into()));
        }
        self.symlinks.insert(name, target);
        Ok(())
    }

    pub fn remove_symlink(&mut self, name: &str) -> Result<()> {
        self.symlinks
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| DbError::NotFound(format!("symlink: {name}")))
    }

    pub fn resolve(&self, path: &VfsPath, driver: &str) -> Result<DbOperation> {
        // Resolve symlinks first (one level only)
        if let VfsPathKind::Symlink { ref name } = path.kind {
            let target = self
                .symlinks
                .get(name)
                .ok_or_else(|| DbError::NotFound(format!("symlink: {name}")))?;
            if matches!(target.kind, VfsPathKind::Symlink { .. }) {
                return Err(DbError::InvalidPath("symlink chains not supported".into()));
            }
            return self.resolve(target, driver);
        }

        let driver = driver.to_string();

        match &path.kind {
            VfsPathKind::DbRoot
            | VfsPathKind::VectorRoot
            | VfsPathKind::GraphRoot
            | VfsPathKind::GraphNodeRoot
            | VfsPathKind::GraphEdgeRoot
            | VfsPathKind::SearchRoot => Ok(DbOperation::ListCollections { driver }),

            VfsPathKind::Collection { name } => Ok(DbOperation::InspectCollection {
                driver,
                collection: name.clone(),
            }),
            VfsPathKind::GraphNode { label } => Ok(DbOperation::InspectCollection {
                driver,
                collection: label.clone(),
            }),
            VfsPathKind::GraphEdge { edge_type } => Ok(DbOperation::InspectCollection {
                driver,
                collection: edge_type.clone(),
            }),
            VfsPathKind::SearchCollection { collection } => Ok(DbOperation::InspectCollection {
                driver,
                collection: collection.clone(),
            }),

            VfsPathKind::TableRoot => Ok(DbOperation::ListTables { driver }),
            VfsPathKind::Table { name } => Ok(DbOperation::DescribeTable {
                driver,
                table: name.clone(),
            }),
            VfsPathKind::View { table, .. } => Ok(DbOperation::DescribeTable {
                driver,
                table: table.clone(),
            }),
            VfsPathKind::ViewEntry { table, view, param } => {
                let mount = self.find_view(table, view)?;
                let value = mount.cast_param(param)?;
                Ok(DbOperation::QueryTable {
                    driver,
                    table: table.clone(),
                    request: TableQuery {
                        filter: Some(Filter::Eq {
                            field: mount.filter_column.clone(),
                            value,
                        }),
                        ..Default::default()
                    },
                })
            }
            VfsPathKind::SearchQuery { collection, .. } => Ok(DbOperation::VectorSearch {
                driver,
                collection: collection.clone(),
                request: VectorSearchRequest {
                    collection: collection.clone(),
                    vector: vec![], // placeholder — Session fills after embedding
                    limit: 10,
                    filter: None,
                },
            }),
            VfsPathKind::Result { .. } | VfsPathKind::Tmp { .. } => {
                Ok(DbOperation::ReadResult { path: path.clone() })
            }
            VfsPathKind::Symlink { .. } => {
                unreachable!("symlinks handled above")
            }
        }
    }

    fn find_view(&self, table: &str, view: &str) -> Result<&ViewMount> {
        self.views
            .iter()
            .find(|v| v.table == table && v.name == view)
            .ok_or_else(|| DbError::NotFound(format!("view: {table}/{view}")))
    }
}

impl Default for VirtualFS {
    fn default() -> Self {
        Self::new()
    }
}
