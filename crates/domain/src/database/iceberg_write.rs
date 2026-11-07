//! The `icebergWrite` workflow node (feature 035, T035-052; `contracts/workflow-lake.schema.json`, data-model.md §2).
//!
//! The shape of the step, its bounds and nothing that touches a catalog: whether the connection exists, whether its modes and
//! its ceiling allow this write, and whether the lake is enabled are publication and admission (T035-054, T035-055).
//!
//! Only the append mode is built. A step that overwrites a table or upserts into it needs options of its own (a scope and a
//! filter, key columns and a delete marker) and the tests that go with them: they are added to [`WriteIntent`] with user
//! stories 4 and 5 (T035-080, T035-085). Until then there is no way to say them, rather than a way to say them untested.

use super::{
    ConnectionRef, EgressField, EgressRefusal, IcebergConfig, LakeLimits, LakeRefusal, WriteMode,
};
use crate::{
    classification::EgressCeiling,
    workflow::{Expression, NodeId},
};

/// The reserved activity that writes. Only an [`IcebergWrite`] node may carry it.
pub const ICEBERG_WRITE_ACTIVITY: &str = "sagat.database.iceberg.write";
/// The reserved activity of the derived compensation of a step that declares `compensation: revert`.
pub const ICEBERG_REVERT_ACTIVITY: &str = "sagat.database.iceberg.revert";
/// Mapped columns: the hard ceiling (the platform default is lower and is a publication setting).
pub const MAX_MAPPED_COLUMNS: usize = 1024;
/// Nesting depth of a mapped field, in JSON pointer segments: the hard ceiling.
pub const MAX_MAPPING_DEPTH: usize = 16;
/// Record blobs one step names. The runner and publication enforce it where the blob descriptors are known.
pub const MAX_RECORD_BLOBS: usize = 32;
const MAX_FIELD_BYTES: usize = 512;
const MAX_NAME_BYTES: usize = 255;
const MAX_NAMESPACE_LEVELS: usize = 8;

/// Why a step is not well formed. The reason names the part, never its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IcebergWriteError {
    #[error(
        "the table is not 1 to 8 namespace levels and a name of 1 to 255 bytes without control characters"
    )]
    Table,
    #[error("the target ref is not 1 to 255 bytes without control characters")]
    Reference,
    #[error("the connection revision must be positive")]
    Connection,
    #[error("a mapped field is not a JSON pointer of at most 512 bytes")]
    Field,
    #[error("a mapped column is not a name of 1 to 255 bytes without control characters")]
    Column,
    #[error("the mapping names more columns than the ceiling")]
    TooManyColumns,
    #[error("a mapped field is nested deeper than the ceiling")]
    MappingTooDeep,
    #[error("a field is mapped twice")]
    DuplicateField,
    #[error("a column is mapped twice")]
    DuplicateColumn,
    #[error("a limit of the step is outside its range")]
    Limits,
}

fn plain(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_NAME_BYTES && !text.chars().any(char::is_control)
}

/// A namespace level or a table name. `.` and `..` are refused: an HTTP client removes them from the path of a request
/// (RFC 3986, 5.2.4), so the catalog would be asked something else than the step says. The lake host refuses them too.
fn identifier_part(text: &str) -> bool {
    plain(text) && text != "." && text != ".."
}

/// RFC 6901: a `~` is the start of `~0` (a `~`) or `~1` (a `/`) and nothing else. The lake host refuses any other use, so
/// the domain refuses it where the step is written and not when it runs.
fn escapes_are_pointer_escapes(pointer: &str) -> bool {
    let mut after = pointer.split('~');
    after.next();
    after.all(|rest| rest.starts_with(['0', '1']))
}

/// An Iceberg table: one to eight namespace levels and a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableIdentifier {
    namespace: Vec<String>,
    name: String,
}

impl TableIdentifier {
    /// # Errors
    /// [`IcebergWriteError::Table`] for no level, more than eight, or a level or name that is empty, longer than 255 bytes,
    /// carries a control character, or is `.` or `..`.
    pub fn new(namespace: Vec<String>, name: &str) -> Result<Self, IcebergWriteError> {
        if namespace.is_empty()
            || namespace.len() > MAX_NAMESPACE_LEVELS
            || !namespace.iter().all(|level| identifier_part(level))
            || !identifier_part(name)
        {
            return Err(IcebergWriteError::Table);
        }
        Ok(Self {
            namespace,
            name: name.to_owned(),
        })
    }

    #[must_use]
    pub fn namespace(&self) -> &[String] {
        &self.namespace
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    fn footprint(&self) -> usize {
        self.namespace.iter().map(String::len).sum::<usize>() + self.name.len()
    }
}

/// Where the records of a step come from: exactly one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordSource {
    /// An inline list of records, evaluated from workflow values.
    Inline(Expression),
    /// Up to [`MAX_RECORD_BLOBS`] record blobs that earlier steps of the same run committed, named by an expression.
    Blobs(Expression),
    /// The result of an earlier root-level database step of the same run.
    DatabaseResult(NodeId),
}

/// One field of a record and the column it is written to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMapping {
    field: String,
    column: String,
    depth: usize,
}

impl ColumnMapping {
    /// # Errors
    /// [`IcebergWriteError::Field`] when `field` is not a JSON pointer (`/a/b`, no empty segment) within 512 bytes,
    /// [`IcebergWriteError::MappingTooDeep`] when it has more than [`MAX_MAPPING_DEPTH`] segments, and
    /// [`IcebergWriteError::Column`] when `column` is not a name of 1 to 255 bytes without control characters.
    pub fn new(field: &str, column: &str) -> Result<Self, IcebergWriteError> {
        let Some(path) = field.strip_prefix('/') else {
            return Err(IcebergWriteError::Field);
        };
        let segments = path.split('/').collect::<Vec<_>>();
        if field.len() > MAX_FIELD_BYTES
            || segments.iter().any(|segment| segment.is_empty())
            || field.chars().any(char::is_control)
            || !escapes_are_pointer_escapes(field)
        {
            return Err(IcebergWriteError::Field);
        }
        if segments.len() > MAX_MAPPING_DEPTH {
            return Err(IcebergWriteError::MappingTooDeep);
        }
        if !plain(column) {
            return Err(IcebergWriteError::Column);
        }
        Ok(Self {
            field: field.to_owned(),
            column: column.to_owned(),
            depth: segments.len(),
        })
    }

    #[must_use]
    pub fn field(&self) -> &str {
        &self.field
    }

    #[must_use]
    pub fn column(&self) -> &str {
        &self.column
    }

    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }
}

/// How a step changes the table. Overwrite and upsert are added with their user stories (T035-080, T035-085).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteIntent {
    Append,
}

impl WriteIntent {
    /// The write mode a connection has to allow for a step with this intent.
    #[must_use]
    pub const fn mode(self) -> WriteMode {
        match self {
            Self::Append => WriteMode::Append,
        }
    }
}

/// What an author says about one `icebergWrite` step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcebergWriteParts {
    pub connection: ConnectionRef,
    pub table: TableIdentifier,
    /// The branch or tag to commit to; `main` when the author says nothing.
    pub reference: String,
    pub intent: WriteIntent,
    pub records: RecordSource,
    pub mapping: Vec<ColumnMapping>,
    /// Drop record fields that have no column instead of refusing the batch.
    pub ignore_unknown_fields: bool,
    pub limits: LakeLimits,
    /// `compensation: revert`: the step is undone by removing its commit.
    pub revert: bool,
}

/// A well-formed `icebergWrite` step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcebergWrite {
    parts: IcebergWriteParts,
    /// The level of each field the step writes, once someone who read the schemas of its source has set it. Not what the
    /// author wrote: it is derived, so it is not part of the canonical document.
    egress: Option<Vec<EgressField>>,
}

impl IcebergWrite {
    /// # Errors
    /// [`IcebergWriteError`] for the first part that is out of bounds.
    pub fn new(parts: IcebergWriteParts) -> Result<Self, IcebergWriteError> {
        let write = Self {
            parts,
            egress: None,
        };
        write.validate()?;
        Ok(write)
    }

    /// Gives the step the level of each field it writes (FR-035-025). Replaces any list it had.
    #[must_use]
    pub fn with_egress(mut self, fields: Vec<EgressField>) -> Self {
        self.egress = Some(fields);
        self
    }

    /// The level of each field the step writes, or `None` when nobody has classified them.
    #[must_use]
    pub fn egress(&self) -> Option<&[EgressField]> {
        self.egress.as_deref()
    }

    /// What the settings of a connection refuse about this step, if anything: first a mode it does not allow, and only
    /// then the fields above its ceiling, so a mode refusal never lists fields. A step whose fields were never classified
    /// is refused whole.
    #[must_use]
    pub fn refusal_by(&self, config: &IcebergConfig) -> Option<LakeRefusal> {
        let mode = self.parts.intent.mode();
        if !config.allowed_modes.contains(&mode) {
            return Some(LakeRefusal::ModeNotAllowed(mode));
        }
        let refusals = self.egress_refusals(config.egress_ceiling);
        (!refusals.is_empty()).then_some(LakeRefusal::Egress(refusals))
    }

    /// Every field that may not leave through a connection whose ceiling is `ceiling`. A step whose fields were never
    /// classified is refused whole.
    #[must_use]
    pub fn egress_refusals(&self, ceiling: EgressCeiling) -> Vec<EgressRefusal> {
        super::iceberg_egress::refusals(self.egress(), ceiling)
    }

    /// Checks everything again: the parts are public until they are wrapped, and a definition does not trust a step it
    /// did not just build.
    ///
    /// # Errors
    /// [`IcebergWriteError`] for the first part that is out of bounds.
    pub fn validate(&self) -> Result<(), IcebergWriteError> {
        let parts = &self.parts;
        if parts.connection.revision == Some(0) {
            return Err(IcebergWriteError::Connection);
        }
        if !plain(&parts.reference) {
            return Err(IcebergWriteError::Reference);
        }
        if parts.mapping.len() > MAX_MAPPED_COLUMNS {
            return Err(IcebergWriteError::TooManyColumns);
        }
        let mut fields = std::collections::BTreeSet::new();
        let mut columns = std::collections::BTreeSet::new();
        for mapping in &parts.mapping {
            if !fields.insert(mapping.field.as_str()) {
                return Err(IcebergWriteError::DuplicateField);
            }
            if !columns.insert(mapping.column.as_str()) {
                return Err(IcebergWriteError::DuplicateColumn);
            }
            if mapping.depth > MAX_MAPPING_DEPTH {
                return Err(IcebergWriteError::MappingTooDeep);
            }
        }
        parts
            .limits
            .validate()
            .map_err(|_| IcebergWriteError::Limits)
    }

    #[must_use]
    pub const fn connection(&self) -> &ConnectionRef {
        &self.parts.connection
    }

    #[must_use]
    pub const fn table(&self) -> &TableIdentifier {
        &self.parts.table
    }

    #[must_use]
    pub fn reference(&self) -> &str {
        &self.parts.reference
    }

    #[must_use]
    pub const fn intent(&self) -> WriteIntent {
        self.parts.intent
    }

    #[must_use]
    pub const fn records(&self) -> &RecordSource {
        &self.parts.records
    }

    #[must_use]
    pub fn mapping(&self) -> &[ColumnMapping] {
        &self.parts.mapping
    }

    #[must_use]
    pub const fn ignore_unknown_fields(&self) -> bool {
        self.parts.ignore_unknown_fields
    }

    #[must_use]
    pub const fn limits(&self) -> LakeLimits {
        self.parts.limits
    }

    /// Whether the step declares `compensation: revert`.
    #[must_use]
    pub const fn revert(&self) -> bool {
        self.parts.revert
    }

    /// What the step holds in memory, for the weight of a definition.
    #[must_use]
    pub fn footprint(&self) -> usize {
        let records = match &self.parts.records {
            RecordSource::Inline(expression) | RecordSource::Blobs(expression) => {
                expression.as_str().len()
            }
            RecordSource::DatabaseResult(node) => node.as_str().len(),
        };
        self.parts.connection.name.as_str().len()
            + self.parts.table.footprint()
            + self.parts.reference.len()
            + records
            + self
                .parts
                .mapping
                .iter()
                .map(|mapping| {
                    std::mem::size_of::<ColumnMapping>()
                        + mapping.field.len()
                        + mapping.column.len()
                })
                .sum::<usize>()
    }
}
