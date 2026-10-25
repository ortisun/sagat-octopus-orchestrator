#![allow(clippy::expect_used)]

use sagat_application::clock::SystemClock;
use sagat_application::error::AppError;
use sagat_application::event_payloads::{
    EventData, EventPayloadError, EventPayloadService, EventPayloadUseCase, EventReveal,
};
use sagat_domain::events::{EventId, EventKey};
use sagat_domain::identity::{Role, Slug};
use sagat_infra_ir::JsonPayloadProjector;

use super::support::engine::{ctx, world};

#[tokio::test]
async fn event_values_need_current_authority_and_every_reveal_is_audited() {
    let w = world().await;
    let key = EventKey {
        source: Slug::parse("source-a").unwrap(),
        event_id: EventId::parse("event-1").unwrap(),
    };
    sqlx::query("INSERT INTO inbox_events (tenant_id,namespace_id,source,event_id,event_type,payload,payload_digest,received_at,expires_at,next_attempt_at) VALUES ($1,$2,$3,$4,'invoice.created',$5,decode(repeat('01',32),'hex'),clock_timestamp(),clock_timestamp()+interval '1 hour',clock_timestamp())")
        .bind(w.admin.tenant_id().as_uuid()).bind(w.namespace.id().as_uuid()).bind(key.source.as_str()).bind(key.event_id.as_str()).bind(br#"{"canary":"event-pii"}"#.as_slice()).execute(&w.db.pool).await.expect("event");
    let reader = w.member("event-reader", Role::Developer).await;
    let service = EventPayloadService::new(w.store.clone(), SystemClock, JsonPayloadProjector);
    assert!(
        matches!(
            service
                .prepare_event(
                    &w.other_admin,
                    w.namespace.id(),
                    key.clone(),
                    EventReveal::Pii,
                    &ctx()
                )
                .await,
            Err(AppError::NotFound)
        ),
        "foreign resources remain 404 before reveal authority is considered"
    );
    let default = service
        .prepare_event(
            &reader,
            w.namespace.id(),
            key.clone(),
            EventReveal::Structure,
            &ctx(),
        )
        .await
        .unwrap();
    let answer = service.event(default).await.unwrap();
    assert!(matches!(answer.data, EventData::Structure(_)));
    assert!(!format!("{answer:?}").contains("event-pii"));
    let denied = service
        .prepare_event(
            &reader,
            w.namespace.id(),
            key.clone(),
            EventReveal::Pii,
            &ctx(),
        )
        .await
        .unwrap();
    assert_eq!(
        service.event(denied).await,
        Err(EventPayloadError::PiiRevealNotGranted)
    );
    sqlx::query("INSERT INTO payload_reveal_grants (tenant_id,namespace_id,subject,granted_by,granted_at) VALUES ($1,$2,$3,$4,clock_timestamp())").bind(w.admin.tenant_id().as_uuid()).bind(w.namespace.id().as_uuid()).bind(reader.subject().as_str()).bind(w.admin.subject().as_str()).execute(&w.db.pool).await.unwrap();
    for _ in 0..2 {
        let prepared = service
            .prepare_event(
                &reader,
                w.namespace.id(),
                key.clone(),
                EventReveal::Pii,
                &ctx(),
            )
            .await
            .unwrap();
        assert!(matches!(
            service.event(prepared).await.unwrap().data,
            EventData::Revealed(_)
        ));
    }
    let audits:i64=sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE tenant_id=$1 AND actor=$2 AND resource_type='event' AND resource_id='source-a/event-1' AND action='payload.pii_revealed' AND reason='event-data:reveal=pii'").bind(w.admin.tenant_id().as_uuid()).bind(reader.subject().as_str()).fetch_one(&w.db.pool).await.unwrap();
    assert_eq!(audits, 2);
    let prepared = service
        .prepare_event(&reader, w.namespace.id(), key, EventReveal::Pii, &ctx())
        .await
        .unwrap();
    sqlx::query("UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject=$2")
        .bind(w.admin.tenant_id().as_uuid())
        .bind(reader.subject().as_str())
        .execute(&w.db.pool)
        .await
        .unwrap();
    assert_eq!(
        service.event(prepared).await,
        Err(EventPayloadError::PiiRevealNotGranted)
    );
}

#[tokio::test]
async fn denied_audit_is_best_effort_but_success_audit_is_mandatory() {
    let w = world().await;
    let key = EventKey {
        source: Slug::parse("source-b").unwrap(),
        event_id: EventId::parse("event-audit").unwrap(),
    };
    sqlx::query("INSERT INTO inbox_events (tenant_id,namespace_id,source,event_id,event_type,payload,payload_digest,received_at,expires_at,next_attempt_at) VALUES ($1,$2,$3,$4,'invoice.created',$5,decode(repeat('02',32),'hex'),clock_timestamp(),clock_timestamp()+interval '1 hour',clock_timestamp())")
        .bind(w.admin.tenant_id().as_uuid()).bind(w.namespace.id().as_uuid()).bind(key.source.as_str()).bind(key.event_id.as_str()).bind(br#"{"canary":"audit-pii"}"#.as_slice()).execute(&w.db.pool).await.expect("event");
    let reader = w.member("audit-reader", Role::Developer).await;
    let service = EventPayloadService::new(w.store.clone(), SystemClock, JsonPayloadProjector);
    sqlx::raw_sql("CREATE FUNCTION refuse_event_denied_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='payload.reveal_denied' THEN RAISE EXCEPTION 'injected denied audit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER refuse_event_denied_audit BEFORE INSERT ON audit_records FOR EACH ROW EXECUTE FUNCTION refuse_event_denied_audit();")
        .execute(&w.db.pool).await.expect("denied audit failure");
    let denied = service
        .prepare_event(
            &reader,
            w.namespace.id(),
            key.clone(),
            EventReveal::Pii,
            &ctx(),
        )
        .await
        .unwrap();
    assert_eq!(
        service.event(denied).await,
        Err(EventPayloadError::PiiRevealNotGranted)
    );
    sqlx::raw_sql("DROP TRIGGER refuse_event_denied_audit ON audit_records; DROP FUNCTION refuse_event_denied_audit();")
        .execute(&w.db.pool).await.unwrap();
    sqlx::query("INSERT INTO payload_reveal_grants (tenant_id,namespace_id,subject,granted_by,granted_at) VALUES ($1,$2,$3,$4,clock_timestamp())")
        .bind(w.admin.tenant_id().as_uuid()).bind(w.namespace.id().as_uuid()).bind(reader.subject().as_str()).bind(w.admin.subject().as_str()).execute(&w.db.pool).await.unwrap();
    sqlx::raw_sql("CREATE FUNCTION refuse_event_success_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='payload.pii_revealed' THEN RAISE EXCEPTION 'injected success audit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER refuse_event_success_audit BEFORE INSERT ON audit_records FOR EACH ROW EXECUTE FUNCTION refuse_event_success_audit();")
        .execute(&w.db.pool).await.expect("success audit failure");
    let reveal = service
        .prepare_event(&reader, w.namespace.id(), key, EventReveal::Pii, &ctx())
        .await
        .unwrap();
    assert_eq!(
        service.event(reveal).await,
        Err(EventPayloadError::Application(AppError::Unavailable)),
        "values cannot cross a failed required audit"
    );
}

/// Constitution V: probing the event reads with a namespace of another tenant is the opaque 404 and
/// leaves a `CrossTenantAccessDenied` record against the tenant that probed, for the three
/// operations (`getInboxEvent`, `getConnectorDeadLetter`, `listConnectorDeadLetters`).
#[tokio::test]
async fn cross_tenant_probes_of_the_event_reads_are_audited_against_the_prober() {
    use sagat_domain::audit::AuditAction;
    use sagat_domain::connectors::ConnectorId;
    use uuid::Uuid;

    let w = world().await;
    let service = EventPayloadService::new(w.store.clone(), SystemClock, JsonPayloadProjector);
    let key = EventKey {
        source: Slug::parse("source-a").unwrap(),
        event_id: EventId::parse("event-probe").unwrap(),
    };
    let audited = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT resource_id FROM audit_records WHERE tenant_id = $1 AND action = $2",
        )
        .bind(w.other_admin.tenant_id().as_uuid())
        .bind(AuditAction::CrossTenantAccessDenied.as_str())
        .fetch_all(&w.db.pool)
        .await
        .unwrap()
    };
    assert!(audited().await.is_empty(), "nothing probed yet");
    // The probes are written off the response path; each one is awaited before the next so that a
    // collapsed repeat cannot hide a missing one.
    let probes: [(&str, Result<(), AppError>); 3] = [
        (
            "event",
            service
                .prepare_event(
                    &w.other_admin,
                    w.namespace.id(),
                    key,
                    EventReveal::Structure,
                    &ctx(),
                )
                .await
                .map(|_| ()),
        ),
        (
            "dead letter",
            service
                .prepare_dead_letter(
                    &w.other_admin,
                    w.namespace.id(),
                    ConnectorId::new(Uuid::new_v4()),
                    Some(Uuid::new_v4()),
                    EventReveal::Pii,
                    &ctx(),
                )
                .await
                .map(|_| ()),
        ),
        (
            "dead letter list",
            service
                .prepare_dead_letter(
                    &w.other_admin,
                    w.namespace.id(),
                    ConnectorId::new(Uuid::new_v4()),
                    None,
                    EventReveal::Structure,
                    &ctx(),
                )
                .await
                .map(|_| ()),
        ),
    ];
    for (what, outcome) in probes {
        assert_eq!(outcome, Err(AppError::NotFound), "{what}");
    }
    let rows = super::support::eventually("the probes are audited", || async {
        let rows = audited().await;
        (!rows.is_empty()).then_some(rows)
    })
    .await;
    assert!(
        rows.iter().all(|r| *r == w.namespace.id().to_string()),
        "naming the namespace they asked for, and no value"
    );
    assert!(
        !rows.is_empty(),
        "at least the first probe (identical repeats collapse)"
    );
}
