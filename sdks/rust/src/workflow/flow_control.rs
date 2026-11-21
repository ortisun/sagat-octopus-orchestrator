//! Feature 022 (FR-022-017, FR-022-039): the flow-control policies of a workflow and of its activity and script steps.
//! The builders assemble a policy with the IR's names; the server checks every bound at publication.
use super::*;

/// A concurrency gate: at most `limit` holders of one key. At the workflow level a run holds a permit from its start to
/// its end and `key` reads `input`; on a step an attempt holds it and `key` reads the run data when the step becomes
/// ready. A member that is `None` is left out of the document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConcurrencyPolicy {
    /// The most holders of one key (1 to 10000).
    pub limit: u32,
    /// An expression whose value is the key; `None`, the whole group is one key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Shared by the definitions of a namespace that name it; `None`, derived from the workflow (and node).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// How long a run or a ready step waits for a permit (1 ms to 1 day); `None`, the server's 10 minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_timeout_ms: Option<u64>,
}

impl ConcurrencyPolicy {
    /// At most `limit` holders of one key, with no other member.
    #[must_use]
    pub const fn new(limit: u32) -> Self {
        Self {
            limit,
            key: None,
            group: None,
            queue_timeout_ms: None,
        }
    }
}

/// The sliding window of a rate limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RatePeriod {
    Second,
    Minute,
    Hour,
}

/// A rate limit: at most `limit` grants of one key in any window of `per`, strictly (there is no burst). At the workflow
/// level a run is granted at its start and `key` reads `input`; on a step a grant counts from its lease. A member that is
/// `None` is left out of the document; the server checks the ceiling of each window (100 a second, 6000 a minute).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RateLimitPolicy {
    /// The most grants of one key in a window (1 to 10000).
    pub limit: u32,
    /// The length of the window.
    pub per: RatePeriod,
    /// An expression whose value is the key; `None`, the whole group is one key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Shared by the definitions of a namespace that name it; `None`, derived from the workflow (and node).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// How long a run or a step waits for a slot (1 ms to 1 day); `None`, the server's 10 minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_timeout_ms: Option<u64>,
}

impl RateLimitPolicy {
    /// At most `limit` grants of one key in a window of `per`, with no other member.
    #[must_use]
    pub const fn new(limit: u32, per: RatePeriod) -> Self {
        Self {
            limit,
            per,
            key: None,
            group: None,
            queue_timeout_ms: None,
        }
    }
}

impl Builder {
    /// Gates the start of every run: a run holds a permit of its key from its start to its end, and waits for one when
    /// the key has `limit` holders. The key reads `input`. A second call replaces the first.
    pub fn concurrency(&self, policy: ConcurrencyPolicy) -> &Self {
        self.run_gate("concurrency", serde_json::to_value(policy))
    }

    /// Limits the starts of runs: at most `limit` runs of one key are granted in any window of `per`; the others wait,
    /// in admission order. The key reads `input`. A second call replaces the first.
    pub fn rate_limit(&self, policy: RateLimitPolicy) -> &Self {
        self.run_gate("rateLimit", serde_json::to_value(policy))
    }

    fn run_gate(&self, field: &str, value: serde_json::Result<Value>) -> &Self {
        if !self.root_only() {
            return self;
        }
        let Ok(value) = value else {
            return self.fail(&format!("invalid {field}"));
        };
        self.change(|d| {
            d.fields.insert(field.into(), value);
        });
        self
    }
}
