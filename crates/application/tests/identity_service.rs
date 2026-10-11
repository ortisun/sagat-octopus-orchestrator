//! Use-case tests for the identity context, against an in-memory store.
//!
//! The fake mirrors the contract of `IdentityStore` (tenant filter, uniqueness, failure injection).
//! It proves the *service* logic: authorization order, audit records, error mapping, pagination.
//! SQL-level guarantees are proven separately against a real PostgreSQL (`tests/integration`).
#![allow(clippy::expect_used, clippy::significant_drop_tightening)]

use std::sync::{Arc, Mutex};

use sagat_application::clock::Clock;
use sagat_application::error::{AppError, ConflictKind, StoreError};
use sagat_application::identity::{
    IdentityService, IdentityStore, MAX_PAGE_SIZE, PageRequest, RequestContext,
};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{
    IdentityError, Membership, Namespace, NamespaceId, Permission, Principal, Role, Slug, Subject,
    Tenant, TenantId, Workspace, WorkspaceId,
};
use time::OffsetDateTime;

const T0: OffsetDateTime = OffsetDateTime::UNIX_EPOCH;

#[derive(Clone, Copy)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> OffsetDateTime {
        T0
    }
}

#[derive(Default)]
struct State {
    tenants: Vec<Tenant>,
    memberships: Vec<Membership>,
    workspaces: Vec<Workspace>,
    namespaces: Vec<Namespace>,
    audits: Vec<AuditRecord>,
    fail_audit_writes: bool,
    fail_everything: Option<StoreError>,
}

#[derive(Clone, Default)]
struct FakeStore(Arc<Mutex<State>>);

impl FakeStore {
    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.0.lock().expect("lock"))
    }

    fn check(&self) -> Result<(), StoreError> {
        self.with(|s| s.fail_everything.clone().map_or(Ok(()), Err))
    }
}

impl IdentityStore for FakeStore {
    async fn find_tenant_by_slug(&self, slug: &Slug) -> Result<Option<Tenant>, StoreError> {
        self.check()?;
        Ok(self.with(|s| s.tenants.iter().find(|t| t.slug() == slug).cloned()))
    }

    async fn find_membership(
        &self,
        tenant: TenantId,
        subject: &Subject,
    ) -> Result<Option<Membership>, StoreError> {
        self.check()?;
        Ok(self.with(|s| {
            s.memberships
                .iter()
                .find(|m| m.tenant_id() == tenant && m.subject() == subject)
                .cloned()
        }))
    }

    async fn create_workspace(
        &self,
        workspace: &Workspace,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        self.check()?;
        self.with(|s| {
            if s.workspaces
                .iter()
                .any(|w| w.tenant_id() == workspace.tenant_id() && w.slug() == workspace.slug())
            {
                return Err(StoreError::Conflict(ConflictKind::WorkspaceSlug));
            }
            s.workspaces.push(workspace.clone());
            s.audits.push(audit.clone());
            Ok(())
        })
    }

    async fn get_workspace(
        &self,
        tenant: TenantId,
        id: WorkspaceId,
    ) -> Result<Option<Workspace>, StoreError> {
        self.check()?;
        Ok(self.with(|s| {
            s.workspaces
                .iter()
                .find(|w| w.id() == id && w.tenant_id() == tenant)
                .cloned()
        }))
    }

    async fn list_workspaces(
        &self,
        tenant: TenantId,
        after: Option<WorkspaceId>,
        limit: u32,
    ) -> Result<Vec<Workspace>, StoreError> {
        self.check()?;
        Ok(self.with(|s| {
            let mut rows: Vec<_> = s
                .workspaces
                .iter()
                .filter(|w| w.tenant_id() == tenant && after.is_none_or(|a| w.id() > a))
                .cloned()
                .collect();
            rows.sort_by_key(Workspace::id);
            rows.truncate(limit as usize);
            rows
        }))
    }

    async fn create_namespace(
        &self,
        namespace: &Namespace,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        self.check()?;
        self.with(|s| {
            if s.namespaces
                .iter()
                .any(|n| n.tenant_id() == namespace.tenant_id() && n.slug() == namespace.slug())
            {
                return Err(StoreError::Conflict(ConflictKind::NamespaceSlug));
            }
            s.namespaces.push(namespace.clone());
            s.audits.push(audit.clone());
            Ok(())
        })
    }

    async fn get_namespace(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> Result<Option<Namespace>, StoreError> {
        self.check()?;
        Ok(self.with(|s| {
            s.namespaces
                .iter()
                .find(|n| n.id() == id && n.tenant_id() == tenant)
                .cloned()
        }))
    }

    async fn list_namespaces(
        &self,
        tenant: TenantId,
        after: Option<NamespaceId>,
        limit: u32,
    ) -> Result<Vec<Namespace>, StoreError> {
        self.check()?;
        Ok(self.with(|s| {
            let mut rows: Vec<_> = s
                .namespaces
                .iter()
                .filter(|n| n.tenant_id() == tenant && after.is_none_or(|a| n.id() > a))
                .cloned()
                .collect();
            rows.sort_by_key(Namespace::id);
            rows.truncate(limit as usize);
            rows
        }))
    }

    async fn workspace_exists_outside(
        &self,
        tenant: TenantId,
        id: WorkspaceId,
    ) -> Result<bool, StoreError> {
        self.check()?;
        Ok(self.with(|s| {
            s.workspaces
                .iter()
                .any(|w| w.id() == id && w.tenant_id() != tenant)
        }))
    }

    async fn namespace_exists_outside(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> Result<bool, StoreError> {
        self.check()?;
        Ok(self.with(|s| {
            s.namespaces
                .iter()
                .any(|n| n.id() == id && n.tenant_id() != tenant)
        }))
    }

    async fn record_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        self.check()?;
        self.with(|s| {
            if s.fail_audit_writes {
                return Err(StoreError::Internal {
                    detail: "audit table unavailable".into(),
                });
            }
            s.audits.push(audit.clone());
            Ok(())
        })
    }

    async fn ping(&self) -> Result<(), StoreError> {
        self.check()
    }
}

struct World {
    svc: IdentityService<FakeStore, FixedClock>,
    store: FakeStore,
    tenant_a: Tenant,
    tenant_b: Tenant,
    admin_a: Principal,
    admin_b: Principal,
}

fn slug(s: &str) -> Slug {
    Slug::parse(s).expect("slug")
}

fn subject(s: &str) -> Subject {
    Subject::parse(s).expect("subject")
}

fn ctx() -> RequestContext {
    RequestContext {
        trace_id: Some("4bf92f3577b34da6a3ce929d0e0e4736".into()),
        span_id: None,
    }
}

fn world() -> World {
    let store = FakeStore::default();
    let tenant_a = Tenant::provision(TenantId::generate(), slug("acme"), T0);
    let tenant_b = Tenant::provision(TenantId::generate(), slug("globex"), T0);
    let alice = Membership::bootstrap_admin(&tenant_a, subject("alice"), T0);
    let bob = Membership::bootstrap_admin(&tenant_b, subject("bob"), T0);
    store.with(|s| {
        s.tenants.extend([tenant_a.clone(), tenant_b.clone()]);
        s.memberships.extend([alice.clone(), bob.clone()]);
    });
    World {
        svc: IdentityService::new(store.clone(), FixedClock),
        admin_a: alice.principal(),
        admin_b: bob.principal(),
        store,
        tenant_a,
        tenant_b,
    }
}

fn grant(w: &World, tenant: &Tenant, who: &str, role: Role) -> Principal {
    let m = Membership::restore(tenant.id(), subject(who), role, T0);
    w.store.with(|s| s.memberships.push(m.clone()));
    m.principal()
}

fn audits(w: &World) -> Vec<AuditRecord> {
    w.store.with(|s| s.audits.clone())
}

// ------------------------------------------------------------- authenticate

#[tokio::test]
async fn authenticate_resolves_tenant_and_role_from_membership() {
    let w = world();
    let session = w
        .svc
        .authenticate("acme", "alice")
        .await
        .expect("authenticated");
    assert_eq!(session.principal, w.admin_a);
    assert_eq!(session.tenant, w.tenant_a);
}

#[tokio::test]
async fn authenticate_gives_the_same_answer_for_every_kind_of_failure() {
    let w = world();
    let cases = [
        ("unknown-tenant", "alice"), // tenant does not exist
        ("acme", "nobody"),          // no membership in that tenant
        ("globex", "alice"),         // member of a *different* tenant
        ("ACME", "alice"),           // malformed slug
        ("acme", ""),                // malformed subject
    ];
    for (tenant, who) in cases {
        assert_eq!(
            w.svc.authenticate(tenant, who).await,
            Err(AppError::Unauthenticated),
            "tenant={tenant:?} subject={who:?}"
        );
    }
}

#[tokio::test]
async fn a_token_cannot_authenticate_as_the_platform_actor_even_if_a_membership_exists() {
    // Review finding 5: `Subject::parse("system:provisioning")` succeeded, so an operator who
    // provisioned that subject as a tenant administrator could sign in as the audit actor.
    let w = world();
    let tenant = w.tenant_a.clone();
    w.store.with(|s| {
        s.memberships.push(Membership::bootstrap_admin(
            &tenant,
            Subject::provisioning(),
            T0,
        ));
    });

    for who in ["system:provisioning", "SYSTEM:PROVISIONING", "system:other"] {
        assert_eq!(
            w.svc.authenticate("acme", who).await,
            Err(AppError::Unauthenticated),
            "{who:?}"
        );
    }
}

#[tokio::test]
async fn authenticate_reports_a_storage_outage_as_unavailable_not_as_a_denial() {
    let w = world();
    w.store.with(|s| {
        s.fail_everything = Some(StoreError::Unavailable {
            detail: "down".into(),
        })
    });
    let err = w
        .svc
        .authenticate("acme", "alice")
        .await
        .expect_err("outage");
    assert_eq!(err, AppError::Unavailable);
    assert!(err.is_retryable());
}

// --------------------------------------------------------------- workspaces

#[tokio::test]
async fn creating_a_workspace_persists_it_with_an_audit_record_in_the_callers_tenant() {
    let w = world();

    let ws = w
        .svc
        .create_workspace(&w.admin_a, "platform", &ctx())
        .await
        .expect("created");

    assert_eq!(ws.tenant_id(), w.tenant_a.id());
    assert_eq!(ws.slug().as_str(), "platform");
    let log = audits(&w);
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].action(), AuditAction::WorkspaceCreated);
    assert_eq!(log[0].tenant_id(), w.tenant_a.id());
    assert_eq!(log[0].actor().as_str(), "alice");
    assert_eq!(log[0].resource_id(), ws.id().to_string());
    assert_eq!(log[0].trace_id(), Some("4bf92f3577b34da6a3ce929d0e0e4736"));
}

#[tokio::test]
async fn a_caller_without_permission_is_refused_before_any_storage_access() {
    let w = world();
    let viewer = grant(&w, &w.tenant_a, "vera", Role::Viewer);
    // Storage is down: if the service touched it, we would see `Unavailable`, not `Forbidden`.
    w.store.with(|s| {
        s.fail_everything = Some(StoreError::Unavailable {
            detail: "down".into(),
        })
    });

    let err = w
        .svc
        .create_workspace(&viewer, "platform", &ctx())
        .await
        .expect_err("denied");

    assert_eq!(
        err,
        AppError::Forbidden {
            permission: Permission::WorkspaceCreate
        }
    );
}

#[tokio::test]
async fn authorization_is_evaluated_before_input_validation() {
    let w = world();
    let viewer = grant(&w, &w.tenant_a, "vera", Role::Viewer);
    // An invalid slug from an unauthorized caller must still be a 403, not a 422 that would
    // confirm the endpoint exists and validates input.
    let err = w
        .svc
        .create_workspace(&viewer, "NOT A SLUG", &ctx())
        .await
        .expect_err("denied");
    assert!(matches!(err, AppError::Forbidden { .. }));
}

#[tokio::test]
async fn invalid_slug_is_a_validation_error_and_stores_nothing() {
    let w = world();
    let err = w
        .svc
        .create_workspace(&w.admin_a, "Bad Slug", &ctx())
        .await
        .expect_err("invalid");
    assert!(matches!(
        err,
        AppError::Invalid(IdentityError::InvalidSlug(_))
    ));
    assert!(audits(&w).is_empty());
}

#[tokio::test]
async fn duplicate_workspace_slug_conflicts_inside_a_tenant_but_not_across_tenants() {
    let w = world();
    w.svc
        .create_workspace(&w.admin_a, "platform", &ctx())
        .await
        .expect("first");

    let dup = w.svc.create_workspace(&w.admin_a, "platform", &ctx()).await;
    assert_eq!(dup, Err(AppError::Conflict(ConflictKind::WorkspaceSlug)));

    // Same name in another tenant is fine: slugs are unique per tenant.
    w.svc
        .create_workspace(&w.admin_b, "platform", &ctx())
        .await
        .expect("other tenant");
}

#[tokio::test]
async fn a_failed_create_leaves_no_audit_record_behind() {
    let w = world();
    w.svc
        .create_workspace(&w.admin_a, "platform", &ctx())
        .await
        .expect("first");
    let before = audits(&w).len();
    let _ = w.svc.create_workspace(&w.admin_a, "platform", &ctx()).await;
    assert_eq!(
        audits(&w).len(),
        before,
        "audit and state change are atomic"
    );
}

#[tokio::test]
async fn any_role_with_workspace_read_can_get_a_workspace() {
    let w = world();
    let ws = w
        .svc
        .create_workspace(&w.admin_a, "platform", &ctx())
        .await
        .expect("created");
    let viewer = grant(&w, &w.tenant_a, "vera", Role::Viewer);

    let found = w
        .svc
        .get_workspace(&viewer, ws.id(), &ctx())
        .await
        .expect("readable");

    assert_eq!(found, ws);
}

#[tokio::test]
async fn getting_a_missing_workspace_is_not_found_and_not_audited() {
    let w = world();
    let err = w
        .svc
        .get_workspace(&w.admin_a, WorkspaceId::generate(), &ctx())
        .await;
    assert_eq!(err, Err(AppError::NotFound));
    assert!(
        audits(&w).is_empty(),
        "a plain miss is not a security event"
    );
}

#[tokio::test]
async fn getting_another_tenants_workspace_is_not_found_and_audited_without_leaking_it() {
    let w = world();
    let foreign = w
        .svc
        .create_workspace(&w.admin_b, "secret-ws", &ctx())
        .await
        .expect("b's");
    let before = audits(&w).len();

    let err = w.svc.get_workspace(&w.admin_a, foreign.id(), &ctx()).await;

    assert_eq!(
        err,
        Err(AppError::NotFound),
        "indistinguishable from a missing workspace"
    );
    let log = audits(&w);
    assert_eq!(log.len(), before + 1);
    let denial = log.last().expect("denial record");
    assert_eq!(denial.action(), AuditAction::CrossTenantAccessDenied);
    assert_eq!(denial.resource_type(), ResourceType::Workspace);
    assert_eq!(
        denial.tenant_id(),
        w.tenant_a.id(),
        "recorded in the ACTOR's tenant"
    );
    assert_eq!(denial.actor().as_str(), "alice");
    assert_eq!(
        denial.resource_id(),
        foreign.id().to_string(),
        "only the id the caller sent"
    );
    let rendered = format!("{denial:?}");
    assert!(
        !rendered.contains(&w.tenant_b.id().to_string()) && !rendered.contains("secret-ws"),
        "audit entry must not carry data of the other tenant: {rendered}"
    );
}

#[tokio::test]
async fn the_denial_stands_even_when_the_audit_write_fails() {
    let w = world();
    let foreign = w
        .svc
        .create_workspace(&w.admin_b, "secret-ws", &ctx())
        .await
        .expect("b's");
    w.store.with(|s| s.fail_audit_writes = true);

    let err = w.svc.get_workspace(&w.admin_a, foreign.id(), &ctx()).await;

    assert_eq!(
        err,
        Err(AppError::NotFound),
        "fail closed: access is denied regardless"
    );
}

// --------------------------------------------------------------- namespaces

#[tokio::test]
async fn creating_a_namespace_binds_it_to_the_workspace_and_audits_it() {
    let w = world();
    let ws = w
        .svc
        .create_workspace(&w.admin_a, "platform", &ctx())
        .await
        .expect("ws");

    let ns = w
        .svc
        .create_namespace(&w.admin_a, ws.id(), "dev", "dev", &ctx())
        .await
        .expect("ns");

    assert_eq!(ns.workspace_id(), ws.id());
    assert_eq!(ns.tenant_id(), w.tenant_a.id());
    assert_eq!(ns.environment().as_str(), "dev");
    let log = audits(&w);
    assert_eq!(
        log.last().expect("record").action(),
        AuditAction::NamespaceCreated
    );
}

#[tokio::test]
async fn namespace_creation_in_a_foreign_workspace_is_not_found_and_audited() {
    let w = world();
    let foreign = w
        .svc
        .create_workspace(&w.admin_b, "b-ws", &ctx())
        .await
        .expect("b's");

    let err = w
        .svc
        .create_namespace(&w.admin_a, foreign.id(), "dev", "dev", &ctx())
        .await;

    assert_eq!(err, Err(AppError::NotFound));
    let log = audits(&w);
    let denial = log.last().expect("record");
    assert_eq!(denial.action(), AuditAction::CrossTenantAccessDenied);
    assert_eq!(denial.tenant_id(), w.tenant_a.id());
    assert!(
        w.store.with(|s| s.namespaces.is_empty()),
        "nothing was created"
    );
}

#[tokio::test]
async fn developers_cannot_create_namespaces_and_learn_nothing_about_foreign_workspaces() {
    let w = world();
    let own = w
        .svc
        .create_workspace(&w.admin_a, "platform", &ctx())
        .await
        .expect("own");
    let foreign = w
        .svc
        .create_workspace(&w.admin_b, "b-ws", &ctx())
        .await
        .expect("foreign");
    let dev = grant(&w, &w.tenant_a, "dan", Role::Developer);
    let expected = Err(AppError::Forbidden {
        permission: Permission::NamespaceCreate,
    });

    let on_own = w
        .svc
        .create_namespace(&dev, own.id(), "dev", "dev", &ctx())
        .await;
    let on_foreign = w
        .svc
        .create_namespace(&dev, foreign.id(), "dev", "dev", &ctx())
        .await;

    assert_eq!(on_own, expected);
    assert_eq!(
        on_foreign, expected,
        "same answer whether or not the workspace is theirs"
    );
}

#[tokio::test]
async fn invalid_environment_and_duplicate_namespace_slug_are_reported_distinctly() {
    let w = world();
    let ws = w
        .svc
        .create_workspace(&w.admin_a, "platform", &ctx())
        .await
        .expect("ws");

    let bad_env = w
        .svc
        .create_namespace(&w.admin_a, ws.id(), "dev", "qa", &ctx())
        .await;
    assert!(matches!(
        bad_env,
        Err(AppError::Invalid(IdentityError::UnknownEnvironment(_)))
    ));

    w.svc
        .create_namespace(&w.admin_a, ws.id(), "dev", "dev", &ctx())
        .await
        .expect("first");
    let dup = w
        .svc
        .create_namespace(&w.admin_a, ws.id(), "dev", "prod", &ctx())
        .await;
    assert_eq!(dup, Err(AppError::Conflict(ConflictKind::NamespaceSlug)));
}

#[tokio::test]
async fn getting_another_tenants_namespace_is_not_found_and_audited() {
    let w = world();
    let ws_b = w
        .svc
        .create_workspace(&w.admin_b, "b-ws", &ctx())
        .await
        .expect("ws");
    let ns_b = w
        .svc
        .create_namespace(&w.admin_b, ws_b.id(), "b-dev", "dev", &ctx())
        .await
        .expect("ns");

    let err = w.svc.get_namespace(&w.admin_a, ns_b.id(), &ctx()).await;

    assert_eq!(err, Err(AppError::NotFound));
    let log = audits(&w);
    let denial = log.last().expect("record");
    assert_eq!(denial.action(), AuditAction::CrossTenantAccessDenied);
    assert_eq!(denial.resource_type(), ResourceType::Namespace);
}

// --------------------------------------------------------------- pagination

#[tokio::test]
async fn listing_pages_through_all_items_in_order_and_only_for_the_callers_tenant() {
    let w = world();
    let mut created = Vec::new();
    for i in 0..5 {
        let ws = w
            .svc
            .create_workspace(&w.admin_a, &format!("ws-{i}"), &ctx())
            .await
            .expect("created");
        created.push(ws.id());
    }
    w.svc
        .create_workspace(&w.admin_b, "ws-of-b", &ctx())
        .await
        .expect("b's");

    let mut seen = Vec::new();
    let mut cursor = None;
    let mut pages = 0;
    // Bounded on purpose: a pagination bug must fail the test, not hang it.
    for _ in 0..10 {
        let page = w
            .svc
            .list_workspaces(
                &w.admin_a,
                &PageRequest {
                    limit: Some(2),
                    cursor: cursor.clone(),
                },
            )
            .await
            .expect("page");
        pages += 1;
        seen.extend(page.items.iter().map(Workspace::id));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    assert_eq!(pages, 3, "5 items at 2 per page");
    assert_eq!(seen, created, "all items, in id order, none of tenant B");
}

#[tokio::test]
async fn an_exact_multiple_of_the_page_size_has_no_phantom_next_page() {
    let w = world();
    for i in 0..4 {
        w.svc
            .create_workspace(&w.admin_a, &format!("ws-{i}"), &ctx())
            .await
            .expect("created");
    }
    let page = w
        .svc
        .list_workspaces(
            &w.admin_a,
            &PageRequest {
                limit: Some(4),
                cursor: None,
            },
        )
        .await
        .expect("page");
    assert_eq!(page.items.len(), 4);
    assert_eq!(page.next_cursor, None);
}

#[tokio::test]
async fn page_size_is_clamped_and_bad_cursors_are_rejected() {
    assert_eq!(
        PageRequest {
            limit: Some(0),
            cursor: None
        }
        .effective_limit(),
        1
    );
    assert_eq!(
        PageRequest {
            limit: Some(10_000),
            cursor: None
        }
        .effective_limit(),
        MAX_PAGE_SIZE
    );
    assert_eq!(PageRequest::default().effective_limit(), 50);

    let w = world();
    let err = w
        .svc
        .list_workspaces(
            &w.admin_a,
            &PageRequest {
                limit: None,
                cursor: Some("garbage".into()),
            },
        )
        .await;
    assert_eq!(err, Err(AppError::Invalid(IdentityError::MalformedId)));
}

#[tokio::test]
async fn listing_namespaces_is_scoped_to_the_callers_tenant() {
    let w = world();
    let ws_a = w
        .svc
        .create_workspace(&w.admin_a, "a-ws", &ctx())
        .await
        .expect("ws");
    let ws_b = w
        .svc
        .create_workspace(&w.admin_b, "b-ws", &ctx())
        .await
        .expect("ws");
    w.svc
        .create_namespace(&w.admin_a, ws_a.id(), "a-dev", "dev", &ctx())
        .await
        .expect("ns");
    w.svc
        .create_namespace(&w.admin_b, ws_b.id(), "b-dev", "dev", &ctx())
        .await
        .expect("ns");

    let page = w
        .svc
        .list_namespaces(&w.admin_a, &PageRequest::default())
        .await
        .expect("page");

    let slugs: Vec<_> = page
        .items
        .iter()
        .map(|n| n.slug().as_str().to_owned())
        .collect();
    assert_eq!(slugs, ["a-dev"]);
}

#[tokio::test]
async fn ping_reflects_storage_health() {
    let w = world();
    assert_eq!(w.svc.ping().await, Ok(()));
    w.store.with(|s| {
        s.fail_everything = Some(StoreError::Unavailable {
            detail: "down".into(),
        })
    });
    assert_eq!(w.svc.ping().await, Err(AppError::Unavailable));
}
