//! Settings of the runner, read from environment variables like the other processes'.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sagat_control_plane::config::{ConfigError, Environment, Lookup, optional, required};
use sagat_domain::identity::NamespaceId;
use sagat_domain::scripts::Policy;
use sagat_infra_sandbox::{GoTarget, Images};

use crate::api::Credentials;

/// A namespace whose script steps this runner serves: the worker protocol names it by slug, the
/// REST calls by id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceTarget {
    pub slug: String,
    pub id: NamespaceId,
}

#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// The API root of the control plane, e.g. `https://control-plane/v1`.
    pub api_url: String,
    /// The worker protocol endpoint, e.g. `https://engine:8443`.
    pub grpc_url: String,
    pub credentials: Credentials,
    /// An extra root certificate (PEM) for the control plane and the engine.
    pub ca_pem: Option<Vec<u8>>,
    /// The name the runner gives its worker sessions.
    pub name: String,
    pub namespaces: Vec<NamespaceTarget>,
    /// Containers at once.
    pub max_concurrent: usize,
    /// Pause between looks at the queues when there was nothing to do.
    pub poll_interval: Duration,
    /// The most any script is given here, whatever its artifact says.
    pub ceiling: Policy,
    /// Public keys (Ed25519, raw) whose signatures are trusted. Empty: the control plane is asked
    /// for its key (trust on first use).
    pub trusted_keys: Vec<Vec<u8>>,
    pub images: Images,
    /// One native architecture for every Go runner eligible for this pool.
    pub go_target: Option<GoTarget>,
    /// Refuse startup if the native Go capability probe fails.
    pub go_required: bool,
    /// Where `/metrics` and `/livez` are served; `None` disables the listener.
    pub admin_addr: Option<SocketAddr>,
    /// Containers older than this that nobody owns are removed (a runner that died mid-run).
    pub sweep_after: Duration,
}

fn namespaces(spec: &str) -> Result<Vec<NamespaceTarget>, ConfigError> {
    let bad = |reason: &str| ConfigError::Invalid {
        name: "SAGAT_RUNNER_NAMESPACES",
        reason: reason.to_owned(),
    };
    spec.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (slug, id) = part
                .split_once('=')
                .ok_or_else(|| bad("expected slug=id pairs, separated by commas"))?;
            Ok(NamespaceTarget {
                slug: slug.trim().to_owned(),
                id: id.trim().parse().map_err(|_| bad("an id is not a UUID"))?,
            })
        })
        .collect()
}

impl RunnerConfig {
    /// # Errors
    /// [`ConfigError`] naming the first missing or invalid variable.
    pub fn from_lookup(get: Lookup<'_>) -> Result<Self, ConfigError> {
        let go_target = get("SAGAT_RUNNER_GO_TARGET")
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                value
                    .trim()
                    .parse::<GoTarget>()
                    .map_err(|reason| ConfigError::Invalid {
                        name: "SAGAT_RUNNER_GO_TARGET",
                        reason: reason.to_owned(),
                    })
            })
            .transpose()?;
        let go_required = optional(get, "SAGAT_RUNNER_GO_REQUIRED", false)?;
        if go_required && go_target.is_none() {
            return Err(ConfigError::Invalid {
                name: "SAGAT_RUNNER_GO_REQUIRED",
                reason: "requires SAGAT_RUNNER_GO_TARGET".to_owned(),
            });
        }
        let environment: Environment = optional(get, "SAGAT_ENV", Environment::Prod)?;
        let api_url = required(get, "SAGAT_RUNNER_API_URL")?;
        let grpc_url = required(get, "SAGAT_RUNNER_GRPC_URL")?;
        for (name, url) in [
            ("SAGAT_RUNNER_API_URL", &api_url),
            ("SAGAT_RUNNER_GRPC_URL", &grpc_url),
        ] {
            let plain = url.starts_with("http://");
            if !plain && !url.starts_with("https://") {
                return Err(ConfigError::Invalid {
                    name,
                    reason: "is not an http(s) URL".to_owned(),
                });
            }
            // The runner presents credentials and receives the source of its tenant's scripts.
            if plain && environment != Environment::Dev {
                return Err(ConfigError::Invalid {
                    name,
                    reason: "must be https outside SAGAT_ENV=dev".to_owned(),
                });
            }
        }
        let credentials = match (
            get("SAGAT_RUNNER_TOKEN").filter(|v| !v.trim().is_empty()),
            get("SAGAT_RUNNER_TOKEN_FILE").filter(|v| !v.trim().is_empty()),
        ) {
            (Some(token), None) => Credentials::Static(token.trim().to_owned()),
            (None, Some(path)) => Credentials::File(PathBuf::from(path.trim())),
            (None, None) => return Err(ConfigError::Missing("SAGAT_RUNNER_TOKEN_FILE")),
            (Some(_), Some(_)) => {
                return Err(ConfigError::Invalid {
                    name: "SAGAT_RUNNER_TOKEN",
                    reason: "set either the token or a file that holds it, not both".to_owned(),
                });
            }
        };
        let ca_pem = get("SAGAT_RUNNER_CA_FILE")
            .filter(|v| !v.trim().is_empty())
            .map(|path| {
                std::fs::read(path.trim()).map_err(|e| ConfigError::Invalid {
                    name: "SAGAT_RUNNER_CA_FILE",
                    reason: format!("cannot be read ({})", e.kind()),
                })
            })
            .transpose()?;
        let ceiling = match get("SAGAT_RUNNER_CEILING")
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
        {
            None => Policy::default(),
            Some(text) => Policy::parse_canonical(&text).map_err(|e| ConfigError::Invalid {
                name: "SAGAT_RUNNER_CEILING",
                reason: e.to_string(),
            })?,
        };
        let trusted_keys = get("SAGAT_RUNNER_TRUSTED_KEYS")
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
            .map(|list| {
                list.split(',')
                    .map(|key| {
                        STANDARD
                            .decode(key.trim())
                            .map_err(|_| ConfigError::Invalid {
                                name: "SAGAT_RUNNER_TRUSTED_KEYS",
                                reason: "a key is not base64".to_owned(),
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let defaults = Images::default();
        let admin_addr =
            match get("SAGAT_ADMIN_ADDR").as_deref().map(str::trim) {
                None | Some("") => Some(SocketAddr::from(([127, 0, 0, 1], 9092))),
                Some("off") => None,
                Some(raw) => Some(raw.parse().map_err(|e: std::net::AddrParseError| {
                    ConfigError::Invalid {
                        name: "SAGAT_ADMIN_ADDR",
                        reason: e.to_string(),
                    }
                })?),
            };
        let max_concurrent: u64 = optional(get, "SAGAT_RUNNER_MAX_CONCURRENT", 4)?;
        if max_concurrent == 0 {
            return Err(ConfigError::Invalid {
                name: "SAGAT_RUNNER_MAX_CONCURRENT",
                reason: "must be greater than zero".to_owned(),
            });
        }
        Ok(Self {
            api_url,
            grpc_url,
            credentials,
            ca_pem,
            name: optional(get, "SAGAT_RUNNER_NAME", "sandbox-runner".to_owned())?,
            namespaces: get("SAGAT_RUNNER_NAMESPACES")
                .map(|spec| namespaces(&spec))
                .transpose()?
                .unwrap_or_default(),
            max_concurrent: usize::try_from(max_concurrent).unwrap_or(4),
            poll_interval: Duration::from_millis(optional(get, "SAGAT_RUNNER_POLL_MS", 500)?),
            ceiling,
            trusted_keys,
            go_target,
            go_required,
            images: Images {
                typescript: optional(get, "SAGAT_RUNNER_IMAGE_TYPESCRIPT", defaults.typescript)?,
                python: optional(get, "SAGAT_RUNNER_IMAGE_PYTHON", defaults.python)?,
                csharp_build: optional(
                    get,
                    "SAGAT_RUNNER_IMAGE_CSHARP_BUILD",
                    defaults.csharp_build,
                )?,
                csharp_runtime: optional(
                    get,
                    "SAGAT_RUNNER_IMAGE_CSHARP_RUNTIME",
                    defaults.csharp_runtime,
                )?,
                go_build: optional(get, "SAGAT_RUNNER_IMAGE_GO_BUILD", defaults.go_build)?,
                go_runtime: optional(get, "SAGAT_RUNNER_IMAGE_GO_RUNTIME", defaults.go_runtime)?,
                java_build: optional(get, "SAGAT_RUNNER_IMAGE_JAVA_BUILD", defaults.java_build)?,
                java_runtime: optional(
                    get,
                    "SAGAT_RUNNER_IMAGE_JAVA_RUNTIME",
                    defaults.java_runtime,
                )?,
            },
            admin_addr,
            sweep_after: Duration::from_secs(optional(get, "SAGAT_RUNNER_SWEEP_AFTER_SECS", 3600)?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| map.get(name).cloned()
    }

    const BASE: [(&str, &str); 3] = [
        ("SAGAT_RUNNER_API_URL", "https://cp.example/v1"),
        ("SAGAT_RUNNER_GRPC_URL", "https://engine.example:8443"),
        ("SAGAT_RUNNER_TOKEN", "t"),
    ];

    #[test]
    fn a_runner_needs_where_the_control_plane_is_and_who_it_is() {
        let ok = RunnerConfig::from_lookup(&lookup(&BASE)).expect("config");
        assert_eq!(ok.max_concurrent, 4);
        assert_eq!(ok.ceiling, Policy::default());
        assert!(ok.namespaces.is_empty() && ok.trusted_keys.is_empty());
        assert!(matches!(
            RunnerConfig::from_lookup(&lookup(&BASE[..2])),
            Err(ConfigError::Missing(_))
        ));
        assert!(RunnerConfig::from_lookup(&lookup(&[])).is_err());
    }

    #[test]
    fn credentials_and_plain_http_are_not_sent_outside_development() {
        let mut http = BASE.to_vec();
        http[0] = ("SAGAT_RUNNER_API_URL", "http://cp.example/v1");
        assert!(RunnerConfig::from_lookup(&lookup(&http)).is_err());
        http.push(("SAGAT_ENV", "dev"));
        assert!(RunnerConfig::from_lookup(&lookup(&http)).is_ok());
        let mut both = BASE.to_vec();
        both.push(("SAGAT_RUNNER_TOKEN_FILE", "/run/token"));
        assert!(
            RunnerConfig::from_lookup(&lookup(&both)).is_err(),
            "one source of credentials"
        );
        let mut elsewhere = BASE.to_vec();
        elsewhere[0] = ("SAGAT_RUNNER_API_URL", "ftp://cp.example");
        assert!(RunnerConfig::from_lookup(&lookup(&elsewhere)).is_err());
    }

    #[test]
    fn namespaces_keys_and_ceiling_are_read_or_refused() {
        let id = "018f3b9a-0000-7000-8000-000000000001";
        let mut with = BASE.to_vec();
        let spec = format!("payments={id}, ledger={id}");
        with.push(("SAGAT_RUNNER_NAMESPACES", spec.as_str()));
        with.push(("SAGAT_RUNNER_TRUSTED_KEYS", "AAAA,BBBB"));
        with.push((
            "SAGAT_RUNNER_CEILING",
            "v1;memory=134217728;cpu=500;wall=10;pids=32;output=8192;scratch=8388608;egress=none",
        ));
        let config = RunnerConfig::from_lookup(&lookup(&with)).expect("config");
        assert_eq!(config.namespaces.len(), 2);
        assert_eq!(config.namespaces[0].slug, "payments");
        assert_eq!(config.trusted_keys.len(), 2);
        assert_eq!(config.ceiling.limits.cpu_millis(), 500);
        for (name, value) in [
            ("SAGAT_RUNNER_NAMESPACES", "payments"),
            ("SAGAT_RUNNER_NAMESPACES", "payments=not-a-uuid"),
            ("SAGAT_RUNNER_TRUSTED_KEYS", "!!!"),
            ("SAGAT_RUNNER_CEILING", "v1;memory=1"),
            ("SAGAT_RUNNER_MAX_CONCURRENT", "0"),
        ] {
            let mut bad = BASE.to_vec();
            bad.push((name, value));
            assert!(
                RunnerConfig::from_lookup(&lookup(&bad)).is_err(),
                "{name}={value}"
            );
        }
    }

    #[test]
    fn native_go_requires_an_explicit_supported_target() {
        for (target, expected) in [
            ("linux-arm64", GoTarget::LinuxArm64),
            ("linux-amd64", GoTarget::LinuxAmd64),
        ] {
            let mut pairs = BASE.to_vec();
            pairs.extend([
                ("SAGAT_RUNNER_GO_TARGET", target),
                ("SAGAT_RUNNER_GO_REQUIRED", "true"),
            ]);
            let config = RunnerConfig::from_lookup(&lookup(&pairs)).expect("native target");
            assert_eq!(config.go_target, Some(expected));
            assert!(config.go_required);
        }
        for pair in [
            ("SAGAT_RUNNER_GO_TARGET", "darwin-arm64"),
            ("SAGAT_RUNNER_GO_REQUIRED", "true"),
        ] {
            let mut pairs = BASE.to_vec();
            pairs.push(pair);
            assert!(RunnerConfig::from_lookup(&lookup(&pairs)).is_err());
        }
        let config = RunnerConfig::from_lookup(&lookup(&BASE)).expect("default");
        assert!(config.go_target.is_none() && !config.go_required);
    }

    #[test]
    fn a_token_never_prints() {
        let config = RunnerConfig::from_lookup(&lookup(&BASE)).expect("config");
        assert!(!format!("{config:?}").contains("Static(\"t\")"));
    }
}
