//! Wire types of the governance routes.

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
