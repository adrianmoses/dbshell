use std::collections::HashMap;

use crate::vfs_path::VfsPath;

/// Tool identity for VirtualFS::resolve(). The same path produces different
/// DbOperations depending on which tool is invoked.
///
/// Phase 4 adds full argv parsing in `dbshell-tools`; Phase 2 constructs
/// ToolCall values programmatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolKind {
    Ls,
    Cat,
    Find,
    Grep,
    Filter,
    Head,
    Tail,
    Wc,
    Sort,
    Echo,
    Rm,
    Ln,
    Merge,
    Begin,
    Commit,
    Rollback,
    Man,
}

impl ToolKind {
    /// Parse a tool name string into a ToolKind.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "ls" => Some(Self::Ls),
            "cat" => Some(Self::Cat),
            "find" => Some(Self::Find),
            "grep" => Some(Self::Grep),
            "filter" => Some(Self::Filter),
            "head" => Some(Self::Head),
            "tail" => Some(Self::Tail),
            "wc" => Some(Self::Wc),
            "sort" => Some(Self::Sort),
            "echo" => Some(Self::Echo),
            "rm" => Some(Self::Rm),
            "ln" => Some(Self::Ln),
            "merge" => Some(Self::Merge),
            "begin" => Some(Self::Begin),
            "commit" => Some(Self::Commit),
            "rollback" => Some(Self::Rollback),
            "man" => Some(Self::Man),
            _ => None,
        }
    }

    /// Whether this tool is a transaction control keyword.
    pub fn is_transaction_control(&self) -> bool {
        matches!(self, Self::Begin | Self::Commit | Self::Rollback)
    }
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub kind: ToolKind,
    pub path: Option<VfsPath>,
    pub args: ToolArgs,
    pub stdin: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ToolArgs {
    pub flags: HashMap<String, String>,
    pub positional: Vec<String>,
}
