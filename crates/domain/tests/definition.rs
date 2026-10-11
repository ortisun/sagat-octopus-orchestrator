//! Invariants of the workflow definition lifecycle (T002-004; FR-001-002, NFR-001-SEC).
#![allow(clippy::expect_used)]

use sagat_domain::audit::{AuditAction, ResourceType};
use sagat_domain::definition::{Workflow, WorkflowId, WorkflowVersion, WorkflowVersionId};
use sagat_domain::identity::{
    Environment, IdentityError, Namespace, NamespaceId, Permission, Principal, Role, Slug, Subject,
    TenantId, WorkspaceId,
};
use sagat_domain::workflow::Digest;
use time::OffsetDateTime;

const NOW: OffsetDateTime = OffsetDateTime::UNIX_EPOCH;

fn principal(tenant: TenantId, role: Role) -> Principal {
    Principal::new(tenant, Subject::parse("alice").expect("subject"), role)
}

fn namespace(tenant: TenantId) -> Namespace {
    Namespace::restore(
        NamespaceId::generate(),
        tenant,
        WorkspaceId::generate(),
        Slug::parse("payments").expect("slug"),
        Environment::Dev,
        NOW,
    )
}

fn name(s: &str) -> Slug {
    Slug::parse(s).expect("slug")
}

#[test]
fn a_draft_starts_at_revision_one_in_the_principals_tenant_and_namespace() {
    let tenant = TenantId::generate();
    let ns = namespace(tenant);
    let w = Workflow::create(
        &principal(tenant, Role::Developer),
        &ns,
        WorkflowId::generate(),
        name("invoice-flow"),
        NOW,
    )
    .expect("created");
    assert_eq!(w.tenant_id(), tenant);
    assert_eq!(w.namespace_id(), ns.id());
    assert_eq!(w.draft_revision(), 1);
    assert_eq!(w.active_version(), None);
    assert_eq!(w.name().as_str(), "invoice-flow");
    assert_eq!((w.created_at(), w.updated_at()), (NOW, NOW));
}

#[test]
fn only_roles_that_write_definitions_can_create_one() {
    let tenant = TenantId::generate();
    let ns = namespace(tenant);
    for role in [Role::Admin, Role::Developer] {
        assert!(
            Workflow::create(
                &principal(tenant, role),
                &ns,
                WorkflowId::generate(),
                name("flow-a"),
                NOW
            )
            .is_ok(),
            "{role}"
        );
    }
    for role in [Role::Operator, Role::Viewer, Role::Worker] {
        assert_eq!(
            Workflow::create(
                &principal(tenant, role),
                &ns,
                WorkflowId::generate(),
                name("flow-a"),
                NOW
            ),
            Err(IdentityError::PermissionDenied {
                role,
                permission: Permission::DefinitionWrite
            }),
            "{role}"
        );
    }
}

#[test]
fn a_namespace_of_another_tenant_is_refused_as_a_tenant_mismatch() {
    let mine = TenantId::generate();
    let theirs = namespace(TenantId::generate());
    assert_eq!(
        Workflow::create(
            &principal(mine, Role::Admin),
            &theirs,
            WorkflowId::generate(),
            name("flow-a"),
            NOW
        ),
        Err(IdentityError::TenantMismatch)
    );
}

#[test]
fn authorization_is_checked_before_the_tenant() {
    // A viewer probing another tenant's namespace must learn nothing about it: the denial comes
    // first, exactly as for namespaces.
    let mine = TenantId::generate();
    let theirs = namespace(TenantId::generate());
    assert!(matches!(
        Workflow::create(
            &principal(mine, Role::Viewer),
            &theirs,
            WorkflowId::generate(),
            name("flow-a"),
            NOW
        ),
        Err(IdentityError::PermissionDenied { .. })
    ));
}

fn workflow(tenant: TenantId) -> Workflow {
    Workflow::create(
        &principal(tenant, Role::Admin),
        &namespace(tenant),
        WorkflowId::generate(),
        name("flow-a"),
        NOW,
    )
    .expect("created")
}

#[test]
fn a_version_records_who_published_what() {
    let tenant = TenantId::generate();
    let w = workflow(tenant);
    let digest = Digest::new([7; 32]);
    let id = WorkflowVersionId::generate();
    let v = WorkflowVersion::publish(&principal(tenant, Role::Developer), &w, id, 3, digest, NOW)
        .expect("published");
    assert_eq!(
        (v.id(), v.workflow_id(), v.tenant_id(), v.number()),
        (id, w.id(), tenant, 3)
    );
    assert_eq!(v.digest(), &digest);
    assert_eq!(v.published_by().as_str(), "alice");
    assert_eq!(v.published_at(), NOW);
}

#[test]
fn only_roles_that_publish_can_publish_and_never_across_tenants() {
    let tenant = TenantId::generate();
    let w = workflow(tenant);
    let digest = Digest::new([1; 32]);
    for role in [Role::Operator, Role::Viewer, Role::Worker] {
        assert_eq!(
            WorkflowVersion::publish(
                &principal(tenant, role),
                &w,
                WorkflowVersionId::generate(),
                1,
                digest,
                NOW
            ),
            Err(IdentityError::PermissionDenied {
                role,
                permission: Permission::DefinitionPublish
            }),
            "{role}"
        );
    }
    assert_eq!(
        WorkflowVersion::publish(
            &principal(TenantId::generate(), Role::Admin),
            &w,
            WorkflowVersionId::generate(),
            1,
            digest,
            NOW
        ),
        Err(IdentityError::TenantMismatch)
    );
}

#[test]
fn restoring_from_storage_keeps_every_field() {
    let tenant = TenantId::generate();
    let w = workflow(tenant);
    let version = WorkflowVersionId::generate();
    let r = Workflow::restore(
        w.id(),
        tenant,
        w.namespace_id(),
        name("flow-a"),
        5,
        Some(version),
        NOW,
        NOW,
    );
    assert_eq!((r.draft_revision(), r.active_version()), (5, Some(version)));
}

#[test]
fn the_new_audit_actions_have_stable_names() {
    for (action, text) in [
        (AuditAction::WorkflowCreated, "workflow.created"),
        (AuditAction::WorkflowDraftUpdated, "workflow.draft_updated"),
        (AuditAction::WorkflowPublished, "workflow.published"),
        (AuditAction::RunStarted, "run.started"),
        (AuditAction::RunCancelRequested, "run.cancel_requested"),
        (AuditAction::RunPaused, "run.paused"),
        (AuditAction::RunResumed, "run.resumed"),
        (AuditAction::RunSignalled, "run.signalled"),
    ] {
        assert_eq!(action.as_str(), text);
        assert_eq!(action.to_string(), text);
    }
    for (resource, text) in [
        (ResourceType::Workflow, "workflow"),
        (ResourceType::WorkflowVersion, "workflow_version"),
        (ResourceType::Run, "run"),
    ] {
        assert_eq!(resource.as_str(), text);
    }
}
