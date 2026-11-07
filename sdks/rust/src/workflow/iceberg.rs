use super::*;
/// A table of an Iceberg catalog: a namespace of 1 to 8 levels and a name.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IcebergTable {
    pub namespace: Vec<String>,
    pub name: String,
}
/// Where the records of the write come from: exactly one of an expression that evaluates to a list of objects, one that
/// evaluates to 1 to 32 blob descriptors, or the id of an earlier root-level database step.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IcebergRecords {
    Inline(String),
    Blobs(String),
    DatabaseResult(String),
}
/// A field of a record (a JSON pointer, `/customer/name`) and the column it is written to.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IcebergMapping {
    pub field: String,
    pub column: String,
}
/// Optional tighter limits on one write; an empty set is kept as an empty object.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IcebergLimits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_record_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_records: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}
/// The undo of a write: removing its commit.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IcebergCompensation {
    Revert,
}
/// An append to an Iceberg table through the isolated Sagat lake runner. Only a scoped logical connection name and
/// revision appear in a workflow: no catalog, bucket, credential or secret. The mode is append; overwrite and upsert are
/// not built yet.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IcebergWriteOptions {
    pub connection_ref: ConnectionRef,
    pub table: IcebergTable,
    pub records: IcebergRecords,
    /// The branch or tag to commit to; `main` when absent.
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping: Option<Vec<IcebergMapping>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_unknown_fields: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<IcebergLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compensation: Option<IcebergCompensation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_error: Option<String>,
}
/// 1 to `limit` bytes of UTF-8 without a control character: the platform's plain text.
fn plain(text: &str, limit: usize) -> bool {
    !text.is_empty() && text.len() <= limit && !text.chars().any(char::is_control)
}
/// A level or a name of a table: plain, and not a path component of its own.
fn part(text: &str) -> bool {
    plain(text, 255) && text != "." && text != ".."
}
/// A JSON pointer into a record whose escapes are `~0` and `~1`, of at most 16 segments.
fn pointer(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('/') else {
        return false;
    };
    plain(text, 512)
        && rest.split('/').count() <= 16
        && rest.split('/').all(|segment| !segment.is_empty())
        && text
            .match_indices('~')
            .all(|(at, _)| matches!(text.as_bytes().get(at + 1), Some(b'0' | b'1')))
}
fn within(value: Option<u64>, low: u64, high: u64) -> bool {
    value.is_none_or(|v| (low..=high).contains(&v))
}
fn valid(options: &IcebergWriteOptions) -> bool {
    let table = &options.table;
    let records = match &options.records {
        // an expression of 1 to 1024 UTF-8 bytes, as everywhere else: the schema says 4096, the engine takes 1024
        IcebergRecords::Inline(text) | IcebergRecords::Blobs(text) => {
            (1..=1024).contains(&text.len())
        }
        IcebergRecords::DatabaseResult(step) => plain(step, 64),
    };
    let mapping = options.mapping.as_deref().unwrap_or_default();
    let mut fields = BTreeSet::new();
    let mut columns = BTreeSet::new();
    // a field or a column twice is two rows of the mapping that disagree about one thing
    let rows = mapping.len() <= 1024
        && mapping.iter().all(|pair| {
            pointer(&pair.field)
                && plain(&pair.column, 255)
                && fields.insert(pair.field.as_str())
                && columns.insert(pair.column.as_str())
        });
    let limits = options.limits.as_ref().is_none_or(|limits| {
        within(limits.max_record_bytes, 1024, 2_147_483_648)
            && within(limits.max_records, 1, 100_000_000)
            && within(limits.timeout_ms, 1000, 3_600_000)
    });
    valid_connection_ref(&options.connection_ref)
        && (1..=8).contains(&table.namespace.len())
        && table.namespace.iter().all(|level| part(level))
        && part(&table.name)
        && options.reference.as_deref().is_none_or(|r| plain(r, 255))
        && records
        && rows
        && limits
        && options
            .on_error
            .as_ref()
            .is_none_or(|route| !route.is_empty())
}
impl Builder {
    /// Declare an append to an Iceberg table. Only the lake runner resolves the connection and writes; shape validation
    /// only here: the catalog, access, modes and classification belong to the server.
    pub fn iceberg_write(&self, id: &str, options: IcebergWriteOptions) -> &Self {
        if !valid(&options) {
            return self.fail("invalid iceberg write");
        }
        self.options(
            json!({"id":id,"type":"icebergWrite","mode":"append"}),
            options,
        )
    }
}
