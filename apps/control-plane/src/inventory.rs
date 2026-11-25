//! `control-plane input-schema-inventory` (design 043, T043-24, FR-043-018, D043-052, D043-072): the stored workflow
//! versions whose `inputSchema` is outside the supported subset, so an operator can see before the upgrade which
//! versions will refuse every start in `enforce` mode, and fix them or choose `report` first.
//!
//! It runs with the database settings of the control plane (the runtime role where a deployment sets one): it only
//! reads, and needs nothing but `SELECT` on `workflow_versions`, `workflows`, `runs`, `schedules` and
//! `schedule_revisions` (the last two absent on a database of 0.2.x, which it reads before the migration). It reads the
//! versions page by page and runs on each stored `inputSchema` the pre-check that publication and the start gate run. It prints one line
//! per version that would be refused (its ids, the issue, the keyword and its pointer), and, for information, one line
//! per version within the subset that has an `inputSchema` and a trigger other than a manual start, whose triggered runs
//! may be refused (G043-004). A version outside the subset that nothing starts any more (not its workflow's active one,
//! pinned by no schedule that is not deleted, no open run, no run created in [`SUPERSEDED_QUIET_DAYS`] days) is
//! superseded: counted in the summary, listed with
//! `--all`, and never a reason to exit 1 (OD-2026-11-16-5). It prints no schema text and no input, and writes nothing.
//! Its exit status is 0 when no version would be refused, 1 when at least one would, and 2 when the command itself
//! failed.

use std::fmt;
use std::time::Duration;

use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::InputSchemaIssue;
use sagat_infra_ir::stored_input_schema;
use sagat_infra_postgres::{schedules_exist, stored_versions_page, version_still_started};
use sagat_infra_schema::run_input::PrecheckRefusal;

use crate::config::DatabaseConfig;
use crate::handover::shown;
use crate::{RunError, open_pool};

/// Versions read per page: a document is at most 1 MiB, so a page holds at most this many MiB.
const PAGE: u32 = 64;

/// A version outside the subset that is not its workflow's active one and has had no open run and no new run for this
/// many days is superseded: nothing starts it, so it does not make the inventory exit 1 (OD-2026-11-16-5, D043-179).
pub const SUPERSEDED_QUIET_DAYS: u16 = 30;

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
    /// The versions outside the subset that nothing starts any more ([`SUPERSEDED_QUIET_DAYS`]): counted in the summary,
    /// listed with [`Scope::All`], never a reason to exit 1.
    pub superseded: Vec<RefusedVersion>,
    /// What the printed inventory lists.
    pub scope: Scope,
    /// The versions read, all of them.
    pub scanned: u64,
    /// The stored documents that are not JSON objects (none should be): listed, never a reason to stop.
    pub unreadable: Vec<VersionIds>,
    /// The versions whose schema made the validator panic, a defect to report: listed, never a reason to stop.
    pub interrupted: Vec<VersionIds>,
}

impl Inventory {
    /// 0 when no version would be refused, 1 when at least one would (D043-072; a superseded one does not,
    /// OD-2026-11-16-5), and 2 when a stored document could not be read or its schema made the validator panic, because
    /// the command cannot say what that version would do (D043-111, D043-187).
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        if !self.unreadable.is_empty() || !self.interrupted.is_empty() {
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
        if self.scope == Scope::All {
            for refused in &self.superseded {
                writeln!(
                    f,
                    "superseded {} issue={} keyword={} at={} nothing has started it for {SUPERSEDED_QUIET_DAYS} days",
                    refused.ids,
                    refused.issue.as_str(),
                    quoted(&refused.keyword),
                    quoted(&refused.at)
                )?;
            }
        }
        for ids in &self.triggered {
            writeln!(
                f,
                "information {ids} the inputSchema is applied to the event envelope of its triggers"
            )?;
        }
        for ids in &self.interrupted {
            writeln!(
                f,
                "interrupted {ids} the validator failed on this schema, a defect of the platform: report it with these ids"
            )?;
        }
        for ids in &self.unreadable {
            writeln!(
                f,
                "unreadable {ids} the stored document is not a JSON object: what this version would do is unknown"
            )?;
        }
        let hint = if self.scope == Scope::All {
            ""
        } else {
            " (--all lists them)"
        };
        write!(
            f,
            "{} stored version(s) read: {} would refuse every start in enforce mode, {} superseded version(s) outside \
             the subset that nothing starts are not counted{hint}, {} with a trigger are listed for information. Nothing \
             was changed.",
            self.scanned,
            self.refused.len(),
            self.superseded.len(),
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

/// Which versions outside the subset the inventory lists (OD-2026-11-16-5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Scope {
    /// Those something may still start; the superseded ones are counted in the summary.
    #[default]
    Current,
    /// The superseded ones as well (`--all`).
    All,
}

/// Reads every stored version and pre-checks its `inputSchema`, sorting a version outside the subset into `refused` when
/// something may still start it and into `superseded` when nothing does (OD-2026-11-16-5); `scope` says what is printed.
///
/// # Errors
/// [`RunError::Database`] when PostgreSQL cannot be reached or a read fails.
pub async fn input_schema_inventory_in(
    db: &DatabaseConfig,
    scope: Scope,
) -> Result<Inventory, RunError> {
    let pool = open_pool(
        db,
        Duration::from_secs(30),
        sagat_infra_postgres::PoolName::Process("inventory"),
    )
    .await?;
    let mut inventory = Inventory {
        scope,
        ..Inventory::default()
    };
    // a database of 0.2.x, read before the migration, has no schedules to pin a version (D043-187)
    let schedules = schedules_exist(&pool)
        .await
        .map_err(|error| RunError::Database(error.to_string()))?;
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
                // the inventory checks one schema at a time in its own process: its pool is never full
                Err(PrecheckRefusal::Busy) => {
                    return Err(RunError::Refused(
                        "the run-input validator of the inventory had no permit; run it again",
                    ));
                }
                // a defect of the validator on this schema, which a retry meets again: the version is listed by its ids
                // and the scan goes on; the command cannot say what it would do, so it exits 2 (D043-187)
                Err(PrecheckRefusal::Interrupted) => inventory.interrupted.push(ids),
                Err(PrecheckRefusal::Unusable(unusable)) => {
                    let refused = RefusedVersion {
                        ids,
                        issue: unusable.issue,
                        keyword: unusable.keyword,
                        at: format!("/inputSchema{}", unusable.at),
                    };
                    let started = version_still_started(
                        &pool,
                        stored.tenant,
                        stored.workflow,
                        stored.version,
                        SUPERSEDED_QUIET_DAYS,
                        schedules,
                    )
                    .await
                    .map_err(|error| RunError::Database(error.to_string()))?;
                    if started {
                        inventory.refused.push(refused);
                    } else {
                        inventory.superseded.push(refused);
                    }
                }
                Ok(()) if read.triggered => inventory.triggered.push(ids),
                Ok(()) => {}
            }
        }
    }
    pool.close().await;
    Ok(inventory)
}

/// Reads every stored version and pre-checks its `inputSchema`: [`Scope::Current`].
///
/// # Errors
/// [`RunError::Database`] when PostgreSQL cannot be reached or a read fails.
pub async fn input_schema_inventory(db: &DatabaseConfig) -> Result<Inventory, RunError> {
    input_schema_inventory_in(db, Scope::Current).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(number: i32) -> VersionIds {
        VersionIds {
            tenant: TenantId::generate(),
            namespace: NamespaceId::generate(),
            workflow: WorkflowId::generate(),
            version: WorkflowVersionId::generate(),
            number,
        }
    }

    /// D043-187: a version whose schema made the validator panic is listed by its ids, beside the versions the scan went
    /// on to judge, and the command exits 2, since it cannot say what that version would do; never 0 or 1.
    #[test]
    fn a_version_the_validator_cannot_judge_makes_the_inventory_exit_2_and_is_listed() {
        let judged = RefusedVersion {
            ids: ids(1),
            issue: InputSchemaIssue::KeywordUnsupported,
            keyword: "patternProperties".to_owned(),
            at: "/inputSchema/patternProperties".to_owned(),
        };
        let interrupted = ids(2);
        let inventory = Inventory {
            refused: vec![judged],
            interrupted: vec![interrupted],
            scanned: 2,
            ..Inventory::default()
        };
        assert_eq!(inventory.exit_code(), 2);
        let printed = inventory.to_string();
        assert!(
            printed.contains(&format!("interrupted {interrupted}")),
            "{printed}"
        );
        assert!(printed.contains("refused tenant="), "{printed}");
        let quiet = Inventory {
            interrupted: vec![ids(3)],
            ..Inventory::default()
        };
        assert_eq!(quiet.exit_code(), 2, "even with nothing refused");
    }
}
