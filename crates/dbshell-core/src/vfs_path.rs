use crate::error::{DbError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResultId(pub String);

#[derive(Debug, Clone, PartialEq)]
pub struct VfsPath {
    pub raw: String,
    pub kind: VfsPathKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VfsPathKind {
    DbRoot,
    VectorRoot,
    Collection {
        name: String,
    },
    GraphRoot,
    GraphNodeRoot,
    GraphEdgeRoot,
    GraphNode {
        label: String,
    },
    GraphEdge {
        edge_type: String,
    },
    TableRoot,
    Table {
        name: String,
    },
    View {
        table: String,
        view: String,
    },
    ViewEntry {
        table: String,
        view: String,
        param: String,
    },
    Symlink {
        name: String,
    },
    SearchRoot,
    SearchCollection {
        collection: String,
    },
    SearchQuery {
        collection: String,
        query: String,
    },
    Result {
        id: ResultId,
    },
    Tmp {
        name: String,
    },
}

impl VfsPath {
    pub fn parse(input: &str) -> Result<VfsPath> {
        let input = input.trim();
        if input.is_empty() {
            return Err(DbError::InvalidPath("empty path".into()));
        }
        if !input.starts_with('/') {
            return Err(DbError::InvalidPath(format!(
                "path must start with '/': {input}"
            )));
        }

        // Strip leading slash, then strip trailing slash for normalization.
        let trimmed = input.strip_prefix('/').unwrap();
        let trimmed = trimmed.strip_suffix('/').unwrap_or(trimmed);

        if trimmed.is_empty() {
            return Err(DbError::InvalidPath("bare '/' is not a valid path".into()));
        }

        let segments: Vec<&str> = trimmed.splitn(2, '/').collect();
        let namespace = segments[0];
        let rest = segments.get(1).copied().unwrap_or("");

        let kind = match namespace {
            "db" => Self::parse_db(rest)?,
            "search" => Self::parse_search(rest)?,
            "results" => Self::parse_results(rest)?,
            "tmp" => Self::parse_tmp(rest)?,
            "links" => Self::parse_links(rest)?,
            _ => {
                return Err(DbError::InvalidPath(format!(
                    "unknown namespace: /{namespace}"
                )))
            }
        };

        Ok(VfsPath {
            raw: input.to_string(),
            kind,
        })
    }

    fn parse_db(rest: &str) -> Result<VfsPathKind> {
        if rest.is_empty() {
            return Ok(VfsPathKind::DbRoot);
        }

        let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();

        match segments.as_slice() {
            ["vectors"] => Ok(VfsPathKind::VectorRoot),
            ["vectors", name] => Ok(VfsPathKind::Collection {
                name: (*name).to_string(),
            }),
            ["graphs"] => Ok(VfsPathKind::GraphRoot),
            ["graphs", "nodes"] => Ok(VfsPathKind::GraphNodeRoot),
            ["graphs", "edges"] => Ok(VfsPathKind::GraphEdgeRoot),
            ["graphs", "nodes", label] => Ok(VfsPathKind::GraphNode {
                label: (*label).to_string(),
            }),
            ["graphs", "edges", edge_type] => Ok(VfsPathKind::GraphEdge {
                edge_type: (*edge_type).to_string(),
            }),
            ["tables"] => Ok(VfsPathKind::TableRoot),
            ["tables", name] => Ok(VfsPathKind::Table {
                name: (*name).to_string(),
            }),
            ["tables", table, view] => Ok(VfsPathKind::View {
                table: (*table).to_string(),
                view: (*view).to_string(),
            }),
            ["tables", table, view, param] => Ok(VfsPathKind::ViewEntry {
                table: (*table).to_string(),
                view: (*view).to_string(),
                param: (*param).to_string(),
            }),
            _ => Err(DbError::InvalidPath(format!("invalid db path: /db/{rest}"))),
        }
    }

    fn parse_search(rest: &str) -> Result<VfsPathKind> {
        if rest.is_empty() {
            return Ok(VfsPathKind::SearchRoot);
        }

        // The query is everything after /search/<collection>/, preserving
        // slashes and spaces as raw text.
        let rest = rest.strip_suffix('/').unwrap_or(rest);

        if let Some(slash_pos) = rest.find('/') {
            let collection = &rest[..slash_pos];
            let query = &rest[slash_pos + 1..];
            if query.is_empty() {
                Ok(VfsPathKind::SearchCollection {
                    collection: collection.to_string(),
                })
            } else {
                Ok(VfsPathKind::SearchQuery {
                    collection: collection.to_string(),
                    query: query.to_string(),
                })
            }
        } else {
            Ok(VfsPathKind::SearchCollection {
                collection: rest.to_string(),
            })
        }
    }

    fn parse_results(rest: &str) -> Result<VfsPathKind> {
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.is_empty() {
            return Err(DbError::InvalidPath(
                "/results/ requires an id (e.g. /results/last)".into(),
            ));
        }
        Ok(VfsPathKind::Result {
            id: ResultId(rest.to_string()),
        })
    }

    fn parse_tmp(rest: &str) -> Result<VfsPathKind> {
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.is_empty() {
            return Err(DbError::InvalidPath(
                "/tmp/ requires a name (e.g. /tmp/scratch)".into(),
            ));
        }
        Ok(VfsPathKind::Tmp {
            name: rest.to_string(),
        })
    }

    fn parse_links(rest: &str) -> Result<VfsPathKind> {
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.is_empty() {
            return Err(DbError::InvalidPath(
                "/links/ requires a name (e.g. /links/my-alias)".into(),
            ));
        }
        Ok(VfsPathKind::Symlink {
            name: rest.to_string(),
        })
    }
}
