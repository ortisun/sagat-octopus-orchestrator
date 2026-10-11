//! What goes wrong around publishing, activating and promoting, and what the platform does about
//! it: two operators moving the pointer at once, a clock that disagrees between replicas, the same
//! person deciding twice, a request published twice, an edit racing an approval, connections killed
//! in the middle of a publication, a duplicate event around a rollback, and a command line that is
//! interrupted or run twice. The invariant: the history is a chain that ends at the active version,
//! no request publishes what its approvers did not see, and nothing is made twice (T011-012,
//! T011-017, T011-022, T011-027).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod cli;
mod events;
mod history;
mod requests;
mod storm;

use sagat_application::clock::Clock;
use sagat_domain::definition::WorkflowId;
use time::OffsetDateTime;

use support::engine::World;

/// A replica whose clock is off by `behind` seconds: in the middle of a rolling restart two
/// replicas of the control plane do not agree on the time, and neither is wrong enough to notice.
#[derive(Clone, Copy)]
pub struct Skewed {
    pub behind: i64,
}

impl Clock for Skewed {
    fn now(&self) -> OffsetDateTime {
        sagat_application::clock::truncate_to_micros(
            OffsetDateTime::now_utc() - time::Duration::seconds(self.behind),
        )
    }
}

/// The rows of the history of a workflow, oldest first, as `(version number, previous number)`.
pub async fn chain(w: &World, workflow: WorkflowId) -> Vec<(i32, Option<i32>)> {
    sqlx::query_as(
        "SELECT v.version_number, p.version_number \
         FROM workflow_activations a \
         JOIN workflow_versions v ON v.tenant_id = a.tenant_id AND v.id = a.version_id \
         LEFT JOIN workflow_versions p ON p.tenant_id = a.tenant_id AND p.id = a.previous_version_id \
         WHERE a.workflow_id = $1 ORDER BY a.created_at, a.id",
    )
    .bind(workflow.as_uuid())
    .fetch_all(&w.db.pool)
    .await
    .expect("history")
}

/// The number of the version the pointer holds.
pub async fn pointer(w: &World, workflow: WorkflowId) -> Option<i32> {
    sqlx::query_scalar(
        "SELECT v.version_number FROM workflows f \
         JOIN workflow_versions v ON v.tenant_id = f.tenant_id AND v.id = f.active_version_id \
         WHERE f.id = $1",
    )
    .bind(workflow.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("pointer")
}

/// The history is a chain (every change starts where the one before ended), it starts from nothing,
/// and its last change is the version the pointer holds.
#[track_caller]
pub fn assert_chain(rows: &[(i32, Option<i32>)], pointer: Option<i32>) {
    assert!(!rows.is_empty(), "a published workflow has a history");
    assert_eq!(
        rows[0].1, None,
        "the first change starts from nothing: {rows:?}"
    );
    for pair in rows.windows(2) {
        assert_eq!(
            pair[1].1,
            Some(pair[0].0),
            "each change starts where the one before ended: {rows:?}"
        );
    }
    assert_eq!(
        rows.last().map(|r| r.0),
        pointer,
        "the newest change is the active version: {rows:?}"
    );
}
