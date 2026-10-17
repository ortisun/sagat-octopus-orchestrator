//! Identity and tenancy: tenants, workspaces, namespaces, memberships and RBAC.
//!
//! Tenant isolation is enforced by construction: every tenant-scoped entity is created from an
//! authenticated [`Principal`] and inherits *its* tenant. No constructor accepts a tenant id
//! supplied by a client (FR-001-001, NFR-001-SEC, `docs/architecture/security-and-sandbox.md`).
//!
//! Uniqueness rules (tenant slug globally; workspace and namespace slugs per tenant) are
//! persistence invariants and are enforced and tested in the PostgreSQL adapter.

use std::fmt;
use std::str::FromStr;

use time::OffsetDateTime;

/// At most this many characters of a rejected value are echoed in an error message.
const ECHO_LIMIT: usize = 32;

/// Truncates a rejected, caller-supplied value so an error message cannot be inflated or used to
/// reflect a large payload back at the caller.
pub(crate) fn clip(value: &str) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(ECHO_LIMIT).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// Typed errors of the identity model. Messages never contain identifiers of other tenants.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    /// A slug does not satisfy the naming rules.
    #[error("invalid slug: {0}")]
    InvalidSlug(SlugViolation),
    /// A subject (OIDC `sub`) is empty, too long or contains control characters.
    #[error("invalid subject: {0}")]
    InvalidSubject(SubjectViolation),
    /// The role does not grant the requested permission.
    #[error("role '{role}' does not grant permission '{permission}'")]
    PermissionDenied { role: Role, permission: Permission },
    /// A referenced entity belongs to a different tenant than the acting principal.
    /// Callers must surface this as "not found" so tenant existence is not disclosed.
    #[error("referenced entity is outside the principal's tenant")]
    TenantMismatch,
    /// Unknown role name.
    #[error("unknown role '{0}'")]
    UnknownRole(String),
    /// Unknown environment name.
    #[error("unknown environment '{0}'")]
    UnknownEnvironment(String),
    /// Malformed identifier.
    #[error("malformed identifier")]
    MalformedId,
}

/// Why a slug was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SlugViolation {
    #[error(
        "length must be between {} and {} characters",
        Slug::MIN_LEN,
        Slug::MAX_LEN
    )]
    Length,
    #[error("only lowercase letters, digits and '-' are allowed")]
    Charset,
    #[error("must start with a lowercase letter")]
    Start,
    #[error("must end with a lowercase letter or digit")]
    End,
    #[error("consecutive '-' are not allowed")]
    ConsecutiveHyphens,
}

/// Why a subject was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SubjectViolation {
    #[error("must not be empty")]
    Empty,
    #[error("must be at most {} bytes", Subject::MAX_LEN)]
    TooLong,
    #[error("must not contain control characters")]
    ControlCharacter,
    /// The `system:` namespace belongs to platform actors such as `system:provisioning`.
    #[error("the 'system:' prefix is reserved for platform actors")]
    Reserved,
}

uuid_id!(
    /// Identifier of a [`Tenant`].
    TenantId
);
uuid_id!(
    /// Identifier of a [`Workspace`].
    WorkspaceId
);
uuid_id!(
    /// Identifier of a [`Namespace`].
    NamespaceId
);

/// A DNS-label-like name: 3–63 chars, `[a-z0-9-]`, starts with a letter, ends with a letter or
/// digit, no consecutive hyphens. Safe to embed in hostnames, paths and subjects.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Slug(String);

impl Slug {
    pub const MIN_LEN: usize = 3;
    pub const MAX_LEN: usize = 63;

    /// Validates and wraps a slug.
    ///
    /// # Errors
    /// Returns [`IdentityError::InvalidSlug`] with the first violated rule.
    pub fn parse(value: &str) -> Result<Self, IdentityError> {
        let bytes = value.as_bytes();
        let violation = if !(Self::MIN_LEN..=Self::MAX_LEN).contains(&bytes.len()) {
            Some(SlugViolation::Length)
        } else if !bytes
            .iter()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        {
            Some(SlugViolation::Charset)
        } else if !bytes[0].is_ascii_lowercase() {
            Some(SlugViolation::Start)
        } else if bytes[bytes.len() - 1] == b'-' {
            Some(SlugViolation::End)
        } else if value.contains("--") {
            Some(SlugViolation::ConsecutiveHyphens)
        } else {
            None
        };
        violation.map_or_else(
            || Ok(Self(value.to_owned())),
            |v| Err(IdentityError::InvalidSlug(v)),
        )
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identity of an authenticated caller: the OIDC `sub` claim (user or service account).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Subject(String);

impl Subject {
    pub const MAX_LEN: usize = 255;

    /// The namespace reserved for platform actors (compared case-insensitively).
    pub const RESERVED_PREFIX: &'static str = "system:";

    /// The actor recorded for platform-level operations that have no tenant principal, such as
    /// provisioning a tenant. Not an identity anyone can authenticate as: it lives in the reserved
    /// `system:` namespace, so [`Subject::parse`] (the only way a token subject or an operator
    /// supplied administrator becomes a `Subject`) refuses it. This constructor deliberately does
    /// not go through `parse`.
    #[must_use]
    pub fn provisioning() -> Self {
        Self("system:provisioning".to_owned())
    }

    /// The actor recorded for what a platform operator does from the command line, outside any
    /// tenant (granting a quota ceiling). Reserved like the other platform actors.
    #[must_use]
    pub fn operator() -> Self {
        Self("system:operator".to_owned())
    }

    /// The actor recorded for what the execution engine does by itself (starting a child run).
    /// Like [`Subject::provisioning`] it lives in the reserved `system:` namespace.
    #[must_use]
    pub fn engine() -> Self {
        Self("system:engine".to_owned())
    }

    /// The actor recorded for what a caller outside the platform did without being a principal: a
    /// request to a webhook endpoint that was refused. Reserved like the other platform actors.
    #[must_use]
    pub fn gateway() -> Self {
        Self("system:gateway".to_owned())
    }

    /// Validates and wraps a subject.
    ///
    /// # Errors
    /// Returns [`IdentityError::InvalidSubject`] when empty, too long, containing control chars,
    /// or inside the reserved `system:` namespace.
    pub fn parse(value: &str) -> Result<Self, IdentityError> {
        let violation = if value.trim().is_empty() {
            Some(SubjectViolation::Empty)
        } else if value.len() > Self::MAX_LEN {
            Some(SubjectViolation::TooLong)
        } else if value.chars().any(char::is_control) {
            Some(SubjectViolation::ControlCharacter)
        } else if value
            .get(..Self::RESERVED_PREFIX.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(Self::RESERVED_PREFIX))
        {
            Some(SubjectViolation::Reserved)
        } else {
            None
        };
        violation.map_or_else(
            || Ok(Self(value.to_owned())),
            |v| Err(IdentityError::InvalidSubject(v)),
        )
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Subject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Deployment stage of a namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Environment {
    Dev,
    Staging,
    Prod,
}

impl Environment {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Staging => "staging",
            Self::Prod => "prod",
        }
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Environment {
    type Err = IdentityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "dev" => Ok(Self::Dev),
            "staging" => Ok(Self::Staging),
            "prod" => Ok(Self::Prod),
            other => Err(IdentityError::UnknownEnvironment(clip(other))),
        }
    }
}

/// An action that can be authorized. Granted to roles by [`Role::grants`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    WorkspaceRead,
    WorkspaceCreate,
    NamespaceRead,
    NamespaceCreate,
    MembershipManage,
    DefinitionRead,
    DefinitionWrite,
    DefinitionPublish,
    AuditRead,
    /// Start a run of a published workflow version.
    RunStart,
    /// Read runs, their journal and runtime graph.
    RunRead,
    /// Read a declared, explicitly disclosed business query result.
    RunQuery,
    /// Submit a declared atomic business-state update.
    RunUpdate,
    /// Read a pending human approval and its redacted decision metadata.
    ApprovalRead,
    /// Decide a pending approval after current eligibility and initiator checks.
    ApprovalDecide,
    /// Inspect only the redacted structure of a run input or retained journal payload.
    RunPayloadInspect,
    /// Read payload values classified as `internal`.
    PayloadReadInternal,
    /// Reveal payload values classified as `pii`; granted per subject and namespace, never by role.
    PayloadRevealPii,
    /// Cancel, pause or resume a run.
    RunOperate,
    /// Deliver a signal to a run.
    RunSignal,
    /// Act as a worker: register, claim tasks, heartbeat, complete and fail steps.
    WorkerExecute,
    /// See the quotas of a namespace and how much of them is used.
    QuotaRead,
    /// Change the quotas of a namespace.
    QuotaManage,
    /// List the secrets of a namespace (names and metadata, never values).
    SecretRead,
    /// Create, rotate and delete secrets and decide who may use them.
    SecretManage,
    /// Reference secrets from a workflow, when the secret's own access list allows it.
    SecretUse,
    /// Change the egress policy of a namespace.
    EgressManage,
    /// Decide which namespaces and activities a worker identity may serve.
    WorkerBind,
    /// Settle a step that waits for a decision in a way that skips or replaces its work: give up on
    /// a compensation, or say that a step with an unknown outcome did or did not happen. Retrying
    /// is `run:operate`; this one is explicit permission with a justification (`FR-003-005`).
    SagaOverride,
    /// Publish an event to the inbox through the API.
    EventPublish,
    /// See the endpoints and the inbox of a namespace, and what each event woke or started.
    EventRead,
    /// Create, change and delete webhook endpoints.
    EventManage,
    /// See the scripts of a namespace, their versions, builds, test runs and executions.
    ScriptRead,
    /// Write the draft of a script and try it in the sandbox. Not publishing.
    ScriptWrite,
    /// Publish a version of a script: the code that is built and that workflows can pin.
    ScriptPublish,
    /// See the operational alerts of a namespace, the workers serving it and how far its telemetry
    /// is behind (`FR-008-005`). Everything it shows is a count, an age or a name.
    OpsRead,
    /// Stop a worker taking new work (drain it) and let it take work again.
    WorkerManage,
    /// Take the builds and test runs of the tenant: read the source of a script, make its
    /// artifact. Held by the runner of the sandbox and by nobody else.
    ScriptBuild,
    /// Fetch a built artifact and ask for what a running script is allowed to ask for (a request
    /// to the outside, an execution record). Held by the runner of the sandbox and by nobody else.
    ScriptExecute,
    /// Approve or reject a request to publish a workflow into a namespace that asks for approvals
    /// (`FR-011-003`). The author of a request can never decide it, whatever they hold.
    PublicationApprove,
    /// Set how many approvals a namespace asks for before something is published into it.
    PublicationManage,
    /// Point new runs of a workflow at another version that was published already: a rollback, or
    /// the way forward again (`FR-011-002`). Not publishing: nothing new is made.
    VersionActivate,
    DatabaseManage,
    DatabaseUse,
    DatabaseAdHoc,
    DatabaseWrite,
    DatabaseResultRead,
    /// Resolve only a leased managed database capability; additionally requires the machine role.
    DatabaseExecute,
}

impl Permission {
    /// Every permission, used to grant the admin role and to test the full matrix.
    pub const ALL: [Self; 48] = [
        Self::WorkspaceRead,
        Self::WorkspaceCreate,
        Self::NamespaceRead,
        Self::NamespaceCreate,
        Self::MembershipManage,
        Self::DefinitionRead,
        Self::DefinitionWrite,
        Self::DefinitionPublish,
        Self::AuditRead,
        Self::RunStart,
        Self::RunRead,
        Self::RunQuery,
        Self::RunUpdate,
        Self::ApprovalRead,
        Self::ApprovalDecide,
        Self::RunPayloadInspect,
        Self::PayloadReadInternal,
        Self::PayloadRevealPii,
        Self::RunOperate,
        Self::RunSignal,
        Self::WorkerExecute,
        Self::QuotaRead,
        Self::QuotaManage,
        Self::SecretRead,
        Self::SecretManage,
        Self::SecretUse,
        Self::EgressManage,
        Self::WorkerBind,
        Self::SagaOverride,
        Self::EventPublish,
        Self::EventRead,
        Self::EventManage,
        Self::ScriptRead,
        Self::ScriptWrite,
        Self::ScriptPublish,
        Self::ScriptBuild,
        Self::ScriptExecute,
        Self::OpsRead,
        Self::WorkerManage,
        Self::PublicationApprove,
        Self::PublicationManage,
        Self::VersionActivate,
        Self::DatabaseManage,
        Self::DatabaseUse,
        Self::DatabaseAdHoc,
        Self::DatabaseWrite,
        Self::DatabaseResultRead,
        Self::DatabaseExecute,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceRead => "workspace:read",
            Self::WorkspaceCreate => "workspace:create",
            Self::NamespaceRead => "namespace:read",
            Self::NamespaceCreate => "namespace:create",
            Self::MembershipManage => "membership:manage",
            Self::DefinitionRead => "definition:read",
            Self::DefinitionWrite => "definition:write",
            Self::DefinitionPublish => "definition:publish",
            Self::AuditRead => "audit:read",
            Self::RunStart => "run:start",
            Self::RunRead => "run:read",
            Self::RunQuery => "run:query",
            Self::RunUpdate => "run:update",
            Self::ApprovalRead => "approval:read",
            Self::ApprovalDecide => "approval:decide",
            Self::RunPayloadInspect => "run:payload:inspect",
            Self::PayloadReadInternal => "payload:read-internal",
            Self::PayloadRevealPii => "payload:reveal-pii",
            Self::RunOperate => "run:operate",
            Self::RunSignal => "run:signal",
            Self::WorkerExecute => "worker:execute",
            Self::QuotaRead => "quota:read",
            Self::QuotaManage => "quota:manage",
            Self::SecretRead => "secret:read",
            Self::SecretManage => "secret:manage",
            Self::SecretUse => "secret:use",
            Self::EgressManage => "egress:manage",
            Self::WorkerBind => "worker:bind",
            Self::SagaOverride => "saga:override",
            Self::EventPublish => "event:publish",
            Self::EventRead => "event:read",
            Self::EventManage => "event:manage",
            Self::ScriptRead => "script:read",
            Self::ScriptWrite => "script:write",
            Self::ScriptPublish => "script:publish",
            Self::ScriptBuild => "script:build",
            Self::ScriptExecute => "script:execute",
            Self::OpsRead => "ops:read",
            Self::WorkerManage => "worker:manage",
            Self::PublicationApprove => "publication:approve",
            Self::PublicationManage => "publication:manage",
            Self::VersionActivate => "version:activate",
            Self::DatabaseManage => "database:manage",
            Self::DatabaseUse => "database:use",
            Self::DatabaseAdHoc => "database:adhoc",
            Self::DatabaseWrite => "database:write",
            Self::DatabaseResultRead => "database:result:read",
            Self::DatabaseExecute => "database:execute",
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Built-in role. Custom roles are not built: feature 009 added resource-level control (secret access lists, worker bindings) on top of these roles instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Admin,
    Developer,
    Operator,
    Viewer,
    /// A machine identity that executes tasks; it can do nothing else.
    Worker,
    /// The identity of a sandbox runner: it executes tasks and builds and runs scripts, and can do
    /// nothing else. Kept apart from [`Self::Worker`] so that a worker that serves activities
    /// never reads the source of a script.
    Runner,
    /// Isolated executor for managed database operations. No catalog or general secret access.
    DatabaseRunner,
}

impl Role {
    /// Every decided catalog permission except PII reveal, which always needs a namespace grant.
    pub const ADMIN_PERMISSIONS: &'static [Permission] = &[
        Permission::WorkspaceRead,
        Permission::WorkspaceCreate,
        Permission::NamespaceRead,
        Permission::NamespaceCreate,
        Permission::MembershipManage,
        Permission::DefinitionRead,
        Permission::DefinitionWrite,
        Permission::DefinitionPublish,
        Permission::AuditRead,
        Permission::RunStart,
        Permission::RunRead,
        Permission::RunQuery,
        Permission::RunUpdate,
        Permission::ApprovalRead,
        Permission::ApprovalDecide,
        Permission::RunPayloadInspect,
        Permission::PayloadReadInternal,
        Permission::RunOperate,
        Permission::RunSignal,
        Permission::WorkerExecute,
        Permission::QuotaRead,
        Permission::QuotaManage,
        Permission::SecretRead,
        Permission::SecretManage,
        Permission::SecretUse,
        Permission::EgressManage,
        Permission::WorkerBind,
        Permission::SagaOverride,
        Permission::EventPublish,
        Permission::EventRead,
        Permission::EventManage,
        Permission::ScriptRead,
        Permission::ScriptWrite,
        Permission::ScriptPublish,
        Permission::ScriptBuild,
        Permission::ScriptExecute,
        Permission::OpsRead,
        Permission::WorkerManage,
        Permission::PublicationApprove,
        Permission::PublicationManage,
        Permission::VersionActivate,
        Permission::DatabaseManage,
        Permission::DatabaseUse,
        Permission::DatabaseAdHoc,
        Permission::DatabaseWrite,
        Permission::DatabaseResultRead,
        Permission::DatabaseExecute,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Developer => "developer",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
            Self::Worker => "worker",
            Self::Runner => "runner",
            Self::DatabaseRunner => "database-runner",
        }
    }

    /// Permissions granted to this role. Anything not listed is denied.
    #[must_use]
    pub const fn permissions(self) -> &'static [Permission] {
        use Permission::{
            ApprovalDecide, ApprovalRead, DatabaseAdHoc, DatabaseUse, DefinitionPublish,
            DefinitionRead, DefinitionWrite, EventManage, EventPublish, EventRead, NamespaceRead,
            OpsRead, PayloadReadInternal, PublicationApprove, QuotaRead, RunOperate,
            RunPayloadInspect, RunQuery, RunRead, RunSignal, RunStart, RunUpdate, ScriptBuild,
            ScriptExecute, ScriptPublish, ScriptRead, ScriptWrite, SecretRead, SecretUse,
            VersionActivate, WorkerExecute, WorkerManage, WorkspaceRead,
        };
        match self {
            Self::Admin => Self::ADMIN_PERMISSIONS,
            Self::Developer => &[
                DatabaseUse,
                DatabaseAdHoc,
                WorkspaceRead,
                NamespaceRead,
                DefinitionRead,
                DefinitionWrite,
                DefinitionPublish,
                RunStart,
                RunRead,
                RunQuery,
                RunUpdate,
                PayloadReadInternal,
                ApprovalRead,
                ApprovalDecide,
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
            ],
            Self::Operator => &[
                WorkspaceRead,
                NamespaceRead,
                DefinitionRead,
                RunStart,
                RunRead,
                RunQuery,
                RunUpdate,
                PayloadReadInternal,
                ApprovalRead,
                ApprovalDecide,
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
            Self::Viewer => &[
                WorkspaceRead,
                NamespaceRead,
                DefinitionRead,
                RunRead,
                ApprovalRead,
                QuotaRead,
                EventRead,
                ScriptRead,
                OpsRead,
            ],
            Self::Worker => &[WorkerExecute],
            Self::Runner => &[WorkerExecute, ScriptBuild, ScriptExecute],
            Self::DatabaseRunner => &[WorkerExecute, Permission::DatabaseExecute],
        }
    }

    /// Whether this role grants `permission`.
    #[must_use]
    pub fn grants(self, permission: Permission) -> bool {
        self.permissions().contains(&permission)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = IdentityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "admin" => Ok(Self::Admin),
            "developer" => Ok(Self::Developer),
            "operator" => Ok(Self::Operator),
            "viewer" => Ok(Self::Viewer),
            "worker" => Ok(Self::Worker),
            "runner" => Ok(Self::Runner),
            "database-runner" => Ok(Self::DatabaseRunner),
            other => Err(IdentityError::UnknownRole(clip(other))),
        }
    }
}

/// An authenticated caller bound to exactly one tenant and one role.
///
/// Built by the application layer after token validation and membership lookup; it is the only
/// source of the tenant for every tenant-scoped operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    tenant_id: TenantId,
    subject: Subject,
    role: Role,
}

impl Principal {
    #[must_use]
    pub const fn new(tenant_id: TenantId, subject: Subject, role: Role) -> Self {
        Self {
            tenant_id,
            subject,
            role,
        }
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn subject(&self) -> &Subject {
        &self.subject
    }

    #[must_use]
    pub const fn role(&self) -> Role {
        self.role
    }

    /// # Errors
    /// [`IdentityError::PermissionDenied`] when the role does not grant `permission`.
    pub fn authorize(&self, permission: Permission) -> Result<(), IdentityError> {
        if self.role.grants(permission) {
            Ok(())
        } else {
            Err(IdentityError::PermissionDenied {
                role: self.role,
                permission,
            })
        }
    }
}

/// Top-level isolation boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    id: TenantId,
    slug: Slug,
    created_at: OffsetDateTime,
}

impl Tenant {
    /// Platform-level provisioning. Not reachable from the tenant API: tenants are created by
    /// operators or the development seed, never by a tenant principal.
    #[must_use]
    pub const fn provision(id: TenantId, slug: Slug, now: OffsetDateTime) -> Self {
        Self {
            id,
            slug,
            created_at: now,
        }
    }

    /// Rebuilds a tenant from trusted storage. Performs no authorization.
    #[must_use]
    pub const fn restore(id: TenantId, slug: Slug, created_at: OffsetDateTime) -> Self {
        Self {
            id,
            slug,
            created_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> TenantId {
        self.id
    }

    #[must_use]
    pub const fn slug(&self) -> &Slug {
        &self.slug
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }
}

/// Groups namespaces inside a tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    id: WorkspaceId,
    tenant_id: TenantId,
    slug: Slug,
    created_at: OffsetDateTime,
}

impl Workspace {
    /// Creates a workspace in the principal's own tenant.
    ///
    /// # Errors
    /// [`IdentityError::PermissionDenied`] unless the principal holds `workspace:create`.
    pub fn create(
        principal: &Principal,
        id: WorkspaceId,
        slug: Slug,
        now: OffsetDateTime,
    ) -> Result<Self, IdentityError> {
        principal.authorize(Permission::WorkspaceCreate)?;
        Ok(Self {
            id,
            tenant_id: principal.tenant_id(),
            slug,
            created_at: now,
        })
    }

    /// Rebuilds a workspace from trusted storage. Performs no authorization.
    #[must_use]
    pub const fn restore(
        id: WorkspaceId,
        tenant_id: TenantId,
        slug: Slug,
        created_at: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            tenant_id,
            slug,
            created_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> WorkspaceId {
        self.id
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn slug(&self) -> &Slug {
        &self.slug
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }
}

/// Addressable scope for definitions and runs; unique by slug inside its tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Namespace {
    id: NamespaceId,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    slug: Slug,
    environment: Environment,
    created_at: OffsetDateTime,
}

impl Namespace {
    /// Creates a namespace inside `workspace`, in the principal's own tenant.
    ///
    /// # Errors
    /// - [`IdentityError::PermissionDenied`] unless the principal holds `namespace:create`.
    /// - [`IdentityError::TenantMismatch`] when `workspace` belongs to another tenant.
    pub fn create(
        principal: &Principal,
        workspace: &Workspace,
        id: NamespaceId,
        slug: Slug,
        environment: Environment,
        now: OffsetDateTime,
    ) -> Result<Self, IdentityError> {
        principal.authorize(Permission::NamespaceCreate)?;
        if workspace.tenant_id() != principal.tenant_id() {
            return Err(IdentityError::TenantMismatch);
        }
        Ok(Self {
            id,
            tenant_id: principal.tenant_id(),
            workspace_id: workspace.id(),
            slug,
            environment,
            created_at: now,
        })
    }

    /// Rebuilds a namespace from trusted storage. Performs no authorization.
    #[must_use]
    pub const fn restore(
        id: NamespaceId,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        slug: Slug,
        environment: Environment,
        created_at: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            tenant_id,
            workspace_id,
            slug,
            environment,
            created_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> NamespaceId {
        self.id
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    #[must_use]
    pub const fn slug(&self) -> &Slug {
        &self.slug
    }

    #[must_use]
    pub const fn environment(&self) -> Environment {
        self.environment
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }
}

/// Binds a subject to a role inside one tenant. A subject has at most one membership per tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    tenant_id: TenantId,
    subject: Subject,
    role: Role,
    created_at: OffsetDateTime,
}

impl Membership {
    /// Grants `role` to `subject` in the principal's own tenant.
    ///
    /// # Errors
    /// [`IdentityError::PermissionDenied`] unless the principal holds `membership:manage`.
    pub fn grant(
        principal: &Principal,
        subject: Subject,
        role: Role,
        now: OffsetDateTime,
    ) -> Result<Self, IdentityError> {
        principal.authorize(Permission::MembershipManage)?;
        Ok(Self {
            tenant_id: principal.tenant_id(),
            subject,
            role,
            created_at: now,
        })
    }

    /// First administrator of a freshly provisioned tenant (provisioning/seed path only).
    #[must_use]
    pub const fn bootstrap_admin(tenant: &Tenant, subject: Subject, now: OffsetDateTime) -> Self {
        Self {
            tenant_id: tenant.id(),
            subject,
            role: Role::Admin,
            created_at: now,
        }
    }

    /// Rebuilds a membership from trusted storage. Performs no authorization.
    #[must_use]
    pub const fn restore(
        tenant_id: TenantId,
        subject: Subject,
        role: Role,
        created_at: OffsetDateTime,
    ) -> Self {
        Self {
            tenant_id,
            subject,
            role,
            created_at,
        }
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn subject(&self) -> &Subject {
        &self.subject
    }

    #[must_use]
    pub const fn role(&self) -> Role {
        self.role
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }

    /// The principal this membership authenticates as.
    #[must_use]
    pub fn principal(&self) -> Principal {
        Principal::new(self.tenant_id, self.subject.clone(), self.role)
    }
}
