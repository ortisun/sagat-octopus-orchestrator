//! What a running script may ask its supervisor for: a request to the outside, under the egress
//! policy of its namespace, and the secrets its step declared. Nothing else leaves the container
//! and nothing else reaches it.

use std::collections::HashMap;
use std::time::Duration;

use reqwest::Method;
use sagat_application::sandbox::{HostCalls, HttpReply, HttpRequest, Refusal};
use sagat_domain::egress::{EgressPolicy, HostRule};
use sagat_infra_egress::{EgressClient, EgressError, EgressLimits, EgressRequest, ResponseBody};

use crate::api::{EgressView, RunnerError};

/// Largest response a script is given, in bytes.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub struct ScriptHost {
    /// `None`: the script may not reach the network at all.
    egress: Option<EgressClient>,
    /// The secrets the step declared and the platform resolved, by name.
    secrets: HashMap<String, String>,
}

impl std::fmt::Debug for ScriptHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Not the values, not even the names.
        f.debug_struct("ScriptHost")
            .field("egress", &self.egress.is_some())
            .field("secrets", &self.secrets.len())
            .finish()
    }
}

impl ScriptHost {
    /// A host that refuses every request and has no secrets: a build, or a test run.
    #[must_use]
    pub fn closed() -> Self {
        Self {
            egress: None,
            secrets: HashMap::new(),
        }
    }

    /// A host whose requests go through the guarded client for `policy` (the namespace's egress
    /// policy, when the script is allowed any), and that hands out `secrets`.
    ///
    /// # Errors
    /// [`RunnerError::Decode`] for a policy that does not parse; [`RunnerError::Transport`] if the
    /// client cannot be built.
    pub fn open(
        policy: Option<&EgressView>,
        secrets: HashMap<String, String>,
    ) -> Result<Self, RunnerError> {
        let egress = policy
            .map(|view| {
                let rules = view
                    .rules
                    .iter()
                    .map(|rule| HostRule::parse(rule))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| RunnerError::Decode)?;
                let policy = EgressPolicy::new(rules, view.ports.clone(), view.allow_http)
                    .map_err(|_| RunnerError::Decode)?;
                EgressClient::new(
                    policy,
                    EgressLimits {
                        total_timeout: Duration::from_secs(20),
                        max_response_bytes: MAX_RESPONSE_BYTES,
                        ..EgressLimits::default()
                    },
                )
                .map_err(|_| RunnerError::Transport)
            })
            .transpose()?;
        Ok(Self { egress, secrets })
    }

    /// Every secret value this host hands out, to be kept out of logs and results.
    #[must_use]
    pub fn secret_values(&self) -> Vec<String> {
        self.secrets.values().cloned().collect()
    }
}

/// Whether a script may set a header on a request it asks the runner to make. The allowlist is by
/// the host of the URL, so a `Host` the script chose would route the request inside whatever answers
/// at an allowed address; the framing and connection headers belong to the client that sends it.
fn script_may_set(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    !(matches!(
        name.as_str(),
        "host"
            | "content-length"
            | "transfer-encoding"
            | "connection"
            | "upgrade"
            | "te"
            | "trailer"
            | "keep-alive"
            | "expect"
            | "forwarded"
            | "x-forwarded-for"
            | "x-forwarded-host"
            | "x-forwarded-proto"
    ) || name.starts_with("proxy-"))
}

impl HostCalls for ScriptHost {
    async fn fetch(&self, request: HttpRequest) -> Result<HttpReply, Refusal> {
        let Some(client) = &self.egress else {
            return Err(Refusal::Denied);
        };
        let method = Method::from_bytes(request.method.to_ascii_uppercase().as_bytes())
            .map_err(|_| Refusal::Failed)?;
        let sent = client
            .send_with(
                EgressRequest {
                    method,
                    url: request.url,
                    headers: request
                        .headers
                        .into_iter()
                        .filter(|(name, _)| script_may_set(name))
                        .collect(),
                    body: request.body,
                },
                ResponseBody::Keep(MAX_RESPONSE_BYTES),
            )
            .await;
        match sent {
            Ok(response) => Ok(HttpReply {
                status: response.status.as_u16(),
                headers: response.headers,
                body: response.body,
            }),
            Err(EgressError::Denied(_)) => Err(Refusal::Denied),
            Err(
                EgressError::InvalidUrl
                | EgressError::Transport
                | EgressError::Timeout
                | EgressError::ResponseTooLarge
                | EgressError::TooManyRedirects,
            ) => Err(Refusal::Failed),
        }
    }

    async fn secret(&self, name: &str) -> Result<String, Refusal> {
        // A name the step did not declare is refused, and audited by whoever reads the denial. A
        // declared name is always present: the step cannot start without its secrets.
        self.secrets.get(name).cloned().ok_or(Refusal::Denied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_cannot_choose_where_an_allowed_request_is_routed_or_how_it_is_framed() {
        for name in [
            "Host",
            "host",
            "Content-Length",
            "Transfer-Encoding",
            "Connection",
            "Upgrade",
            "TE",
            "Proxy-Authorization",
            "X-Forwarded-Host",
        ] {
            assert!(!script_may_set(name), "{name}");
        }
        for name in [
            "Authorization",
            "Content-Type",
            "Accept",
            "Idempotency-Key",
            "X-Api-Key",
        ] {
            assert!(script_may_set(name), "{name}");
        }
    }

    #[tokio::test]
    async fn a_closed_host_refuses_everything() {
        let host = ScriptHost::closed();
        let request = HttpRequest {
            method: "GET".to_owned(),
            url: "https://example.com".to_owned(),
            headers: vec![],
            body: vec![],
        };
        assert_eq!(host.fetch(request).await.map(|_| ()), Err(Refusal::Denied));
        assert_eq!(host.secret("any").await, Err(Refusal::Denied));
    }

    #[tokio::test]
    async fn only_the_declared_secrets_are_handed_out_and_never_printed() {
        let host = ScriptHost::open(
            None,
            HashMap::from([("api-key".to_owned(), "s3cret-value-123".to_owned())]),
        )
        .expect("host");
        assert_eq!(
            host.secret("api-key").await.as_deref(),
            Ok("s3cret-value-123")
        );
        assert_eq!(host.secret("other").await, Err(Refusal::Denied));
        assert!(!format!("{host:?}").contains("s3cret"));
        assert_eq!(host.secret_values(), vec!["s3cret-value-123".to_owned()]);
    }

    #[tokio::test]
    async fn a_policy_that_names_nothing_reaches_nothing_and_the_metadata_address_never() {
        let view = EgressView {
            rules: vec!["*.example.com".to_owned()],
            ports: vec![443],
            allow_http: false,
        };
        let host = ScriptHost::open(Some(&view), HashMap::new()).expect("host");
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "https://169.254.169.254/",
            "https://other.test/",
            "http://api.example.com/",
        ] {
            let request = HttpRequest {
                method: "GET".to_owned(),
                url: url.to_owned(),
                headers: vec![],
                body: vec![],
            };
            assert_eq!(
                host.fetch(request).await.map(|_| ()),
                Err(Refusal::Denied),
                "{url}"
            );
        }
    }
}
