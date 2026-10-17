#![allow(clippy::expect_used)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use sagat_application::authz::NamespaceDirectory;
use sagat_application::clock::Clock;
use sagat_application::error::StoreError;
use sagat_application::identity::RequestContext;
use sagat_application::reveal_grants::{
    GrantRead, GrantRevoke, GrantWrite, RevealGrant, RevealGrantError, RevealGrantService,
    RevealGrantStore,
};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{NamespaceId, Principal, Role, Subject, TenantId};
use time::{Date, Month, OffsetDateTime, Time, UtcOffset};

#[derive(Clone, Copy)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> OffsetDateTime {
        Date::from_calendar_date(2026, Month::October, 17)
            .expect("date")
            .with_time(Time::from_hms(12, 0, 0).expect("time"))
            .assume_offset(UtcOffset::UTC)
    }
}

#[derive(Clone, Default)]
struct FakeStore {
    namespace_exists: bool,
    outside: bool,
    grants: Arc<Mutex<Vec<RevealGrant>>>,
    forbid_reads: bool,
    writes: Arc<Mutex<VecDeque<GrantWrite>>>,
    revoke: Option<GrantRevoke>,
    audits: Arc<Mutex<Vec<AuditRecord>>>,
}

impl NamespaceDirectory for FakeStore {
    async fn namespace_exists(
        &self,
        _tenant: TenantId,
        _namespace: NamespaceId,
    ) -> Result<bool, StoreError> {
        Ok(self.namespace_exists)
    }

    async fn namespace_exists_outside(
        &self,
        _tenant: TenantId,
        _namespace: NamespaceId,
    ) -> Result<bool, StoreError> {
        Ok(self.outside)
    }

    async fn record_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        self.audits.lock().expect("audits").push(audit.clone());
        Ok(())
    }
}

impl RevealGrantStore for FakeStore {
    async fn reveal_grants(
        &self,
        _tenant: TenantId,
        _namespace: NamespaceId,
        _actor: &Subject,
        after: Option<&Subject>,
        limit: u32,
    ) -> Result<GrantRead, StoreError> {
        if self.forbid_reads {
            return Ok(GrantRead::ActorForbidden);
        }
        Ok(GrantRead::Authorized(
            self.grants
                .lock()
                .expect("grants")
                .iter()
                .filter(|grant| after.is_none_or(|after| grant.subject > *after))
                .take(limit as usize)
                .cloned()
                .collect(),
        ))
    }

    async fn grant_reveal(
        &self,
        _tenant: TenantId,
        _namespace: NamespaceId,
        _actor: &Subject,
        _subject: &Subject,
        audit: &AuditRecord,
    ) -> Result<GrantWrite, StoreError> {
        self.audits.lock().expect("audits").push(audit.clone());
        Ok(self
            .writes
            .lock()
            .expect("writes")
            .pop_front()
            .expect("configured write"))
    }

    async fn revoke_reveal(
        &self,
        _tenant: TenantId,
        _namespace: NamespaceId,
        _actor: &Subject,
        _subject: &Subject,
        audit: &AuditRecord,
    ) -> Result<GrantRevoke, StoreError> {
        self.audits.lock().expect("audits").push(audit.clone());
        Ok(self.revoke.unwrap_or(GrantRevoke::Removed))
    }
}

fn subject(value: &str) -> Subject {
    Subject::parse(value).expect("subject")
}

fn principal(role: Role, name: &str) -> Principal {
    Principal::new(TenantId::generate(), subject(name), role)
}

fn grant(name: &str) -> RevealGrant {
    RevealGrant {
        subject: subject(name),
        granted_by: subject("admin"),
        granted_at: FixedClock.now(),
    }
}

fn service(store: FakeStore) -> RevealGrantService<FakeStore, FixedClock> {
    RevealGrantService::new(store, FixedClock)
}

#[tokio::test]
async fn only_membership_managers_prepare_a_namespace() {
    let store = FakeStore {
        namespace_exists: true,
        ..FakeStore::default()
    };
    let error = service(store)
        .prepare(
            &principal(Role::Developer, "developer"),
            NamespaceId::generate(),
            &RequestContext::default(),
        )
        .await
        .expect_err("developer cannot manage grants");

    assert!(matches!(
        error,
        sagat_application::error::AppError::Forbidden { .. }
    ));
}

#[tokio::test]
async fn grant_maps_closed_store_outcomes_and_allows_self_grant() {
    for (outcome, expected) in [
        (GrantWrite::UnknownSubject, RevealGrantError::UnknownSubject),
        (
            GrantWrite::RoleCannotReveal,
            RevealGrantError::RoleCannotReveal,
        ),
        (GrantWrite::LimitReached, RevealGrantError::GrantLimit),
    ] {
        let store = FakeStore {
            namespace_exists: true,
            writes: Arc::new(Mutex::new(VecDeque::from([outcome]))),
            ..FakeStore::default()
        };
        let svc = service(store);
        let admin = principal(Role::Admin, "admin");
        let scope = svc
            .prepare(&admin, NamespaceId::generate(), &RequestContext::default())
            .await
            .expect("prepare");

        assert_eq!(
            svc.grant(&scope, "admin", &RequestContext::default())
                .await
                .expect_err("closed refusal"),
            expected
        );
    }
}

#[tokio::test]
async fn a_created_grant_uses_the_approved_audit_identity_and_fixed_reason() {
    let created = grant("developer");
    let store = FakeStore {
        namespace_exists: true,
        writes: Arc::new(Mutex::new(VecDeque::from([GrantWrite::Created(
            created.clone(),
        )]))),
        ..FakeStore::default()
    };
    let audits = Arc::clone(&store.audits);
    let svc = service(store);
    let admin = principal(Role::Admin, "admin");
    let namespace = NamespaceId::generate();
    let scope = svc
        .prepare(&admin, namespace, &RequestContext::default())
        .await
        .expect("prepare");

    let actual = svc
        .grant(&scope, "developer", &RequestContext::default())
        .await
        .expect("grant");

    assert_eq!(actual, created);
    let audit = audits.lock().expect("audits").pop().expect("audit");
    assert_eq!(audit.action(), AuditAction::PayloadRevealGranted);
    assert_eq!(audit.resource_type(), ResourceType::PayloadRevealGrant);
    assert_eq!(audit.resource_id(), format!("{namespace}/developer"));
    assert_eq!(audit.reason(), Some("granted"));
}

#[tokio::test]
async fn list_is_stable_and_returns_an_opaque_boundary_to_the_http_adapter() {
    let store = FakeStore {
        namespace_exists: true,
        grants: Arc::new(Mutex::new(vec![grant("a"), grant("b"), grant("c")])),
        ..FakeStore::default()
    };
    let svc = service(store);
    let admin = principal(Role::Admin, "admin");
    let scope = svc
        .prepare(&admin, NamespaceId::generate(), &RequestContext::default())
        .await
        .expect("prepare");

    let page = svc.list(&scope, None, 2).await.expect("list");

    assert_eq!(page.items.len(), 2);
    assert_eq!(page.next_after.as_ref().map(Subject::as_str), Some("b"));
}

#[tokio::test]
async fn revoke_is_idempotent_for_an_absent_or_malformed_subject() {
    let store = FakeStore {
        namespace_exists: true,
        ..FakeStore::default()
    };
    let svc = service(store);
    let admin = principal(Role::Admin, "admin");
    let scope = svc
        .prepare(&admin, NamespaceId::generate(), &RequestContext::default())
        .await
        .expect("prepare");

    assert!(
        !svc.revoke(&scope, "\u{7}", &RequestContext::default())
            .await
            .expect("idempotent revoke")
    );
}

#[tokio::test]
async fn a_role_snapshot_cannot_authorize_after_the_store_sees_demotion() {
    let admin = principal(Role::Admin, "admin");

    let list_store = FakeStore {
        namespace_exists: true,
        forbid_reads: true,
        ..FakeStore::default()
    };
    let list_service = service(list_store);
    let list_scope = list_service
        .prepare(&admin, NamespaceId::generate(), &RequestContext::default())
        .await
        .expect("snapshot prepared");
    assert!(matches!(
        list_service.list(&list_scope, None, 1).await,
        Err(RevealGrantError::Application(
            sagat_application::error::AppError::Forbidden { .. }
        ))
    ));

    let write_store = FakeStore {
        namespace_exists: true,
        writes: Arc::new(Mutex::new(VecDeque::from([GrantWrite::ActorForbidden]))),
        revoke: Some(GrantRevoke::ActorForbidden),
        ..FakeStore::default()
    };
    let write_service = service(write_store);
    let write_scope = write_service
        .prepare(&admin, NamespaceId::generate(), &RequestContext::default())
        .await
        .expect("snapshot prepared");
    for error in [
        write_service
            .grant(&write_scope, "developer", &RequestContext::default())
            .await
            .expect_err("demoted actor cannot grant"),
        write_service
            .revoke(&write_scope, "developer", &RequestContext::default())
            .await
            .expect_err("demoted actor cannot revoke"),
    ] {
        assert!(matches!(
            error,
            RevealGrantError::Application(sagat_application::error::AppError::Forbidden { .. })
        ));
    }
}
