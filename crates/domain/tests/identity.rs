//! Invariant tests for the identity model (T001-004, T001-009; FR-001-001, NFR-001-SEC).
// Test helpers (not `#[test]` fns) build fixtures with `expect`; a panic there is a test failure.
#![allow(clippy::expect_used)]

use sagat_domain::identity::{
    Environment, IdentityError, Membership, Namespace, NamespaceId, Permission, Principal, Role,
    Slug, SlugViolation, Subject, SubjectViolation, Tenant, TenantId, Workspace, WorkspaceId,
};
use time::OffsetDateTime;

const NOW: OffsetDateTime = OffsetDateTime::UNIX_EPOCH;

fn slug(s: &str) -> Slug {
    Slug::parse(s).expect("valid slug")
}

fn subject(s: &str) -> Subject {
    Subject::parse(s).expect("valid subject")
}

fn principal(tenant: TenantId, role: Role) -> Principal {
    Principal::new(tenant, subject("user-1"), role)
}

// ---------------------------------------------------------------- slugs

#[test]
fn slug_accepts_dns_label_like_names() {
    for ok in ["dev", "a1b", "payments-prod", "team-42-eu", &"a".repeat(63)] {
        assert!(Slug::parse(ok).is_ok(), "{ok} should be valid");
    }
}

#[test]
fn slug_reports_the_violated_rule() {
    let too_long = "a".repeat(64);
    let cases: [(&str, SlugViolation); 10] = [
        ("", SlugViolation::Length),
        ("ab", SlugViolation::Length),
        (too_long.as_str(), SlugViolation::Length),
        ("Dev", SlugViolation::Charset),
        ("dev_x", SlugViolation::Charset),
        ("dev x", SlugViolation::Charset),
        ("café", SlugViolation::Charset),
        ("1abc", SlugViolation::Start),
        ("abc-", SlugViolation::End),
        ("a--b", SlugViolation::ConsecutiveHyphens),
    ];
    for (input, expected) in cases {
        assert_eq!(
            Slug::parse(input),
            Err(IdentityError::InvalidSlug(expected)),
            "input {input:?}"
        );
    }
    assert_eq!(
        Slug::parse("-abc"),
        Err(IdentityError::InvalidSlug(SlugViolation::Start))
    );
}

#[test]
fn slug_rejects_path_and_injection_shapes() {
    for bad in ["../etc", "a/b", "a.b", "a;drop", "a'b", "<script>"] {
        assert!(Slug::parse(bad).is_err(), "{bad} must be rejected");
    }
}

// ------------------------------------------------------------- subjects

#[test]
fn subject_accepts_typical_oidc_subjects() {
    for ok in [
        "user-123",
        "auth0|5f7c8ec7c33c6c004bbafe82",
        "svc:sdk-client@tenant",
        "ユーザー",
    ] {
        assert!(Subject::parse(ok).is_ok(), "{ok} should be valid");
    }
}

#[test]
fn subject_rejects_empty_oversized_and_control_characters() {
    assert_eq!(
        Subject::parse(""),
        Err(IdentityError::InvalidSubject(SubjectViolation::Empty))
    );
    assert_eq!(
        Subject::parse("   "),
        Err(IdentityError::InvalidSubject(SubjectViolation::Empty))
    );
    assert_eq!(
        Subject::parse(&"x".repeat(256)),
        Err(IdentityError::InvalidSubject(SubjectViolation::TooLong))
    );
    assert!(Subject::parse(&"x".repeat(255)).is_ok());
    for bad in ["a\nb", "a\u{0}b", "a\tb", "a\u{7f}b"] {
        assert_eq!(
            Subject::parse(bad),
            Err(IdentityError::InvalidSubject(
                SubjectViolation::ControlCharacter
            )),
            "{bad:?}"
        );
    }
}

#[test]
fn the_system_namespace_is_reserved_for_platform_actors() {
    // `system:provisioning` is the actor written into the audit trail for operator actions. A
    // subject that could be parsed from a token or an operator command would let someone
    // authenticate as (or impersonate in the audit trail) the platform itself.
    for reserved in [
        "system:provisioning",
        "system:anything-else",
        "system:",
        "SYSTEM:provisioning",
        "System:Provisioning",
    ] {
        assert_eq!(
            Subject::parse(reserved),
            Err(IdentityError::InvalidSubject(SubjectViolation::Reserved)),
            "{reserved:?}"
        );
    }
    // Only the namespace is reserved, not words that merely start the same way.
    for ok in [
        "system",
        "systemd",
        "system-user",
        "svc:system:x",
        "my-system:x",
        " system:x",
    ] {
        assert!(Subject::parse(ok).is_ok(), "{ok:?} is an ordinary subject");
    }
}

#[test]
fn the_platform_actor_exists_but_cannot_be_parsed_back_into_a_subject() {
    let actor = Subject::provisioning();

    assert_eq!(actor.as_str(), "system:provisioning");
    assert_eq!(
        Subject::parse(actor.as_str()),
        Err(IdentityError::InvalidSubject(SubjectViolation::Reserved))
    );
}

#[test]
fn a_reserved_subject_violation_explains_itself() {
    let message = IdentityError::InvalidSubject(SubjectViolation::Reserved).to_string();

    assert!(
        message.contains("system:") && message.contains("reserved"),
        "{message}"
    );
}

// ------------------------------------------------------------------ RBAC

const ALL_ROLES: [Role; 6] = [
    Role::Admin,
    Role::Developer,
    Role::Operator,
    Role::Viewer,
    Role::Worker,
    Role::Runner,
];

/// The authoritative matrix, written out by hand (not derived from `Role::permissions`).
fn expected(role: Role) -> Vec<Permission> {
    use Permission::{
        DatabaseAdHoc, DatabaseUse, DefinitionPublish, DefinitionRead, DefinitionWrite,
        EventManage, EventPublish, EventRead, NamespaceRead, OpsRead, PublicationApprove,
        QuotaRead, RunOperate, RunPayloadInspect, RunRead, RunSignal, RunStart, ScriptBuild,
        ScriptExecute, ScriptPublish, ScriptRead, ScriptWrite, SecretRead, SecretUse,
        VersionActivate, WorkerExecute, WorkerManage, WorkspaceRead,
    };
    match role {
        Role::Admin => Permission::ALL.to_vec(),
        Role::Developer => {
            vec![
                DatabaseUse,
                DatabaseAdHoc,
                WorkspaceRead,
                NamespaceRead,
                DefinitionRead,
                DefinitionWrite,
                DefinitionPublish,
                RunStart,
                RunRead,
                SecretRead,
                SecretUse,
                QuotaRead,
                EventPublish,
                EventRead,
                EventManage,
                ScriptRead,
                ScriptWrite,
                ScriptPublish,
                OpsRead,
                PublicationApprove,
            ]
        }
        Role::Operator => vec![
            WorkspaceRead,
            NamespaceRead,
            DefinitionRead,
            RunStart,
            RunRead,
            RunPayloadInspect,
            RunOperate,
            RunSignal,
            QuotaRead,
            EventPublish,
            EventRead,
            ScriptRead,
            OpsRead,
            WorkerManage,
            PublicationApprove,
            VersionActivate,
        ],
        Role::Viewer => vec![
            WorkspaceRead,
            NamespaceRead,
            DefinitionRead,
            RunRead,
            QuotaRead,
            EventRead,
            ScriptRead,
            OpsRead,
        ],
        Role::Worker => vec![WorkerExecute],
        Role::Runner => vec![WorkerExecute, ScriptBuild, ScriptExecute],
    }
}

#[test]
fn role_permission_matrix_is_exactly_as_specified() {
    for role in ALL_ROLES {
        let allowed = expected(role);
        for permission in Permission::ALL {
            assert_eq!(
                role.grants(permission),
                allowed.contains(&permission),
                "role {role} / permission {permission}"
            );
        }
    }
}

#[test]
fn only_admin_can_create_scopes_manage_members_or_read_audit() {
    for restricted in [
        Permission::WorkspaceCreate,
        Permission::NamespaceCreate,
        Permission::MembershipManage,
        Permission::AuditRead,
        // Who may change what a tenant's secrets, quotas, egress and workers are allowed to do.
        Permission::SecretManage,
        Permission::QuotaManage,
        Permission::EgressManage,
        Permission::WorkerBind,
        // Who sets how many approvals a namespace asks for.
        Permission::PublicationManage,
    ] {
        assert!(Role::Admin.grants(restricted));
        for role in [
            Role::Developer,
            Role::Operator,
            Role::Viewer,
            Role::Worker,
            Role::Runner,
        ] {
            assert!(
                !role.grants(restricted),
                "{role} must not hold {restricted}"
            );
        }
    }
}

#[test]
fn a_worker_can_execute_tasks_and_do_nothing_else() {
    assert_eq!(Role::Worker.permissions(), &[Permission::WorkerExecute]);
    for permission in Permission::ALL {
        assert_eq!(
            Role::Worker.grants(permission),
            permission == Permission::WorkerExecute,
            "{permission}"
        );
    }
    for role in [Role::Developer, Role::Operator, Role::Viewer] {
        assert!(
            !role.grants(Permission::WorkerExecute),
            "{role} is a human role"
        );
    }
}

#[test]
fn only_operators_and_admins_can_operate_signal_or_inspect_run_payloads() {
    for permission in [
        Permission::RunOperate,
        Permission::RunSignal,
        Permission::RunPayloadInspect,
    ] {
        for role in ALL_ROLES {
            assert_eq!(
                role.grants(permission),
                matches!(role, Role::Admin | Role::Operator),
                "{role} / {permission}"
            );
        }
    }
}

#[test]
fn authorize_reports_role_and_permission_on_denial() {
    let p = principal(TenantId::generate(), Role::Viewer);
    assert_eq!(p.authorize(Permission::WorkspaceRead), Ok(()));
    assert_eq!(
        p.authorize(Permission::WorkspaceCreate),
        Err(IdentityError::PermissionDenied {
            role: Role::Viewer,
            permission: Permission::WorkspaceCreate
        })
    );
}

#[test]
fn role_and_environment_round_trip_and_reject_unknown_values() {
    for role in ALL_ROLES {
        assert_eq!(role.as_str().parse::<Role>(), Ok(role));
    }
    for env in [Environment::Dev, Environment::Staging, Environment::Prod] {
        assert_eq!(env.as_str().parse::<Environment>(), Ok(env));
    }
    assert_eq!(
        "root".parse::<Role>(),
        Err(IdentityError::UnknownRole("root".into()))
    );
    assert_eq!(
        "ADMIN".parse::<Role>(),
        Err(IdentityError::UnknownRole("ADMIN".into()))
    );
    assert_eq!(
        "qa".parse::<Environment>(),
        Err(IdentityError::UnknownEnvironment("qa".into()))
    );
}

// ------------------------------------------------------------ workspaces

#[test]
fn admin_creates_workspace_in_their_own_tenant() {
    let tenant = TenantId::generate();
    let admin = principal(tenant, Role::Admin);
    let id = WorkspaceId::generate();

    let ws = Workspace::create(&admin, id, slug("platform"), NOW).expect("admin may create");

    assert_eq!(ws.id(), id);
    assert_eq!(
        ws.tenant_id(),
        tenant,
        "tenant must come from the principal"
    );
    assert_eq!(ws.slug().as_str(), "platform");
    assert_eq!(ws.created_at(), NOW);
}

#[test]
fn non_admin_roles_cannot_create_workspaces() {
    for role in [Role::Developer, Role::Operator, Role::Viewer, Role::Worker] {
        let p = principal(TenantId::generate(), role);
        let result = Workspace::create(&p, WorkspaceId::generate(), slug("platform"), NOW);
        assert_eq!(
            result,
            Err(IdentityError::PermissionDenied {
                role,
                permission: Permission::WorkspaceCreate
            })
        );
    }
}

// ------------------------------------------------------------ namespaces

fn workspace_of(tenant: TenantId) -> Workspace {
    Workspace::restore(WorkspaceId::generate(), tenant, slug("platform"), NOW)
}

#[test]
fn admin_creates_namespace_inheriting_tenant_and_workspace() {
    let tenant = TenantId::generate();
    let admin = principal(tenant, Role::Admin);
    let ws = workspace_of(tenant);
    let id = NamespaceId::generate();

    let ns = Namespace::create(&admin, &ws, id, slug("dev"), Environment::Dev, NOW)
        .expect("admin may create");

    assert_eq!(ns.id(), id);
    assert_eq!(ns.tenant_id(), tenant);
    assert_eq!(ns.workspace_id(), ws.id());
    assert_eq!(ns.slug().as_str(), "dev");
    assert_eq!(ns.environment(), Environment::Dev);
}

#[test]
fn namespace_in_a_foreign_workspace_is_rejected_as_tenant_mismatch() {
    let mine = TenantId::generate();
    let theirs = TenantId::generate();
    let admin = principal(mine, Role::Admin);
    let foreign_workspace = workspace_of(theirs);

    let result = Namespace::create(
        &admin,
        &foreign_workspace,
        NamespaceId::generate(),
        slug("dev"),
        Environment::Dev,
        NOW,
    );

    assert_eq!(result, Err(IdentityError::TenantMismatch));
}

#[test]
fn permission_is_checked_before_tenant_so_denied_roles_learn_nothing() {
    // A developer probing a workspace of another tenant must get the same answer as probing
    // their own: "permission denied", never "tenant mismatch".
    let mine = TenantId::generate();
    let theirs = TenantId::generate();
    let developer = principal(mine, Role::Developer);

    for ws in [workspace_of(mine), workspace_of(theirs)] {
        let result = Namespace::create(
            &developer,
            &ws,
            NamespaceId::generate(),
            slug("dev"),
            Environment::Dev,
            NOW,
        );
        assert_eq!(
            result,
            Err(IdentityError::PermissionDenied {
                role: Role::Developer,
                permission: Permission::NamespaceCreate
            })
        );
    }
}

// ----------------------------------------------------------- memberships

#[test]
fn admin_grants_a_role_inside_their_own_tenant() {
    let tenant = TenantId::generate();
    let admin = principal(tenant, Role::Admin);

    let m = Membership::grant(&admin, subject("alice"), Role::Developer, NOW).expect("admin");

    assert_eq!(m.tenant_id(), tenant);
    assert_eq!(m.subject().as_str(), "alice");
    assert_eq!(m.role(), Role::Developer);
    assert_eq!(
        m.principal(),
        Principal::new(tenant, subject("alice"), Role::Developer)
    );
}

#[test]
fn only_admin_can_grant_memberships() {
    for role in [Role::Developer, Role::Operator, Role::Viewer, Role::Worker] {
        let p = principal(TenantId::generate(), role);
        assert_eq!(
            Membership::grant(&p, subject("mallory"), Role::Admin, NOW),
            Err(IdentityError::PermissionDenied {
                role,
                permission: Permission::MembershipManage
            }),
            "{role} must not be able to escalate privileges"
        );
    }
}

#[test]
fn bootstrap_admin_binds_the_first_admin_to_the_provisioned_tenant() {
    let tenant = Tenant::provision(TenantId::generate(), slug("acme"), NOW);
    let m = Membership::bootstrap_admin(&tenant, subject("root-user"), NOW);
    assert_eq!(m.tenant_id(), tenant.id());
    assert_eq!(m.role(), Role::Admin);
}

// ------------------------------------------------------------------ ids

#[test]
fn ids_round_trip_through_text_and_reject_garbage() {
    let id = TenantId::generate();
    assert_eq!(id.to_string().parse::<TenantId>(), Ok(id));
    for bad in [
        "",
        "not-a-uuid",
        "123",
        "00000000-0000-0000-0000-00000000000g",
    ] {
        assert_eq!(
            bad.parse::<TenantId>(),
            Err(IdentityError::MalformedId),
            "{bad:?}"
        );
    }
}

#[test]
fn generated_ids_are_unique_uuid_v7() {
    let a = WorkspaceId::generate();
    let b = WorkspaceId::generate();
    assert_ne!(a, b);
    assert_eq!(a.as_uuid().get_version_num(), 7);
}

#[test]
fn error_messages_never_disclose_identifiers() {
    let mine = TenantId::generate();
    let theirs = TenantId::generate();
    let admin = principal(mine, Role::Admin);
    let err = Namespace::create(
        &admin,
        &workspace_of(theirs),
        NamespaceId::generate(),
        slug("dev"),
        Environment::Dev,
        NOW,
    )
    .expect_err("foreign workspace");

    let message = err.to_string();
    assert!(
        !message.contains(&mine.to_string()),
        "leaks the caller tenant: {message}"
    );
    assert!(
        !message.contains(&theirs.to_string()),
        "leaks the other tenant: {message}"
    );
}

#[test]
fn rejected_values_are_clipped_in_error_messages() {
    let huge = "x".repeat(100_000);
    let role = huge.parse::<Role>().expect_err("unknown role").to_string();
    let environment = huge
        .parse::<Environment>()
        .expect_err("unknown environment")
        .to_string();
    for message in [role, environment] {
        assert!(
            message.len() < 100,
            "an error must not reflect a 100 kB payload: {} bytes",
            message.len()
        );
        assert!(message.contains('…'), "{message}");
    }
    // Short values are shown whole.
    assert_eq!(
        "root".parse::<Role>(),
        Err(IdentityError::UnknownRole("root".into()))
    );
}

#[test]
fn only_the_runner_reads_the_source_of_every_script_or_fetches_artifacts() {
    // The source of the tenant's scripts and the artifacts are for the sandbox's runner. A person
    // who may write scripts, and a worker that serves activities, hold neither permission.
    for permission in [Permission::ScriptBuild, Permission::ScriptExecute] {
        assert!(Role::Admin.grants(permission) && Role::Runner.grants(permission));
        for role in [Role::Developer, Role::Operator, Role::Viewer, Role::Worker] {
            assert!(
                !role.grants(permission),
                "{role} must not hold {permission}"
            );
        }
    }
    // The runner serves tasks, builds and runs; it reads, writes and publishes nothing by itself.
    for permission in [
        Permission::ScriptRead,
        Permission::ScriptWrite,
        Permission::ScriptPublish,
        Permission::SecretRead,
        Permission::SecretUse,
        Permission::DefinitionRead,
        Permission::RunStart,
    ] {
        assert!(
            !Role::Runner.grants(permission),
            "runner must not hold {permission}"
        );
    }
    assert_eq!("runner".parse::<Role>(), Ok(Role::Runner));
}
