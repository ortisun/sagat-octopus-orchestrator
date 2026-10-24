#![allow(clippy::expect_used, clippy::unwrap_used)] // test fixtures/assertions fail immediately

use std::collections::BTreeMap;

use sagat_application::clock::Clock;
use sagat_application::error::StoreError;
use sagat_application::event_payloads::*;
use sagat_application::identity::RequestContext;
use sagat_application::payloads::{
    PayloadProjection, PayloadProjector, PayloadStructure, StoredPayloadBytes,
    UnsupportedPayloadFormat,
};
use sagat_domain::audit::{AuditAction, AuditRecord};
use sagat_domain::connectors::{ConnectorId, DeadLetter};
use sagat_domain::events::{EventEnvelope, EventId, EventKey, InboxStatus};
use sagat_domain::identity::{NamespaceId, Principal, Role, Slug, Subject, TenantId};
use sagat_domain::workflow::EventType;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone, Copy)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH
    }
}
struct Projector;
impl PayloadProjector for Projector {
    fn project(&self, _: &[u8]) -> Result<PayloadProjection, UnsupportedPayloadFormat> {
        Ok(PayloadProjection {
            tree: PayloadStructure::Object {
                child_count: 1,
                children: vec![PayloadStructure::String],
                omitted_children: 0,
            },
            truncated: false,
        })
    }
}

struct OpaqueProjector;
impl PayloadProjector for OpaqueProjector {
    fn project(&self, _: &[u8]) -> Result<PayloadProjection, UnsupportedPayloadFormat> {
        Err(UnsupportedPayloadFormat)
    }
}

#[derive(Clone)]
struct Store {
    role: Role,
    grant: bool,
    fail_audit: bool,
    audits: std::sync::Arc<std::sync::Mutex<Vec<AuditRecord>>>,
    lookups: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl Store {
    fn new(role: Role, grant: bool) -> Self {
        Self {
            role,
            grant,
            fail_audit: false,
            audits: std::sync::Arc::default(),
            lookups: std::sync::Arc::default(),
        }
    }

    fn write_audit(&self, record: &AuditRecord) -> Result<(), StoreError> {
        if self.fail_audit {
            return Err(StoreError::Unavailable {
                detail: "injected audit failure".into(),
            });
        }
        self.audits
            .lock()
            .expect("audit records")
            .push(record.clone());
        Ok(())
    }
}
impl EventPayloadStore for Store {
    async fn event_scope(
        &self,
        _: TenantId,
        n: NamespaceId,
        _: &EventKey,
    ) -> Result<Option<EventPayloadScope>, StoreError> {
        self.lookups
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Some(EventPayloadScope { namespace_id: n }))
    }
    async fn dead_letter_scope(
        &self,
        _: TenantId,
        n: NamespaceId,
        _: ConnectorId,
        _: Option<Uuid>,
    ) -> Result<Option<EventPayloadScope>, StoreError> {
        self.lookups
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Some(EventPayloadScope { namespace_id: n }))
    }
    async fn read_event(
        &self,
        _: &PreparedInboxEvent,
        e: &dyn EventPayloadEvaluator,
        audits: &EventPayloadAudits,
    ) -> Result<EventPayloadRead<InboxEventRead>, StoreError> {
        Ok(match e.event(event(), self.role, self.grant) {
            Ok(v) => {
                if matches!(&v.data, EventData::Revealed(_)) {
                    self.write_audit(&audits.revealed)?;
                }
                EventPayloadRead::Ready(v)
            }
            Err(v) => EventPayloadRead::Refused(v),
        })
    }
    async fn read_dead_letter(
        &self,
        _: &PreparedDeadLetter,
        e: &dyn EventPayloadEvaluator,
        audits: &EventPayloadAudits,
    ) -> Result<EventPayloadRead<DeadLetterRead>, StoreError> {
        Ok(match e.dead_letter(letter(true), self.role, self.grant) {
            Ok(v) => {
                if matches!(&v.data, DeadLetterData::Revealed { .. }) {
                    self.write_audit(&audits.revealed)?;
                }
                EventPayloadRead::Ready(v)
            }
            Err(v) => EventPayloadRead::Refused(v),
        })
    }
    async fn list_dead_letters(
        &self,
        _: &PreparedDeadLetter,
        _: Option<Uuid>,
        _: u32,
        e: &dyn EventPayloadEvaluator,
    ) -> Result<EventPayloadRead<Vec<DeadLetterRead>>, StoreError> {
        Ok(match e.dead_letter(letter(false), self.role, self.grant) {
            Ok(v) => EventPayloadRead::Ready(vec![v]),
            Err(v) => EventPayloadRead::Refused(v),
        })
    }
}

fn principal(role: Role) -> Principal {
    Principal::new(
        TenantId::new(Uuid::new_v4()),
        Subject::parse("reader").unwrap(),
        role,
    )
}
fn namespace() -> NamespaceId {
    NamespaceId::new(Uuid::new_v4())
}
fn key() -> EventKey {
    EventKey {
        source: Slug::parse("source-a").unwrap(),
        event_id: EventId::parse("evt-1").unwrap(),
    }
}
fn ctx() -> RequestContext {
    RequestContext {
        trace_id: None,
        span_id: None,
    }
}
fn event() -> SelectedInboxEvent {
    SelectedInboxEvent {
        envelope: EventEnvelope::new(
            key(),
            EventType::parse("invoice.created").unwrap(),
            None,
            None,
        )
        .unwrap(),
        occurred_at: None,
        payload: StoredPayloadBytes::new(br#"{"canary":"event-secret"}"#.to_vec()),
        received_at: OffsetDateTime::UNIX_EPOCH,
        expires_at: OffsetDateTime::UNIX_EPOCH,
        status: InboxStatus::Pending,
        attempts: 0,
        last_error: None,
        dispatched_at: None,
        deliveries: vec![],
    }
}
fn letter(truncated: bool) -> SelectedDeadLetter {
    SelectedDeadLetter {
        id: Uuid::new_v4(),
        connector_id: ConnectorId::new(Uuid::new_v4()),
        position: "1".into(),
        reason: DeadLetter::BadEnvelope,
        headers: BTreeMap::from([("authorization".into(), "letter-secret".into())]),
        body: StoredPayloadBytes::new(br#"{"canary":"body-secret"}"#.to_vec()),
        truncated,
        received_at: OffsetDateTime::UNIX_EPOCH,
    }
}

#[tokio::test]
async fn default_event_read_is_structure_only_and_debug_redacts_values() {
    let service =
        EventPayloadService::new(Store::new(Role::Developer, false), FixedClock, Projector);
    let p = principal(Role::Developer);
    let prepared = service
        .prepare_event(&p, namespace(), key(), EventReveal::Structure, &ctx())
        .await
        .unwrap();
    let read = service.event(prepared).await.unwrap();
    assert!(matches!(read.data, EventData::Structure(_)));
    let debug = format!("{read:?}");
    assert!(!debug.contains("event-secret"));
    assert!(debug.contains("size_bytes"));
}

#[tokio::test]
async fn event_and_letter_values_require_the_mandatory_audit_write_to_succeed() {
    for fail_audit in [false, true] {
        let mut store = Store::new(Role::Developer, true);
        store.fail_audit = fail_audit;
        let audits = std::sync::Arc::clone(&store.audits);
        let service = EventPayloadService::new(store, FixedClock, Projector);
        let caller = principal(Role::Developer);
        let prepared = service
            .prepare_event(&caller, namespace(), key(), EventReveal::Pii, &ctx())
            .await
            .expect("prepared event reveal");
        let event = service.event(prepared).await;
        assert_eq!(event.is_ok(), !fail_audit);
        assert_eq!(
            audits.lock().expect("audits").len(),
            usize::from(!fail_audit)
        );
        let prepared = service
            .prepare_dead_letter(
                &caller,
                namespace(),
                ConnectorId::new(Uuid::new_v4()),
                Some(Uuid::new_v4()),
                EventReveal::Pii,
                &ctx(),
            )
            .await
            .expect("prepared letter reveal");
        let letter = service.dead_letter(prepared).await;
        assert_eq!(letter.is_ok(), !fail_audit);
        let records = audits.lock().expect("audits");
        assert_eq!(records.len(), 2 * usize::from(!fail_audit));
        for record in &*records {
            assert_eq!(record.action(), AuditAction::PayloadPiiRevealed);
            assert_eq!(record.actor(), caller.subject());
            assert_eq!(record.tenant_id(), caller.tenant_id());
        }
        for debug in [format!("{event:?}"), format!("{letter:?}")] {
            for canary in ["event-secret", "letter-secret", "body-secret"] {
                assert!(!debug.contains(canary), "Debug leaked a payload value");
            }
        }
    }
}

#[tokio::test]
async fn an_unsupported_list_body_is_opaque_without_revealing_headers_or_auditing() {
    let store = Store::new(Role::Developer, true);
    let audits = std::sync::Arc::clone(&store.audits);
    let service = EventPayloadService::new(store, FixedClock, OpaqueProjector);
    let caller = principal(Role::Developer);
    let prepared = service
        .prepare_dead_letter(
            &caller,
            namespace(),
            ConnectorId::new(Uuid::new_v4()),
            None,
            EventReveal::Pii,
            &ctx(),
        )
        .await
        .expect("prepared list");
    let rows = service
        .dead_letters(prepared, None, 200)
        .await
        .expect("structure list");
    assert_eq!(rows.len(), 1);
    assert!(matches!(
        rows[0].data,
        DeadLetterData::Structure(DeadLetterStructure::Opaque)
    ));
    assert!(rows[0].letter.headers.is_empty());
    assert!(rows[0].letter.body.as_slice().is_empty());
    assert!(audits.lock().expect("audits").is_empty());
}

#[tokio::test]
async fn pii_reveal_requires_internal_role_and_namespace_grant() {
    for (role, grant) in [(Role::Viewer, true), (Role::Developer, false)] {
        let service = EventPayloadService::new(Store::new(role, grant), FixedClock, Projector);
        let p = principal(role);
        let prepared = service
            .prepare_event(&p, namespace(), key(), EventReveal::Pii, &ctx())
            .await
            .unwrap();
        assert!(matches!(
            service.event(prepared).await,
            Err(EventPayloadError::PiiRevealNotGranted)
        ));
    }
    let service =
        EventPayloadService::new(Store::new(Role::Developer, true), FixedClock, Projector);
    let p = principal(Role::Developer);
    let prepared = service
        .prepare_event(&p, namespace(), key(), EventReveal::Pii, &ctx())
        .await
        .unwrap();
    assert!(matches!(
        service.event(prepared).await.unwrap().data,
        EventData::Revealed(_)
    ));
    for role in [Role::Operator, Role::Admin] {
        let service = EventPayloadService::new(Store::new(role, true), FixedClock, Projector);
        let principal = principal(role);
        let prepared = service
            .prepare_event(&principal, namespace(), key(), EventReveal::Pii, &ctx())
            .await
            .unwrap();
        assert!(matches!(
            service.event(prepared).await.unwrap().data,
            EventData::Revealed(_)
        ));
    }
}

#[tokio::test]
async fn truncated_dead_letter_is_opaque_and_list_ignores_reveal() {
    let service = EventPayloadService::new(Store::new(Role::Admin, true), FixedClock, Projector);
    let p = principal(Role::Admin);
    let connector = ConnectorId::new(Uuid::new_v4());
    let detail = service
        .prepare_dead_letter(
            &p,
            namespace(),
            connector,
            Some(Uuid::new_v4()),
            EventReveal::Structure,
            &ctx(),
        )
        .await
        .unwrap();
    assert!(matches!(
        service.dead_letter(detail).await.unwrap().data,
        DeadLetterData::Structure(DeadLetterStructure::Opaque)
    ));
    let list = service
        .prepare_dead_letter(&p, namespace(), connector, None, EventReveal::Pii, &ctx())
        .await
        .unwrap();
    let item = service
        .dead_letters(list, None, 50)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(matches!(item.data, DeadLetterData::Structure(_)));
    let debug = format!("{item:?}");
    assert!(!debug.contains("letter-secret"));
    assert!(!debug.contains("body-secret"));
}

#[tokio::test]
async fn roles_without_event_read_are_refused_before_lookup() {
    for role in [Role::Worker, Role::Runner, Role::DatabaseRunner] {
        let store = Store::new(role, true);
        let lookups = std::sync::Arc::clone(&store.lookups);
        let service = EventPayloadService::new(store, FixedClock, Projector);
        let p = principal(role);
        assert!(
            service
                .prepare_event(&p, namespace(), key(), EventReveal::Structure, &ctx())
                .await
                .is_err()
        );
        assert_eq!(lookups.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
