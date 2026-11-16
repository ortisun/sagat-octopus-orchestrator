//! `control-plane input-schema-inventory` (design 043, T043-24, FR-043-018, D043-052, D043-072): the stored workflow
//! versions whose `inputSchema` is outside the supported subset, so an operator can see before the upgrade which
//! versions will refuse every start in `enforce` mode, and fix them or choose `report` first.
//!
//! It runs with the database settings of the control plane (the runtime role where a deployment sets one): it only
//! reads, and needs nothing but `SELECT` on `workflow_versions`. It reads them page by page and runs on each
//! stored `inputSchema` the pre-check that publication and the start gate run. It prints one line per version that
//! would be refused (its ids, the issue, the keyword and its pointer), and, for information, one line per version
//! within the subset that has an `inputSchema` and a trigger other than a manual start, whose triggered runs may be
//! refused (G043-004). It prints no schema text and no input, and writes nothing. Its exit status is 0 when no version
//! would be refused, 1 when at least one would, and 2 when the command itself failed.

use std::fmt;
use std::time::Duration;

use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::InputSchemaIssue;
use sagat_infra_ir::stored_input_schema;
use sagat_infra_postgres::stored_versions_page;

use crate::config::DatabaseConfig;
use crate::handover::shown;
use crate::{RunError, open_pool};

/// Versions read per page: a document is at most 1 MiB, so a page holds at most this many MiB.
const PAGE: u32 = 64;

/// The ids of one stored version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionIds {
    pub tenant: TenantId,
    pub namespace: NamespaceId,
    pub workflow: WorkflowId,
    pub version: WorkflowVersionId,
    pub number: i32,
}

impl fmt::Display for VersionIds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "tenant={} namespace={} workflow={} version={} number={}",
            self.tenant, self.namespace, self.workflow, self.version, self.number
        )
    }
}

/// A version every start of which would be refused in `enforce` mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedVersion {
    pub ids: VersionIds,
    pub issue: InputSchemaIssue,
    /// The keyword refused, from the schema (author vocabulary).
    pub keyword: String,
    /// Its JSON pointer in the document (`/inputSchema/...`).
    pub at: String,
}

/// What the inventory found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    /// The versions that would be refused, in id order.
    pub refused: Vec<RefusedVersion>,
    /// The versions within the subset with an `inputSchema` and a non-manual trigger: information only.
    pub triggered: Vec<VersionIds>,
    /// The versions read, all of them.
    pub scanned: u64,
    /// The stored documents that are not JSON objects (none should be): listed, never a reason to stop.
    pub unreadable: Vec<VersionIds>,
}

impl Inventory {
    /// 0 when no version would be refused, 1 when at least one would (D043-072), and 2 when a stored document could
    /// not be read, because the command cannot say what that version would do (D043-111).
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        if !self.unreadable.is_empty() {
            2
        } else {
            u8::from(!self.refused.is_empty())
        }
    }
}

/// The exit status of the command for what it did: the inventory's own, or 2 when the command failed.
#[must_use]
pub fn exit_code(outcome: &Result<Inventory, RunError>) -> u8 {
    match outcome {
        Ok(inventory) => inventory.exit_code(),
        Err(_) => 2,
    }
}

impl fmt::Display for Inventory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for refused in &self.refused {
            writeln!(
                f,
                "refused {} issue={} keyword={} at={}",
                refused.ids,
                refused.issue.as_str(),
                quoted(&refused.keyword),
                quoted(&refused.at)
            )?;
        }
        for ids in &self.triggered {
            writeln!(
                f,
                "information {ids} the inputSchema is applied to the event envelope of its triggers"
            )?;
        }
        for ids in &self.unreadable {
            writeln!(
                f,
                "unreadable {ids} the stored document is not a JSON object: what this version would do is unknown"
            )?;
        }
        write!(
            f,
            "{} stored version(s) read: {} would refuse every start in enforce mode, {} with a trigger are listed for \
             information. Nothing was changed.",
            self.scanned,
            self.refused.len(),
            self.triggered.len()
        )
    }
}

/// A field of the schema as it is printed: in double quotes, with `"` and `\` escaped and every character that could
/// rewrite the line shown as `?` (D043-111), so a keyword or a pointer cannot pass for another field.
fn quoted(text: &str) -> String {
    format!(
        "\"{}\"",
        shown(text).replace('\\', "\\\\").replace('"', "\\\"")
    )
}

/// Reads every stored version and pre-checks its `inputSchema`.
///
/// # Errors
/// [`RunError::Database`] when PostgreSQL cannot be reached or a read fails.
pub async fn input_schema_inventory(db: &DatabaseConfig) -> Result<Inventory, RunError> {
    let pool = open_pool(db, Duration::from_secs(30)).await?;
    let mut inventory = Inventory::default();
    let mut after = None;
    loop {
        let page = stored_versions_page(&pool, after, PAGE)
            .await
            .map_err(|error| RunError::Database(error.to_string()))?;
        let Some(last) = page.last() else {
            break;
        };
        after = Some(last.version);
        for stored in &page {
            inventory.scanned += 1;
            let ids = VersionIds {
                tenant: stored.tenant,
                namespace: stored.namespace,
                workflow: stored.workflow,
                version: stored.version,
                number: stored.number,
            };
            let Ok(read) = stored_input_schema(&stored.document) else {
                inventory.unreadable.push(ids);
                continue;
            };
            let Some(schema) = read.schema else {
                continue;
            };
            match sagat_infra_schema::run_input::precheck(schema.bytes()) {
                Err(unusable) => inventory.refused.push(RefusedVersion {
                    ids,
                    issue: unusable.issue,
                    keyword: unusable.keyword,
                    at: format!("/inputSchema{}", unusable.at),
                }),
                Ok(()) if read.triggered => inventory.triggered.push(ids),
                Ok(()) => {}
            }
        }
    }
    pool.close().await;
    Ok(inventory)
}
