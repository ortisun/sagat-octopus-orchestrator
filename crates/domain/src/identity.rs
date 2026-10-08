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

    /// Validates and wraps a subject.
    ///
    /// # Errors
    /// Returns [`IdentityError::InvalidSubject`] when empty, too long or containing control chars.
    pub fn parse(value: &str) -> Result<Self, IdentityError> {
        let violation = if value.trim().is_empty() {
            Some(SubjectViolation::Empty)
        } else if value.len() > Self::MAX_LEN {
            Some(SubjectViolation::TooLong)
        } else if value.chars().any(char::is_control) {
            Some(SubjectViolation::ControlCharacter)
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
            other => Err(IdentityError::UnknownEnvironment(other.to_owned())),
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
}

impl Permission {
    /// Every permission, used to grant the admin role and to test the full matrix.
    pub const ALL: [Self; 9] = [
        Self::WorkspaceRead,
        Self::WorkspaceCreate,
        Self::NamespaceRead,
        Self::NamespaceCreate,
        Self::MembershipManage,
        Self::DefinitionRead,
        Self::DefinitionWrite,
        Self::DefinitionPublish,
        Self::AuditRead,
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
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Built-in role. Custom roles are out of scope for feature 001 (advanced RBAC is feature 009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Admin,
    Developer,
    Operator,
    Viewer,
}

impl Role {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Developer => "developer",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }

    /// Permissions granted to this role. Anything not listed is denied.
    ///
    /// Operator currently equals Viewer; run-level operations (pause, retry, ...) will extend it
    /// in feature 002.
    #[must_use]
    pub const fn permissions(self) -> &'static [Permission] {
        use Permission::{
            DefinitionPublish, DefinitionRead, DefinitionWrite, NamespaceRead, WorkspaceRead,
        };
        match self {
            Self::Admin => &Permission::ALL,
            Self::Developer => &[
                WorkspaceRead,
                NamespaceRead,
                DefinitionRead,
                DefinitionWrite,
                DefinitionPublish,
            ],
            Self::Operator | Self::Viewer => &[WorkspaceRead, NamespaceRead, DefinitionRead],
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
            other => Err(IdentityError::UnknownRole(other.to_owned())),
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
