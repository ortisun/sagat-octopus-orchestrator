//! The migrations of feature 008 go back and forward, also with data in the tables they touch.

use std::time::Duration;

use sagat_application::outbound::{OutboundStore, SubscriptionRecord};
use sagat_application::telemetry::TelemetryState;
use sagat_domain::outbound::{
    DeliveryLimits, OutboundKind, SubscriptionId, SubscriptionStatus, TargetUrl,
};
use sagat_domain::secrets::SecretName;
use sagat_infra_postgres::{MIGRATOR, migrate};
use sagat_scheduler::{Monitor, MonitorSettings};
use serde_json::json;
use sqlx::PgPool;

use crate::support::engine::{World, activity, document, world};
use crate::support::{migrated_db, now, slug};

/// The last migration before this feature's.
const BEFORE: i64 = 20_261_014_000_000;

const TABLES: [&str; 4] = [
    "telemetry_pending",
    "engine_leases",
    "operational_alerts",
    "worker_drains",
];

const COLUMNS: [(&str, &str); 4] = [
    ("runs", "trace_parent_span"),
    ("runs", "rerun_of"),
    ("inbox_events", "trace_id"),
    ("inbox_events", "trace_parent_span"),
];

async fn table_exists(pool: &PgPool, name: &str) -> bool {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(format!("public.{name}"))
        .fetch_one(pool)
        .await
        .expect("query")
}

async fn column_exists(pool: &PgPool, table: &str, column: &str) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2)",
    )
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await
    .expect("query")
}

async fn assert_added(pool: &PgPool, expected: bool) {
    for table in TABLES {
        assert_eq!(table_exists(pool, table).await, expected, "{table}");
    }
    for (table, column) in COLUMNS {
        assert_eq!(
            column_exists(pool, table, column).await,
            expected,
            "{table}.{column}"
        );
    }
}

#[tokio::test]
async fn rolling_back_removes_what_the_feature_added_and_going_forward_restores_it() {
    let db = migrated_db().await;
    assert_added(&db.pool, true).await;
    MIGRATOR.undo(&db.pool, BEFORE).await.expect("roll back");
    assert_added(&db.pool, false).await;
    migrate(&db.pool).await.expect("forward again");
    assert_added(&db.pool, true).await;
}

async fn subscribe(w: &World, name: &str, kinds: Vec<OutboundKind>) -> SubscriptionRecord {
    let now = now();
    let record = SubscriptionRecord {
        id: SubscriptionId::generate(),
        tenant_id: w.admin.tenant_id(),
        namespace_id: w.namespace.id(),
        name: slug(name),
        url: TargetUrl::parse("https://hooks.example.com/in").expect("url"),
        secret_name: SecretName::parse("signing").expect("secret"),
        kinds,
        limits: DeliveryLimits::new(100, Duration::from_secs(2)).expect("limits"),
        status: SubscriptionStatus::Active,
        health: sagat_application::outbound::Health::default(),
        created_by: "test".to_owned(),
        created_at: now,
        updated_at: now,
    };
    let audit = sagat_domain::audit::AuditRecord::provisioning(
        w.admin.tenant_id(),
        sagat_domain::audit::AuditAction::WebhookSubscriptionCreated,
        sagat_domain::audit::ResourceType::WebhookSubscription,
        record.id.to_string(),
        "test",
        now,
    );
    w.store
        .create_subscription(&record, 100, &audit)
        .await
        .expect("subscribe");
    record
}

#[tokio::test]
async fn a_rollback_with_notes_alerts_drains_and_alert_subscriptions_keeps_what_older_code_understands()
 {
    let w = world().await;
    w.put_secret("signing", "whsec-0123456789abcdef0123456789abcdef")
        .await;
    // A run with its note; an alert that is firing; a worker being drained; one subscription that
    // asked for alerts among other things and one that asked for nothing else.
    let (wf, _) = w
        .publish(document("rolled-back", "a", vec![activity("a")]))
        .await;
    let run = w.start(wf, &json!({})).await;
    let mixed = subscribe(
        &w,
        "mixed",
        vec![OutboundKind::RunFinished, OutboundKind::AlertFired],
    )
    .await;
    let alerts_only = subscribe(
        &w,
        "alerts-only",
        vec![OutboundKind::AlertFired, OutboundKind::AlertResolved],
    )
    .await;
    let monitor = Monitor::new(
        w.store.clone(),
        MonitorSettings {
            thresholds: sagat_application::operations::Thresholds {
                unclaimed: Duration::ZERO,
                ..sagat_application::operations::Thresholds::default()
            },
            ..MonitorSettings::default()
        },
    );
    assert!(monitor.run_once().await.fired >= 1);
    let told = |pool: &sqlx::PgPool| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM webhook_deliveries WHERE kind = 'ops.alert.fired'",
            )
            .fetch_one(&pool)
            .await
            .expect("count")
        }
    };
    assert!(told(&w.db.pool).await >= 1, "the subscribers were told");
    sqlx::query(
        "INSERT INTO worker_drains (tenant_id, namespace_id, subject, reason, drained_by) \
         VALUES ($1, $2, 'worker-1', 'rolling the deploy', 'olivia')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .bind(w.namespace.id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("a drain");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM telemetry_pending")
            .fetch_one(&w.db.pool)
            .await
            .expect("count"),
        1
    );

    MIGRATOR
        .undo(&w.db.pool, BEFORE)
        .await
        .expect("roll back with data in the tables");
    assert_added(&w.db.pool, false).await;
    assert_eq!(
        told(&w.db.pool).await,
        0,
        "notifications that older code cannot deliver are not left behind"
    );

    // What older code understands is still there: the run, and the subscription narrowed to the
    // one kind it knows; the one that asked for nothing else is gone.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM runs WHERE id = $1")
            .bind(run.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count"),
        1
    );
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kinds::text[] FROM webhook_subscriptions WHERE id = $1")
            .bind(mixed.id.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("kinds");
    assert_eq!(kinds, ["run.finished"]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM webhook_subscriptions WHERE id = $1")
            .bind(alerts_only.id.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count"),
        0
    );

    // Going forward again: the run from before is still there and says it has no telemetry
    // tracked, which is the truth about a run the projection never saw.
    migrate(&w.db.pool).await.expect("forward again");
    assert_added(&w.db.pool, true).await;
    let view = w
        .runs
        .telemetry(&w.admin, run, &crate::support::engine::ctx())
        .await
        .expect("a run from before is readable");
    assert_eq!(view.state, TelemetryState::Untracked, "{view:?}");
}
