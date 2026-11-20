//! Current real event admission and one exact native receipt/value transaction.
use super::*;
use sagat_application::{
    dispatch::{ClaimedEvent, EventDispatchStore, WaitingStep},
    events::PublishRequest,
};
use sagat_domain::{events::EventId, workflow::EventType};

mod admission;
mod races;

async fn source(name: &str) -> (Fixture, PgRunStore, WaitingStep, ClaimedEvent) {
    source_document(name, wait_document(name, true, 60000)).await
}

async fn source_document(
    name: &str,
    document: serde_json::Value,
) -> (Fixture, PgRunStore, WaitingStep, ClaimedEvent) {
    let fixture = scoped_worker::fixture_with_document(name, document).await;
    source_fixture(fixture).await
}

async fn source_fixture(fixture: Fixture) -> (Fixture, PgRunStore, WaitingStep, ClaimedEvent) {
    let (fixture, store) = pending_wait_fixture(fixture).await;
    guards::start(&fixture, &store, false)
        .await
        .expect("real wait subscription");
    fixture
        .world
        .events
        .publish(
            &fixture.world.admin,
            PublishRequest {
                namespace: "payments".into(),
                event_id: Some(EventId::parse("scoped-event-1").expect("id")),
                event_type: EventType::parse("payment.confirmed").expect("type"),
                subject: Some("order-1".into()),
                correlation_key: Some(execution::CorrelationKey::parse("first").expect("key")),
                occurred_at: None,
                data: br#"{"value":"scope-event"}"#.to_vec(),
                dedupe_ttl: None,
            },
            &crate::support::engine::ctx(),
        )
        .await
        .expect("actual event service admission");
    let mut admitted = store
        .claim_events(8, std::time::Duration::from_secs(60))
        .await
        .expect("actual native claim");
    assert_eq!(admitted.len(), 1);
    let event = admitted.pop().expect("one admitted event");
    let (step_id,since):(uuid::Uuid,time::OffsetDateTime)=sqlx::query_as("SELECT step_id,event_since FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[0]")
        .bind(fixture.world.admin.tenant_id().as_uuid()).bind(fixture.run.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("actual current qualified subscription");
    let candidate = WaitingStep {
        tenant_id: fixture.world.admin.tenant_id(),
        namespace_id: fixture.world.namespace.id(),
        run_id: fixture.run,
        instance: wait(0),
        step_id,
        event_type: EventType::parse("payment.confirmed").expect("type"),
        correlation_key: execution::CorrelationKey::parse("first").expect("key"),
        since,
    };
    (fixture, store, candidate, event)
}

pub(super) async fn persist_full_delivery(
    tx: &mut impl RunTx,
    candidate: &WaitingStep,
    event: &ClaimedEvent,
) -> Option<Vec<u8>> {
    let document = tx
        .record_scoped_event_delivery(candidate, event)
        .await
        .expect("authorized native full receipt")?;
    let progress = {
        let (definition, state) = tx.parts();
        execution::satisfy_event_wait_instance(
            definition,
            state,
            &candidate.instance,
            &event.envelope.event_type,
            event.envelope.correlation_key.as_ref().expect("key"),
            event.received_at,
        )
        .expect("pure full matching event")
    };
    let effects = delivery_effects(candidate, tx.now());
    tx.persist_addressed(
        &progress.events,
        &[(candidate.instance.clone(), document.clone())],
        effects,
    )
    .await
    .expect("atomic native full value/outcome persistence");
    Some(document)
}

async fn persist_path_delivery(
    tx: &mut impl PathExecutionTx,
    candidate: &WaitingStep,
    event: &ClaimedEvent,
) -> Option<Vec<u8>> {
    let document = tx
        .record_scoped_event_delivery(candidate, event)
        .await
        .expect("authorized native bounded receipt")?;
    let progress = {
        let prepared = tx.snapshot();
        execution::satisfy_event_wait_path_instance(
            &prepared.definition,
            &mut prepared.state,
            &candidate.instance,
            &event.envelope.event_type,
            event.envelope.correlation_key.as_ref().expect("key"),
            event.received_at,
        )
        .expect("pure bounded matching event")
    };
    let effects = delivery_effects(candidate, tx.now());
    tx.persist_addressed(
        &progress.events,
        &[(candidate.instance.clone(), document.clone())],
        effects,
    )
    .await
    .expect("atomic native bounded value/outcome persistence");
    Some(document)
}

fn delivery_effects(candidate: &WaitingStep, now: time::OffsetDateTime) -> Effects {
    use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
    let mut effects = Effects::by(Actor::System("event-dispatcher"), None);
    effects.audit = Some(AuditRecord::engine(
        candidate.tenant_id,
        AuditAction::RunSignalled,
        ResourceType::Run,
        candidate.run_id.to_string(),
        "event woke scoped execution",
        None,
        now,
    ));
    effects
}

async fn deliver(
    store: &PgRunStore,
    candidate: &WaitingStep,
    event: &ClaimedEvent,
    full: bool,
) -> Vec<u8> {
    if full {
        let mut tx = store
            .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
            .await
            .expect("full ingress entry")
            .expect("run");
        let document = persist_full_delivery(&mut tx, candidate, event)
            .await
            .expect("new delivery");
        tx.commit().await.expect("atomic full receipt/value commit");
        document
    } else {
        let prepared = store
            .load_path(
                candidate.tenant_id,
                candidate.run_id,
                candidate.instance.path().clone(),
            )
            .await
            .expect("bounded snapshot")
            .expect("path");
        let mut tx = store
            .begin_path(prepared, Lock::Wait)
            .await
            .expect("revision fenced entry")
            .expect("run");
        let document = persist_path_delivery(&mut tx, candidate, event)
            .await
            .expect("new delivery");
        tx.commit()
            .await
            .expect("atomic bounded receipt/value commit");
        document
    }
}

async fn receive(full: bool) {
    let (fixture, store, candidate, event) = source(&format!("scoped-event-receive-{full}")).await;
    let document = deliver(&store, &candidate, &event, full).await;
    let payload: serde_json::Value = serde_json::from_slice(&document).expect("canonical document");
    assert_eq!(payload["data"], json!({"value":"scope-event"}));
    let receipt:(String,uuid::Uuid,Vec<String>,Vec<i32>)=sqlx::query_as("SELECT consumer,instance_id,instance_path_nodes,instance_path_indices FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2")
        .bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("independent exact receipt metadata");
    assert_eq!(
        receipt,
        (
            format!("wait-instance:{}/{}", fixture.run, candidate.step_id),
            candidate.step_id,
            vec!["items".into()],
            vec![0]
        )
    );
    let actual:(String,bool,Vec<u8>,bool)=sqlx::query_as("SELECT s.status,s.routed,p.content,s.waiting_kind IS NULL AND s.due_at IS NULL AND s.event_type IS NULL AND s.correlation_key IS NULL AND s.event_since IS NULL AND s.event_scanned_at IS NULL FROM steps s JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='b' AND s.instance_path_indices=ARRAY[0]")
        .bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("qualified current output");
    assert_eq!(actual, ("SUCCEEDED".into(), true, document, true));
    let sibling:(String,Option<Vec<u8>>)=sqlx::query_as("SELECT status,output_digest FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='b' AND instance_path_indices=ARRAY[1]")
        .bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("untouched same-name sibling");
    assert_eq!(sibling, ("PENDING".into(), None));
    assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
    store
        .verify_run(candidate.tenant_id, candidate.run_id)
        .await
        .expect("native receipt/value cold structured-codec replay");
}

#[tokio::test]
async fn a_current_event_retains_its_exact_scoped_receipt_and_value_in_the_full_writer() {
    receive(true).await;
}

#[tokio::test]
async fn a_current_event_retains_its_exact_scoped_receipt_and_value_in_the_bounded_writer() {
    receive(false).await;
}

#[tokio::test]
async fn a_staged_receipt_cannot_commit_without_its_satisfaction_and_value() {
    for full in [false, true] {
        let (fixture, store, candidate, event) =
            source(&format!("event-receipt-orphan-{full}")).await;
        let accepted = if full {
            let mut tx = store
                .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
                .await
                .expect("full entry")
                .expect("run");
            assert!(
                tx.record_scoped_event_delivery(&candidate, &event)
                    .await
                    .expect("real staged full receipt")
                    .is_some()
            );
            tx.commit().await.is_ok()
        } else {
            let prepared = store
                .load_path(
                    candidate.tenant_id,
                    candidate.run_id,
                    candidate.instance.path().clone(),
                )
                .await
                .expect("snapshot")
                .expect("path");
            let mut tx = store
                .begin_path(prepared, Lock::Wait)
                .await
                .expect("entry")
                .expect("run");
            assert!(
                tx.record_scoped_event_delivery(&candidate, &event)
                    .await
                    .expect("real staged bounded receipt")
                    .is_some()
            );
            tx.commit().await.is_ok()
        };
        assert!(
            !accepted,
            "a native receipt alone cannot acknowledge delivery: full={full}"
        );
        let receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2",
        )
        .bind(candidate.tenant_id.as_uuid())
        .bind(candidate.run_id.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("no orphan row");
        assert_eq!(receipts, 0);
    }
}

#[tokio::test]
async fn even_an_unmatched_receipt_attempt_cannot_be_repeated_in_one_transaction() {
    for full in [false, true] {
        let (_fixture, store, candidate, event) =
            source(&format!("receipt-single-attempt-{full}")).await;
        let mut unmatched = candidate.clone();
        unmatched.step_id = uuid::Uuid::now_v7();
        if full {
            let mut tx = store
                .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
                .await
                .expect("entry")
                .expect("run");
            assert!(
                tx.record_scoped_event_delivery(&unmatched, &event)
                    .await
                    .expect("unmatched candidate")
                    .is_none()
            );
            assert!(
                tx.record_scoped_event_delivery(&candidate, &event)
                    .await
                    .is_err(),
                "a second full receipt operation was accepted"
            );
        } else {
            let prepared = store
                .load_path(
                    candidate.tenant_id,
                    candidate.run_id,
                    candidate.instance.path().clone(),
                )
                .await
                .expect("snapshot")
                .expect("path");
            let mut tx = store
                .begin_path(prepared, Lock::Wait)
                .await
                .expect("entry")
                .expect("run");
            assert!(
                tx.record_scoped_event_delivery(&unmatched, &event)
                    .await
                    .expect("unmatched candidate")
                    .is_none()
            );
            assert!(
                tx.record_scoped_event_delivery(&candidate, &event)
                    .await
                    .is_err(),
                "a second bounded receipt operation was accepted"
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InvalidCompletion {
    NoReceipt,
    WrongValue,
    MissingValue,
    GenericCompletion,
    LateDedupeConflict,
    LateAfterSuccess,
}

async fn invalid_completion(case: InvalidCompletion) {
    use sagat_application::definitions::Idempotency;
    use sagat_domain::execution::{AddressedRunEvent, RunEvent};
    use sha2::Digest as _;
    for full in [false, true] {
        let (fixture, store, candidate, event) =
            source(&format!("event-invalid-{case:?}-{full}").to_lowercase()).await;
        let idempotency = Idempotency {
            scope: "scoped-event-receipt-test".into(),
            key: "existing".into(),
            fingerprint: b"request".to_vec(),
        };
        if matches!(
            case,
            InvalidCompletion::LateDedupeConflict | InvalidCompletion::LateAfterSuccess
        ) {
            sqlx::query("INSERT INTO command_dedupe (tenant_id,scope,idempotency_key,request_hash,resource_id) VALUES ($1,$2,$3,$4,'already-recorded')")
                .bind(candidate.tenant_id.as_uuid()).bind(&idempotency.scope).bind(&idempotency.key).bind(sha2::Sha256::digest(&idempotency.fingerprint).to_vec()).execute(&fixture.world.db.pool).await.expect("actual non-aborting late conflict");
        }
        let before = guards::snapshot(&fixture).await;
        macro_rules! refuse {
            ($tx:ident, $progress:expr, $document:ident) => {{
                let progress = $progress;
                let events = if case == InvalidCompletion::GenericCompletion {
                    progress.events.into_iter().map(|addressed| {
                        if matches!(addressed.event(), RunEvent::WaitSatisfied { .. }) {
                            AddressedRunEvent::in_path(candidate.instance.path().clone(), RunEvent::StepCompleted { node: candidate.instance.node().clone(), attempt: 0, generation: 0 }).expect("syntactically addressed generic completion")
                        } else { addressed }
                    }).collect::<Vec<_>>()
                } else { progress.events };
                let outputs = match case {
                    InvalidCompletion::MissingValue => vec![],
                    InvalidCompletion::WrongValue => vec![(candidate.instance.clone(), b"{}".to_vec())],
                    _ => vec![(candidate.instance.clone(), $document)],
                };
                let mut effects = Effects::by(Actor::System("event-dispatcher"), None);
                if case == InvalidCompletion::LateDedupeConflict {
                    effects.idempotency = Some((idempotency.clone(), "new-ack".into()));
                }
                if case == InvalidCompletion::LateAfterSuccess {
                    $tx.persist_addressed(&events, &outputs, effects).await.expect("first complete delivery persist");
                    let mut later = Effects::by(Actor::System("event-dispatcher"), None);
                    later.audit = Some(sagat_domain::audit::AuditRecord::engine(candidate.tenant_id, sagat_domain::audit::AuditAction::RunSignalled, sagat_domain::audit::ResourceType::Run, candidate.run_id.to_string(), "failed second persist must not commit", None, $tx.now()));
                    later.idempotency = Some((idempotency.clone(), "new-ack".into()));
                    assert!($tx.persist_addressed(&[], &[], later).await.is_err(), "late second persistence must hit its real conflict");
                } else {
                    assert!($tx.persist_addressed(&events, &outputs, effects).await.is_err(), "invalid native completion accepted: {case:?}, full={full}");
                }
                if matches!(case, InvalidCompletion::LateDedupeConflict | InvalidCompletion::LateAfterSuccess) {
                    assert_eq!($tx.replay(&idempotency).await.expect("SQL transaction remains usable"), Some("already-recorded".into()));
                }
                if case == InvalidCompletion::NoReceipt { drop($tx); }
                else { assert!($tx.commit().await.is_err(), "ignoring persistence failure must not commit a receipt: {case:?}, full={full}"); }
            }};
        }
        if full {
            let mut tx = store
                .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
                .await
                .expect("entry")
                .expect("run");
            let document = if case == InvalidCompletion::NoReceipt {
                b"{}".to_vec()
            } else {
                tx.record_scoped_event_delivery(&candidate, &event)
                    .await
                    .expect("staged receipt")
                    .expect("new")
            };
            let progress = {
                let (definition, state) = tx.parts();
                execution::satisfy_event_wait_instance(
                    definition,
                    state,
                    &candidate.instance,
                    &event.envelope.event_type,
                    event.envelope.correlation_key.as_ref().expect("key"),
                    event.received_at,
                )
                .expect("pure matching outcome")
            };
            refuse!(tx, progress, document);
        } else {
            let prepared = store
                .load_path(
                    candidate.tenant_id,
                    candidate.run_id,
                    candidate.instance.path().clone(),
                )
                .await
                .expect("snapshot")
                .expect("path");
            let mut tx = store
                .begin_path(prepared, Lock::Wait)
                .await
                .expect("entry")
                .expect("run");
            let document = if case == InvalidCompletion::NoReceipt {
                b"{}".to_vec()
            } else {
                tx.record_scoped_event_delivery(&candidate, &event)
                    .await
                    .expect("staged receipt")
                    .expect("new")
            };
            let progress = {
                let prepared = tx.snapshot();
                execution::satisfy_event_wait_path_instance(
                    &prepared.definition,
                    &mut prepared.state,
                    &candidate.instance,
                    &event.envelope.event_type,
                    event.envelope.correlation_key.as_ref().expect("key"),
                    event.received_at,
                )
                .expect("pure matching outcome")
            };
            refuse!(tx, progress, document);
        }
        assert_eq!(
            guards::snapshot(&fixture).await,
            before,
            "rollback of all value/outcome/usage/audit/outbox projections: {case:?}, full={full}"
        );
        let receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2",
        )
        .bind(candidate.tenant_id.as_uuid())
        .bind(candidate.run_id.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("receipt rollback");
        assert_eq!(receipts, 0);
    }
}

#[tokio::test]
async fn satisfaction_without_a_current_receipt_is_refused_by_both_writers() {
    invalid_completion(InvalidCompletion::NoReceipt).await;
}
#[tokio::test]
async fn satisfaction_with_another_value_is_refused_by_both_writers() {
    invalid_completion(InvalidCompletion::WrongValue).await;
}
#[tokio::test]
async fn satisfaction_without_a_value_is_refused_by_both_writers() {
    invalid_completion(InvalidCompletion::MissingValue).await;
}
#[tokio::test]
async fn a_generic_completion_cannot_consume_a_scoped_event_receipt() {
    invalid_completion(InvalidCompletion::GenericCompletion).await;
}
#[tokio::test]
async fn a_non_aborting_late_persist_error_cannot_commit_a_receipt_or_its_value() {
    invalid_completion(InvalidCompletion::LateDedupeConflict).await;
}

async fn probe(
    store: &PgRunStore,
    candidate: &WaitingStep,
    event: &ClaimedEvent,
    full: bool,
) -> Result<Option<Vec<u8>>, sagat_application::error::StoreError> {
    if full {
        let mut tx = store
            .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
            .await?
            .expect("run");
        tx.record_scoped_event_delivery(candidate, event).await
    } else {
        let prepared = store
            .load_path(
                candidate.tenant_id,
                candidate.run_id,
                candidate.instance.path().clone(),
            )
            .await?
            .expect("path");
        let mut tx = store.begin_path(prepared, Lock::Wait).await?.expect("run");
        tx.record_scoped_event_delivery(candidate, event).await
    }
}

#[tokio::test]
async fn a_claim_snapshot_cannot_replace_current_admission_fields_in_either_writer() {
    let (fixture, store, candidate, event) = source("event-current-admission").await;
    let before = guards::snapshot(&fixture).await;
    for full in [false, true] {
        for field in [
            "type",
            "key",
            "subject",
            "payload",
            "occurred",
            "received",
            "source",
            "id",
            "tenant",
            "namespace",
        ] {
            let mut stale = event.clone();
            match field {
                "type" => {
                    stale.envelope.event_type = EventType::parse("another.type").expect("type")
                }
                "key" => {
                    stale.envelope.correlation_key =
                        Some(execution::CorrelationKey::parse("another").expect("key"))
                }
                "subject" => stale.envelope.subject = Some("another".into()),
                "payload" => stale.payload = b"{}".to_vec(),
                "occurred" => stale.occurred_at = Some(event.received_at),
                "received" => stale.received_at += time::Duration::microseconds(1),
                "source" => {
                    stale.envelope.key.source =
                        sagat_domain::identity::Slug::parse("another-source").expect("source")
                }
                "id" => stale.envelope.key.event_id = EventId::parse("another-event").expect("id"),
                "tenant" => stale.tenant_id = fixture.world.other_admin.tenant_id(),
                "namespace" => stale.namespace_id = fixture.world.other_namespace.id(),
                _ => unreachable!(),
            }
            assert!(
                probe(&store, &candidate, &stale, full)
                    .await
                    .expect("stale candidate is no work")
                    .is_none(),
                "stale {field}, full={full}"
            );
        }
    }
    assert_eq!(guards::snapshot(&fixture).await, before);
    assert!(
        probe(&store, &candidate, &event, false)
            .await
            .expect("original admission still valid")
            .is_some()
    );
}

#[tokio::test]
async fn a_duplicate_receipt_is_no_work_only_with_all_exact_instance_metadata() {
    let (fixture, store, candidate, event) = source("event-exact-receipt-conflict").await;
    let consumer = format!("wait-instance:{}/{}", candidate.run_id, candidate.step_id);
    sqlx::query("INSERT INTO event_deliveries (tenant_id,namespace_id,source,event_id,consumer,run_id,node,delivered_at,instance_id,instance_path_nodes,instance_path_indices) VALUES ($1,$2,$3,$4,$5,$6,'b',clock_timestamp(),$7,ARRAY['items'],ARRAY[0])")
        .bind(candidate.tenant_id.as_uuid()).bind(candidate.namespace_id.as_uuid()).bind(event.envelope.key.source.as_str()).bind(event.envelope.key.event_id.as_str()).bind(&consumer).bind(candidate.run_id.as_uuid()).bind(candidate.step_id).execute(&fixture.world.db.pool).await.expect("owner-created duplicate/corruption fixture");
    for full in [false, true] {
        assert!(
            probe(&store, &candidate, &event, full)
                .await
                .expect("exact duplicate")
                .is_none()
        );
    }
    let mutations = [
        (
            "UPDATE event_deliveries SET instance_id='00000000-0000-0000-0000-000000000001' WHERE consumer=$1",
            "UPDATE event_deliveries SET instance_id=$2 WHERE consumer=$1",
        ),
        (
            "UPDATE event_deliveries SET instance_path_indices=ARRAY[1] WHERE consumer=$1",
            "UPDATE event_deliveries SET instance_path_indices=ARRAY[0] WHERE consumer=$1 AND instance_id=$2",
        ),
        (
            "UPDATE event_deliveries SET instance_path_nodes=ARRAY['another'] WHERE consumer=$1",
            "UPDATE event_deliveries SET instance_path_nodes=ARRAY['items'] WHERE consumer=$1 AND instance_id=$2",
        ),
        (
            "UPDATE event_deliveries SET node='another' WHERE consumer=$1",
            "UPDATE event_deliveries SET node='b' WHERE consumer=$1 AND instance_id=$2",
        ),
        (
            "UPDATE event_deliveries SET run_id=NULL WHERE consumer=$1",
            "UPDATE event_deliveries SET run_id=$3 WHERE consumer=$1 AND instance_id=$2",
        ),
    ];
    for (mutate, restore) in mutations {
        sqlx::query(mutate)
            .bind(&consumer)
            .execute(&fixture.world.db.pool)
            .await
            .expect("CHECK-valid owner receipt corruption");
        for full in [false, true] {
            assert!(
                probe(&store, &candidate, &event, full).await.is_err(),
                "malformed receipt was accepted: {mutate}, full={full}"
            );
        }
        // Fixed SQL vocabulary; each restore binds only the parameters its literal uses.
        if restore.contains("$3") {
            sqlx::query(restore)
                .bind(&consumer)
                .bind(candidate.step_id)
                .bind(candidate.run_id.as_uuid())
                .execute(&fixture.world.db.pool)
                .await
                .expect("restore receipt");
        } else {
            sqlx::query(restore)
                .bind(&consumer)
                .bind(candidate.step_id)
                .execute(&fixture.world.db.pool)
                .await
                .expect("restore receipt");
        }
    }
}

#[tokio::test]
async fn a_later_failed_persist_cannot_commit_an_already_staged_successful_delivery() {
    invalid_completion(InvalidCompletion::LateAfterSuccess).await;
}

#[tokio::test]
async fn a_retained_event_uses_its_pinned_body_schema_for_public_pii_and_secret_disclosure() {
    use sagat_application::classification::ClassifiedNode;
    use sagat_application::payload_disclosure::{
        DisclosureSelector, PayloadDisclosureResult, PayloadDisclosureService,
        PayloadDisclosureUseCase,
    };
    use sagat_domain::classification::Level;
    for full in [false, true] {
        for level in ["secret", "pii", "public"] {
            let name = format!("event-disclosure-{level}-{full}");
            let mut document = wait_document(&name, true, 60000);
            document["bodies"]["line"]["nodes"][1]["outputSchema"] =
                json!({"type":"object","x-sagat-classification":level});
            let fixture = scoped_worker::fixture_with_classified_document(&name, document).await;
            let (fixture, store, candidate, event) = source_fixture(fixture).await;
            let retained = deliver(&store, &candidate, &event, full).await;
            assert!(
                String::from_utf8(retained)
                    .expect("canonical JSON")
                    .contains("scope-event")
            );
            let sequence:i64=sqlx::query_scalar("SELECT seq FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='wait.satisfied' AND node='b' AND instance_path_indices=ARRAY[0]").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("actual value-bearing scoped journal sequence");
            let service = PayloadDisclosureService::new(
                store.clone(),
                sagat_application::clock::SystemClock,
                sagat_infra_schema::classification::Classifier::default(),
                sagat_infra_ir::JsonPayloadProjector,
            );
            let prepared = service
                .prepare(
                    &fixture.world.admin,
                    candidate.run_id,
                    DisclosureSelector::Journal {
                        sequence: u64::try_from(sequence).expect("sequence"),
                    },
                    false,
                    &crate::support::engine::ctx(),
                )
                .await
                .expect("current read authority");
            let answer = service
                .disclose(prepared)
                .await
                .expect("actual classified native journal read");
            let PayloadDisclosureResult::Disclosed { projection, .. } = answer.result else {
                panic!("current pinned carrier must classify the real value: {level}")
            };
            match level {
                "secret" => assert!(matches!(
                    projection.tree,
                    ClassifiedNode::Withheld {
                        level: Level::Secret,
                        ..
                    }
                )),
                "pii" => assert!(matches!(
                    projection.tree,
                    ClassifiedNode::Withheld {
                        level: Level::Pii,
                        ..
                    }
                )),
                "public" => assert!(matches!(
                    projection.tree,
                    ClassifiedNode::Container {
                        level: Level::Public,
                        ..
                    }
                )),
                _ => unreachable!(),
            }
            let public_surface:String=sqlx::query_scalar("SELECT concat((SELECT string_agg(detail::text,'') FROM run_events WHERE tenant_id=$1 AND run_id=$2),(SELECT string_agg(reason,'') FROM audit_records WHERE tenant_id=$1),(SELECT string_agg(payload::text,'') FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2))").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("journal metadata, audits and outbox");
            assert!(
                !public_surface.contains("scope-event"),
                "event value escaped retained payload boundaries"
            );
        }
    }
}

#[tokio::test]
async fn a_successful_retry_before_receipt_staging_cannot_erase_an_earlier_persist_failure() {
    use sagat_application::definitions::Idempotency;
    use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
    use sha2::Digest as _;
    for full in [false, true] {
        let (fixture, store, candidate, event) =
            source(&format!("event-prior-failure-{full}")).await;
        let idempotency = Idempotency {
            scope: "event-prior-failure".into(),
            key: "existing".into(),
            fingerprint: b"request".to_vec(),
        };
        sqlx::query("INSERT INTO command_dedupe (tenant_id,scope,idempotency_key,request_hash,resource_id) VALUES ($1,$2,$3,$4,'already-recorded')").bind(candidate.tenant_id.as_uuid()).bind(&idempotency.scope).bind(&idempotency.key).bind(sha2::Sha256::digest(&idempotency.fingerprint).to_vec()).execute(&fixture.world.db.pool).await.expect("actual non-aborting conflict");
        let before = guards::snapshot(&fixture).await;
        macro_rules! refuse_prior {
            ($tx:ident) => {{
                let mut effects = Effects::by(Actor::System("event-dispatcher"), None);
                effects.audit = Some(AuditRecord::engine(
                    candidate.tenant_id,
                    AuditAction::RunSignalled,
                    ResourceType::Run,
                    candidate.run_id.to_string(),
                    "pre-receipt failed audit must not commit",
                    None,
                    $tx.now(),
                ));
                effects.idempotency = Some((idempotency.clone(), "new-ack".into()));
                assert!($tx.persist_addressed(&[], &[], effects).await.is_err());
                $tx.persist_addressed(
                    &[],
                    &[],
                    Effects::by(Actor::System("event-dispatcher"), None),
                )
                .await
                .expect("legacy retry behavior before a receipt attempt");
                assert!(
                    $tx.record_scoped_event_delivery(&candidate, &event)
                        .await
                        .is_err(),
                    "a retry erased prior failure history, full={full}"
                );
                assert!(
                    $tx.commit().await.is_err(),
                    "receipt attempted after failure must refuse all partial writes"
                );
            }};
        }
        if full {
            let mut tx = store
                .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
                .await
                .expect("entry")
                .expect("run");
            refuse_prior!(tx);
        } else {
            let prepared = store
                .load_path(
                    candidate.tenant_id,
                    candidate.run_id,
                    candidate.instance.path().clone(),
                )
                .await
                .expect("snapshot")
                .expect("path");
            let mut tx = store
                .begin_path(prepared, Lock::Wait)
                .await
                .expect("entry")
                .expect("run");
            refuse_prior!(tx);
        }
        assert_eq!(guards::snapshot(&fixture).await, before);
    }
}

#[tokio::test]
async fn current_retained_quota_rolls_back_the_receipt_value_and_outcome_in_both_writers() {
    for full in [false, true] {
        let (fixture, store, candidate, event) =
            source(&format!("event-retained-quota-{full}")).await;
        sqlx::query("INSERT INTO quota_limits (tenant_id,namespace_id,resource,max_value,updated_by) SELECT tenant_id,NULL,'tenant_content_bytes',content_bytes+content_reserved_bytes,'test' FROM tenant_retained_usage WHERE tenant_id=$1").bind(candidate.tenant_id.as_uuid()).execute(&fixture.world.db.pool).await.expect("current effective content quota");
        let before = guards::snapshot(&fixture).await;
        let error = if full {
            let mut tx = store
                .begin(candidate.tenant_id, candidate.run_id, Lock::Wait)
                .await
                .expect("entry")
                .expect("run");
            assert!(
                persist_full_delivery(&mut tx, &candidate, &event)
                    .await
                    .is_some()
            );
            tx.commit()
                .await
                .expect_err("current quota refuses all deferred writes")
        } else {
            let prepared = store
                .load_path(
                    candidate.tenant_id,
                    candidate.run_id,
                    candidate.instance.path().clone(),
                )
                .await
                .expect("snapshot")
                .expect("path");
            let mut tx = store
                .begin_path(prepared, Lock::Wait)
                .await
                .expect("entry")
                .expect("run");
            assert!(
                persist_path_delivery(&mut tx, &candidate, &event)
                    .await
                    .is_some()
            );
            tx.commit()
                .await
                .expect_err("current quota refuses all deferred writes")
        };
        assert!(
            matches!(
                error,
                sagat_application::error::StoreError::QuotaExceeded { .. }
            ),
            "typed quota refusal: {error:?}"
        );
        assert_eq!(guards::snapshot(&fixture).await, before);
        let receipts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2",
        )
        .bind(candidate.tenant_id.as_uuid())
        .bind(candidate.run_id.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("no orphan receipt");
        assert_eq!(receipts, 0);
    }
}

#[tokio::test]
async fn paused_delivery_cold_resume_and_duplicate_keep_one_value_outcome_and_logical_charge() {
    use sagat_application::runs::RunOperation;
    for full in [false, true] {
        let (fixture, store, candidate, event) =
            source(&format!("event-paused-replay-{full}")).await;
        let before: i64 = sqlx::query_scalar(
            "SELECT logical_output_bytes FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2",
        )
        .bind(candidate.tenant_id.as_uuid())
        .bind(candidate.run_id.as_uuid())
        .fetch_one(&fixture.world.db.pool)
        .await
        .expect("initial logical charge");
        timeouts::operate(
            &fixture,
            &store,
            RunOperation::Pause,
            "pause-before-delivery",
        )
        .await;
        let document = deliver(&store, &candidate, &event, full).await;
        let cold = PgRunStore::new(
            fixture.world.db.runtime_pool().await,
            DefinitionCache::new(Arc::new(StructuredJsonIrCodec), 8),
        )
        .with_scoped_worker_protocol();
        let actual:(String,bool,bool)=sqlx::query_as("SELECT r.status,s.routed,s.timed_out FROM runs r JOIN steps s ON s.tenant_id=r.tenant_id AND s.run_id=r.id WHERE r.tenant_id=$1 AND r.id=$2 AND s.node='b' AND s.instance_path_indices=ARRAY[0]").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("paused value with deferred route");
        assert_eq!(actual, ("PAUSED".into(), false, false));
        cold.verify_run(candidate.tenant_id, candidate.run_id)
            .await
            .expect("paused cold replay");
        assert!(
            probe(&cold, &candidate, &event, full)
                .await
                .expect("post-commit duplicate")
                .is_none()
        );
        timeouts::operate(
            &fixture,
            &cold,
            RunOperation::Resume,
            "resume-after-delivery",
        )
        .await;
        let prepared = cold
            .load_path(
                candidate.tenant_id,
                candidate.run_id,
                candidate.instance.path().clone(),
            )
            .await
            .expect("cold source")
            .expect("path");
        let mut tx = cold
            .begin_path(prepared, Lock::Wait)
            .await
            .expect("cold entry")
            .expect("run");
        let lease = fixture
            .placement
            .snapshot()
            .lease_of(
                Shard::of(candidate.run_id.as_uuid()),
                std::time::Instant::now(),
            )
            .expect("own shard");
        assert!(
            tx.holds_shard("scoped-worker-test", lease)
                .await
                .expect("current scheduler authority")
        );
        let progress = {
            let prepared = tx.snapshot();
            execution::release_pending_path(&prepared.definition, &mut prepared.state)
                .expect("explicit cold deferred routing")
        };
        tx.persist_addressed(
            &progress.events,
            &[],
            Effects::by(Actor::System("scheduler"), None),
        )
        .await
        .expect("native cold route");
        tx.commit().await.expect("one route commit");
        let final_value:(Vec<u8>,bool,i64)=sqlx::query_as("SELECT p.content,s.routed,u.logical_output_bytes FROM steps s JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest JOIN run_instance_usage u ON u.tenant_id=s.tenant_id AND u.run_id=s.run_id WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='b' AND s.instance_path_indices=ARRAY[0]").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("one retained cold value and charge");
        assert_eq!(
            final_value,
            (
                document.clone(),
                true,
                before + i64::try_from(document.len()).expect("bytes")
            )
        );
        let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2),count(*) FILTER(WHERE event_type='wait.satisfied'),count(*) FILTER(WHERE event_type='step.routed' AND node='b' AND instance_path_indices=ARRAY[0]) FROM run_events WHERE tenant_id=$1 AND run_id=$2").bind(candidate.tenant_id.as_uuid()).bind(candidate.run_id.as_uuid()).fetch_one(&fixture.world.db.pool).await.expect("one receipt, outcome and route");
        assert_eq!(counts, (1, 1, 1));
        cold.verify_run(candidate.tenant_id, candidate.run_id)
            .await
            .expect("resumed cold replay");
        assert!(scoped_worker::poll(&fixture, 8).await.is_empty());
    }
}
