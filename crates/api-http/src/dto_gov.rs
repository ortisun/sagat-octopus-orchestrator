//! Wire types of the governance routes.

use sagat_application::bindings::{BindingView, Revoked};
use sagat_application::quotas::QuotaView;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaBody {
    pub resource: &'static str,
    pub limit: u64,
    /// Where the limit comes from: `default`, `tenant` or `namespace`.
    pub source: &'static str,
    /// How much is used now; absent for a per-namespace resource in the tenant-wide view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<u64>,
}

impl From<&QuotaView> for QuotaBody {
    fn from(view: &QuotaView) -> Self {
        Self {
            resource: view.resource.as_str(),
            limit: view.limit,
            source: view.source.as_str(),
            used: view.used,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct QuotaListBody {
    pub items: Vec<QuotaBody>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetQuotaBody {
    pub limit: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokedBody {
    /// Worker sessions closed because the binding no longer covers what they advertise.
    pub sessions_closed: u64,
    /// Attempts those sessions held, released for the scheduler to reclaim.
    pub leases_released: u64,
}

impl From<Revoked> for RevokedBody {
    fn from(revoked: Revoked) -> Self {
        Self {
            sessions_closed: revoked.sessions,
            leases_released: revoked.leases,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingBody {
    pub subject: String,
    pub activities: Vec<String>,
    /// Only in the answer to a change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked: Option<RevokedBody>,
}

impl BindingBody {
    #[must_use]
    pub fn listed(view: &BindingView) -> Self {
        Self {
            subject: view.subject.clone(),
            activities: view.patterns.clone(),
            revoked: None,
        }
    }

    #[must_use]
    pub fn changed(view: &BindingView) -> Self {
        Self {
            revoked: Some(view.revoked.into()),
            ..Self::listed(view)
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BindingListBody {
    pub items: Vec<BindingBody>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetBindingBody {
    pub activities: Vec<String>,
}
