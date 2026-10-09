//! Wire types of the governance routes.

use sagat_application::bindings::{BindingView, Revoked};
use sagat_application::egress::StoredPolicy;
use sagat_application::quotas::QuotaView;
use sagat_application::secrets::SecretView;
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretBody {
    pub name: String,
    pub version: u32,
    /// Which key of the ring the current version is sealed under.
    pub key_id: String,
    pub created_by: String,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}

impl From<&SecretView> for SecretBody {
    fn from(view: &SecretView) -> Self {
        Self {
            name: view.name.clone(),
            version: view.version,
            key_id: view.key_id.clone(),
            created_by: view.created_by.clone(),
            updated_at: view.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SecretListBody {
    pub items: Vec<SecretBody>,
}

#[derive(Debug, Serialize)]
pub struct SecretAccessBody {
    pub subjects: Vec<String>,
}

/// The value of a secret on its way in. It is the only place the API accepts one, and it never
/// prints.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetSecretBody {
    pub value: String,
}

impl std::fmt::Debug for SetSecretBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetSecretBody")
            .field("value", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EgressPolicyBody {
    /// Exact host names, `*.domain` rules and CIDR networks, in canonical text.
    pub rules: Vec<String>,
    /// The ports that may be reached; empty means 443 (and 80 when plain HTTP is allowed).
    pub ports: Vec<u16>,
    pub allow_http: bool,
}

impl From<&StoredPolicy> for EgressPolicyBody {
    fn from(policy: &StoredPolicy) -> Self {
        Self {
            rules: policy.rules.clone(),
            ports: policy.ports.clone(),
            allow_http: policy.allow_http,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetEgressPolicyBody {
    pub rules: Vec<String>,
    #[serde(default)]
    pub ports: Vec<u16>,
    #[serde(default)]
    pub allow_http: bool,
}
