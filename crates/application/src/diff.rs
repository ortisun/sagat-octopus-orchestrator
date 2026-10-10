//! What differs between two workflow documents, as an editor shows it before a draft is published
//! (FR-007-004): the nodes and edges that changed, the changes outside the graph, and the two
//! lists an author acts on — what breaks the contract with callers and emitters, and what somebody
//! must prepare (a worker, a secret, a script).
//!
//! The comparison is of the documents as written, by an adapter ([`crate::definitions::IrCodec`]),
//! so a draft that does not validate can still be compared with the version it came from.

/// What one side of a comparison is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSide {
    /// A published version.
    Version(u32),
    /// The draft, at a revision.
    Draft(u64),
    /// Nothing yet: a workflow that has no version compares an empty document with its draft.
    Empty,
}

/// How a part of a document changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Added,
    Removed,
    Changed,
}

impl Change {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Changed => "changed",
        }
    }
}

/// A node present on both sides whose definition differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeChange {
    pub id: String,
    /// The top-level fields of the node that differ, sorted.
    pub fields: Vec<String>,
}

/// A link between two nodes (`next`, `onError`, `true`, `false`, `branch`, `timeout`, `begin`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiffEdge {
    pub from: String,
    pub to: String,
    pub kind: &'static str,
}

/// A difference outside the nodes: the metadata, the entrypoint, a trigger, a schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentChange {
    /// A JSON pointer into the document.
    pub path: String,
    pub change: Change,
}

/// Something an author should know before publishing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFinding {
    /// Stable, upper snake case (`TRIGGER_REMOVED`, `INPUT_REQUIRED_ADDED`, `ACTIVITY_ADDED`, ...).
    pub code: &'static str,
    /// Safe: names and paths only, never a value of the document beyond a clipped name.
    pub message: String,
    pub node: Option<String>,
    pub path: Option<String>,
}

/// The differences between two documents.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocumentDiff {
    pub identical: bool,
    pub nodes_added: Vec<String>,
    pub nodes_removed: Vec<String>,
    pub nodes_changed: Vec<NodeChange>,
    pub edges_added: Vec<DiffEdge>,
    pub edges_removed: Vec<DiffEdge>,
    pub changes: Vec<DocumentChange>,
    /// What can make a caller or an emitter of the workflow's contract fail.
    pub breaking: Vec<DiffFinding>,
    /// What somebody must prepare before the new version can run: a worker bound to a new
    /// activity, a secret, a script.
    pub attention: Vec<DiffFinding>,
}

/// The comparison of two documents of one workflow, with what each side was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionDiff {
    pub from: DiffSide,
    pub to: DiffSide,
    pub diff: DocumentDiff,
}
