//! The migration of the instance-capability registry (`T018-080`, `20270113000000_instance_registry`) against a real
//! PostgreSQL: its `down` refuses while a requirement, the history of an activation or a live process is there, removes
//! everything it made on a schema that uses none of it, and the `up` applies again. The style of
//! `pending_resets_migration.rs`.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::instances::{Capability, InstanceKind, InstanceStore};
use sagat_domain::identity::Subject;
use sagat_infra_postgres::migrate;

use crate::migration::undo_to;
use crate::support::instances::{nothing, registry};

/// The version of this migration and the one before it, from the migrator (a later migration cannot make a rollback of
/// this one alone stop in the wrong place).
const THIS: i64 = 20_270_113_000_000;

fn previous() -> i64 {
    sagat_infra_postgres::MIGRATOR
        .iter()
        .filter(|migration| migration.migration_type.is_up_migration())
        .map(|migration| migration.version)
        .filter(|version| *version < THIS)
        .max()
        .expect("a migration before this one")
}

const OBJECTS: [&str; 4] = [
    "serving_instances",
    "platform_requirements",
    "platform_requirement_epoch",
    "platform_requirement_history",
];
const FUNCTIONS: [&str; 9] = [
    "instance_register",
    "instance_heartbeat",
    "instance_deregister",
    "platform_requirement_lock",
    "platform_requirement_activate",
    "platform_requirement_remove",
    "platform_requirements_guard",
    "platform_requirement_epoch_guard",
    "platform_requirements_epoch_moved",
];

async fn present(r: &crate::support::instances::Registry) -> (i64, i64) {
    let tables: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_class WHERE relname = ANY($1) AND relkind = 'r'",
    )
    .bind(OBJECTS.map(str::to_owned).to_vec())
    .fetch_one(&r.db.pool)
    .await
    .expect("tables");
    let functions: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_proc WHERE proname = ANY($1)")
        .bind(FUNCTIONS.map(str::to_owned).to_vec())
        .fetch_one(&r.db.pool)
        .await
        .expect("functions");
    (tables, functions)
}

#[tokio::test]
async fn the_rollback_is_refused_while_a_requirement_or_the_history_of_one_is_there() {
    let r = registry().await;
    r.require(Capability::SchedulesV1, &[InstanceKind::ExecutionEngine])
        .await
        .expect("activate");
    let refused = undo_to(&r.db, previous())
        .await
        .expect_err("a requirement is evidence");
    assert!(format!("{refused:?}").contains("retained"), "{refused:?}");
    assert_eq!(present(&r).await, (4, 9), "nothing was lost");

    // removing the requirement leaves the history of both acts, and the history is retained too
    r.operator
        .remove(Capability::SchedulesV1, &Subject::operator(), nothing)
        .await
        .expect("remove");
    let refused = undo_to(&r.db, previous())
        .await
        .expect_err("the history is evidence");
    assert!(format!("{refused:?}").contains("retained"), "{refused:?}");
    assert_eq!(r.history().await, 2);

    // an operator who has read the history may remove it; the guard is disabled on purpose and says so here: this is
    // the one way the history ever leaves, and it is not the rollback's
    sqlx::query("ALTER TABLE platform_requirement_history DISABLE TRIGGER platform_requirement_history_append_only")
        .execute(&r.db.pool)
        .await
        .expect("the operator lifts the guard");
    sqlx::query("DELETE FROM platform_requirement_history")
        .execute(&r.db.pool)
        .await
        .expect("the operator removes the history");
    undo_to(&r.db, previous())
        .await
        .expect("a schema that records nothing rolls back");
    assert_eq!(
        present(&r).await,
        (0, 0),
        "everything the migration made is gone"
    );
    migrate(&r.db.pool).await.expect("forward again");
    assert_eq!(present(&r).await, (4, 9));
    assert_eq!(r.epoch_now().await, 0, "the epoch starts again at zero");
}

#[tokio::test]
async fn the_rollback_is_refused_while_a_process_is_live_and_goes_when_only_silent_rows_remain() {
    let r = registry().await;
    let instance = r.serving(InstanceKind::ControlPlane, &[]).await;
    let refused = undo_to(&r.db, previous())
        .await
        .expect_err("a live process is using the registry");
    assert!(format!("{refused:?}").contains("live"), "{refused:?}");
    assert_eq!(present(&r).await, (4, 9), "nothing was lost");

    r.age(instance, 40).await;
    undo_to(&r.db, previous())
        .await
        .expect("rows of processes that are gone are the registry's own bookkeeping");
    assert_eq!(present(&r).await, (0, 0));

    migrate(&r.db.pool).await.expect("forward again");
    assert_eq!(present(&r).await, (4, 9));
    assert_eq!(r.rows().await, 0, "the rows went with the table");
    // and the registry works again
    let again = r
        .serving(InstanceKind::ExecutionEngine, &[Capability::SchedulesV1])
        .await;
    assert!(r.row_exists(again).await);
    assert_eq!(r.process.epoch().await.expect("epoch"), 0);
}
