//! PostgreSQL deferrable constraints fail at COMMIT, after the audit INSERT has succeeded.
#![allow(clippy::expect_used)]

use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::event_payloads::{
    EventPayloadError, EventPayloadService, EventPayloadUseCase, EventReveal,
};
use sagat_application::payload_disclosure::{
    DisclosureSelector, PayloadDisclosureError, PayloadDisclosureService, PayloadDisclosureUseCase,
};
use sagat_domain::events::{EventId, EventKey};
use sagat_domain::identity::{Role, Slug};
use sagat_infra_ir::JsonPayloadProjector;
use sagat_infra_schema::classification::Classifier;
use serde_json::json;

use super::support::engine::{ctx, document, end_node, world};

async fn audit_commit_failure(reveal_granted: bool) {
    let world = world().await;
    let mut source = document("commit-audit", "done", vec![end_node("done")]);
    source["inputSchema"] = json!({"type":"object", "x-sagat-classification":"pii"});
    let (workflow, _) = world.publish(source).await;
    let run = world
        .start(workflow, &json!({"personal":"commit-canary"}))
        .await;
    let reader = world.member("commit-reader", Role::Developer).await;
    let key = EventKey {
        source: Slug::parse("commit-source").expect("source"),
        event_id: EventId::parse("commit-event").expect("event id"),
    };
    sqlx::query("INSERT INTO inbox_events (tenant_id,namespace_id,source,event_id,event_type,payload,payload_digest,received_at,expires_at,next_attempt_at) VALUES ($1,$2,$3,$4,'commit.test',$5,decode(repeat('03',32),'hex'),clock_timestamp(),clock_timestamp()+interval '1 hour',clock_timestamp())")
        .bind(world.admin.tenant_id().as_uuid()).bind(world.namespace.id().as_uuid())
        .bind(key.source.as_str()).bind(key.event_id.as_str())
        .bind(br#"{"personal":"commit-canary"}"#.as_slice())
        .execute(&world.db.pool).await.expect("event");
    if reveal_granted {
        sqlx::query("INSERT INTO payload_reveal_grants (tenant_id,namespace_id,subject,granted_by,granted_at) VALUES ($1,$2,$3,$4,clock_timestamp())")
            .bind(world.admin.tenant_id().as_uuid()).bind(world.namespace.id().as_uuid())
            .bind(reader.subject().as_str()).bind(world.admin.subject().as_str())
            .execute(&world.db.pool).await.expect("grant");
    }
    sqlx::raw_sql("CREATE FUNCTION refuse_reveal_audit_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action IN ('payload.reveal_denied','payload.pii_revealed') THEN RAISE EXCEPTION 'injected audit commit failure'; END IF; RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER refuse_reveal_audit_commit AFTER INSERT ON audit_records DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION refuse_reveal_audit_commit();")
        .execute(&world.db.pool).await.expect("deferred failure");
    let run_service = PayloadDisclosureService::new(
        world.store.clone(),
        SystemClock,
        Classifier::default(),
        JsonPayloadProjector,
    );
    let prepared_run = run_service
        .prepare(&reader, run, DisclosureSelector::Input, true, &ctx())
        .await
        .expect("prepare run");
    let run_result = run_service.disclose(prepared_run).await.map(|_| ());
    let event_service =
        EventPayloadService::new(world.store.clone(), SystemClock, JsonPayloadProjector);
    let prepared_event = event_service
        .prepare_event(&reader, world.namespace.id(), key, EventReveal::Pii, &ctx())
        .await
        .expect("prepare event");
    let event_result = event_service.event(prepared_event).await.map(|_| ());
    let expected = if reveal_granted {
        (
            Err(PayloadDisclosureError::Application(AppError::Internal)),
            Err(EventPayloadError::Application(AppError::Internal)),
        )
    } else {
        (
            Err(PayloadDisclosureError::PiiRevealNotGranted),
            Err(EventPayloadError::PiiRevealNotGranted),
        )
    };
    assert_eq!((run_result, event_result), expected);
    let persisted: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE actor=$1 AND action IN ('payload.reveal_denied','payload.pii_revealed')")
        .bind(reader.subject().as_str()).fetch_one(&world.db.pool).await.expect("rolled back audits");
    assert_eq!(persisted, 0);
}

#[tokio::test]
async fn a_failed_denial_audit_commit_preserves_typed_refusal() {
    audit_commit_failure(false).await;
}

#[tokio::test]
async fn a_failed_success_audit_commit_never_returns_values() {
    audit_commit_failure(true).await;
}
