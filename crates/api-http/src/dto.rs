//! Wire types. They differ from the domain entities on purpose: camelCase names, RFC 3339
//! timestamps, and the tenant omitted (it is implied by the credentials).

use sagat_domain::identity::{Namespace, Workspace};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBody {
    pub id: String,
    pub slug: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl From<&Workspace> for WorkspaceBody {
    fn from(w: &Workspace) -> Self {
        Self {
            id: w.id().to_string(),
            slug: w.slug().to_string(),
            created_at: w.created_at(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceBody {
    pub id: String,
    pub workspace_id: String,
    pub slug: String,
    pub environment: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl From<&Namespace> for NamespaceBody {
    fn from(n: &Namespace) -> Self {
        Self {
            id: n.id().to_string(),
            workspace_id: n.workspace_id().to_string(),
            slug: n.slug().to_string(),
            environment: n.environment().to_string(),
            created_at: n.created_at(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageBody<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthContextBody {
    pub tenant: String,
    pub subject: String,
    pub role: String,
    pub permissions: Vec<&'static str>,
    pub trace_id: String,
}

#[derive(Debug, Serialize)]
pub struct HealthBody {
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateWorkspaceBody {
    pub slug: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateNamespaceBody {
    pub slug: String,
    pub environment: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}
