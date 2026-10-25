//! A startup check that the database role the process runs as cannot rewrite the audit trail.
//!
//! The migrations grant the runtime role `sagat_app` exactly what the control plane needs, and
//! `audit_records` is protected by triggers. All of that is moot if an operator points
//! `DATABASE_URL` at the owner role or at a superuser: such a role can disable the triggers, drop
//! the table or rewrite it. [`verify`] turns that mistake into a refusal to start instead of a
//! silent loss of the guarantee.

use std::fmt;

use sqlx::{PgPool, Row};

/// One way in which the connected role is too powerful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeFinding {
    /// A superuser bypasses every privilege check and can switch the audit triggers off.
    Superuser,
    /// The owner of a table can alter it, drop its triggers and grant itself anything.
    OwnsAuditTrail,
    CanUpdateAuditTrail,
    CanDeleteAuditTrail,
    CanTruncateAuditTrail,
    /// The role owns an append-only table of the execution plane.
    OwnsAppendOnly(&'static str),
    /// The role may update, delete from or truncate an append-only table of the execution plane.
    CanRewriteAppendOnly(&'static str),
}

impl fmt::Display for PrivilegeFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Superuser => "it is a PostgreSQL superuser",
            Self::OwnsAuditTrail => {
                "it owns the audit trail table (directly or through a role it belongs to)"
            }
            Self::CanUpdateAuditTrail => "it may UPDATE audit records",
            Self::CanDeleteAuditTrail => "it may DELETE audit records",
            Self::CanTruncateAuditTrail => "it may TRUNCATE the audit trail",
            Self::OwnsAppendOnly(table) => {
                return write!(f, "it owns the append-only table '{table}'");
            }
            Self::CanRewriteAppendOnly(table) => {
                return write!(
                    f,
                    "it may UPDATE, DELETE or TRUNCATE the append-only table '{table}'"
                );
            }
        })
    }
}

/// Why the process must not serve requests with the connected database role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeastPrivilegeError {
    /// The role can do more than the control plane needs. Names the role and every reason; never a
    /// connection string or password.
    TooPrivileged {
        role: String,
        findings: Vec<PrivilegeFinding>,
    },
    /// `audit_records` does not exist, so the role's privileges on it cannot be established (and the
    /// role that later migrates the schema would own what the process then serves with).
    SchemaMissing,
    /// The role could not be inspected (the database failed or was unreachable).
    Unavailable(String),
}

impl fmt::Display for LeastPrivilegeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooPrivileged { role, findings } => {
                write!(
                    f,
                    "database role '{role}' is too privileged to serve requests: "
                )?;
                for (i, finding) in findings.iter().enumerate() {
                    if i > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{finding}")?;
                }
                f.write_str(
                    ". Connect as the least-privilege runtime role (sagat_app) and keep the \
                     owner role for `migrate` and `provision-tenant`",
                )
            }
            Self::SchemaMissing => f.write_str(
                "the audit trail does not exist yet, so the database role's privileges cannot be \
                 verified; run `control-plane migrate` before starting the API",
            ),
            Self::Unavailable(detail) => {
                write!(
                    f,
                    "could not inspect the database role's privileges: {detail}"
                )
            }
        }
    }
}

impl std::error::Error for LeastPrivilegeError {}

/// What `pg_roles` / the catalog say about the role of this connection and `audit_records`.
const INSPECT: &str = "\
SELECT \
    r.rolname::text AS role, \
    r.rolsuper AS superuser, \
    c.oid IS NOT NULL AS table_exists, \
    COALESCE(pg_has_role(current_user, c.relowner, 'MEMBER'), false) AS owns, \
    COALESCE(has_any_column_privilege(current_user, c.oid, 'UPDATE'), false) AS can_update, \
    COALESCE(has_table_privilege(current_user, c.oid, 'DELETE'), false) AS can_delete, \
    COALESCE(has_table_privilege(current_user, c.oid, 'TRUNCATE'), false) AS can_truncate \
FROM pg_roles r \
LEFT JOIN pg_class c ON c.oid = to_regclass('public.audit_records')::oid \
WHERE r.rolname = current_user";

/// Tables nobody may rewrite or erase: the journal, the payloads and the published versions. Checked
/// when they exist (a schema migrated only up to feature 001 has none of them).
const APPEND_ONLY: [&str; 4] = [
    "run_events",
    "payloads",
    "workflow_versions",
    "leaf_collection_success_certifications",
];

const INSPECT_APPEND_ONLY: &str = "\
SELECT c.oid IS NOT NULL AS table_exists, \
    COALESCE(pg_has_role(current_user, c.relowner, 'MEMBER'), false) AS owns, \
    COALESCE(has_any_column_privilege(current_user, c.oid, 'UPDATE'), false) \
        OR COALESCE(has_table_privilege(current_user, c.oid, 'DELETE'), false) \
        OR COALESCE(has_table_privilege(current_user, c.oid, 'TRUNCATE'), false) AS can_rewrite \
FROM (SELECT to_regclass($1)::oid AS oid) t LEFT JOIN pg_class c ON c.oid = t.oid";

pub(crate) async fn verify(pool: &PgPool) -> Result<(), LeastPrivilegeError> {
    let unavailable = |error: sqlx::Error| LeastPrivilegeError::Unavailable(error.to_string());
    let row = sqlx::query(INSPECT)
        .fetch_one(pool)
        .await
        .map_err(unavailable)?;
    let role: String = row.try_get("role").map_err(unavailable)?;
    let flag = |name: &str| row.try_get::<bool, _>(name).map_err(unavailable);

    let mut findings = Vec::new();
    if flag("superuser")? {
        findings.push(PrivilegeFinding::Superuser);
    }
    let table_exists = flag("table_exists")?;
    if table_exists {
        for (column, finding) in [
            ("owns", PrivilegeFinding::OwnsAuditTrail),
            ("can_update", PrivilegeFinding::CanUpdateAuditTrail),
            ("can_delete", PrivilegeFinding::CanDeleteAuditTrail),
            ("can_truncate", PrivilegeFinding::CanTruncateAuditTrail),
        ] {
            if flag(column)? {
                findings.push(finding);
            }
        }
    }
    for table in APPEND_ONLY {
        let row = sqlx::query(INSPECT_APPEND_ONLY)
            .bind(format!("public.{table}"))
            .fetch_one(pool)
            .await
            .map_err(unavailable)?;
        if !row
            .try_get::<bool, _>("table_exists")
            .map_err(unavailable)?
        {
            continue;
        }
        if row.try_get::<bool, _>("owns").map_err(unavailable)? {
            findings.push(PrivilegeFinding::OwnsAppendOnly(table));
        }
        if row.try_get::<bool, _>("can_rewrite").map_err(unavailable)? {
            findings.push(PrivilegeFinding::CanRewriteAppendOnly(table));
        }
    }
    if !findings.is_empty() {
        return Err(LeastPrivilegeError::TooPrivileged { role, findings });
    }
    if table_exists {
        Ok(())
    } else {
        Err(LeastPrivilegeError::SchemaMissing)
    }
}
