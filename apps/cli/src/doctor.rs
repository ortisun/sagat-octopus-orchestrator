//! `sagat doctor`: is this machine ready to work against that platform, and if not, what to do.
//!
//! It looks at the things that go wrong on the first day, in the order a person would look: the
//! address, the token, whether anything answers, whether it is ready, whether the token is accepted
//! and what it may do, whether the versions fit, whether the namespace is there, the worker
//! endpoint and, if asked, the message broker. Each finding says what to do about it. The token is
//! read to see when it expires and who it is for, never printed and never sent anywhere but to the
//! platform.

use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::net::TcpStream;

use crate::args::Common;
use crate::client::{CallError, Client};
use crate::{EXIT_DONE, EXIT_FAILED, Env, Io, namespace_of};

/// The worker protocol this tool offers when it serves activities.
pub const WORKER_PROTOCOL: &str = "1.2";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
    Skip,
}

impl Status {
    fn word(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "FAIL",
            Self::Skip => "skip",
        }
    }
}

struct Check {
    name: &'static str,
    status: Status,
    detail: String,
    help: Option<String>,
}

fn check(
    name: &'static str,
    status: Status,
    detail: impl Into<String>,
    help: Option<&str>,
) -> Check {
    Check {
        name,
        status,
        detail: detail.into(),
        help: help.map(str::to_owned),
    }
}

/// What the token says about itself, without verifying it (the platform does that).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TokenFacts {
    pub expires_at: Option<u64>,
    pub issued_at: Option<u64>,
    pub subject: Option<String>,
    pub issuer: Option<String>,
}

/// Reads the claims of a JWT; `None` when it is not one.
#[must_use]
pub fn read_token(token: &str) -> Option<TokenFacts> {
    let mut parts = token.split('.');
    let (_, payload, _) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    let text = |key: &str| claims.get(key).and_then(Value::as_str).map(str::to_owned);
    Some(TokenFacts {
        expires_at: claims.get("exp").and_then(Value::as_u64),
        issued_at: claims.get("iat").and_then(Value::as_u64),
        subject: text("sub"),
        issuer: text("iss"),
    })
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn minutes(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{} h {} min", seconds / 3600, (seconds % 3600) / 60)
    } else {
        format!("{} min", seconds.div_ceil(60))
    }
}

fn token_check(token: Option<&str>) -> Check {
    let Some(token) = token else {
        return check(
            "token",
            Status::Fail,
            "no token",
            Some(
                "pass --token-file <path> or set SAGAT_TOKEN; in the local stack, `docs/quickstarts/README.md` shows how to get one",
            ),
        );
    };
    let Some(facts) = read_token(token) else {
        return check(
            "token",
            Status::Warn,
            "present, and not a JWT: its expiry cannot be checked here",
            Some("if the platform refuses it, get a new token from your identity provider"),
        );
    };
    let who = facts
        .subject
        .as_deref()
        .map_or_else(String::new, |s| format!(" for {}", crate::one_line(s)));
    match facts.expires_at {
        Some(exp) if exp <= now() => check(
            "token",
            Status::Fail,
            format!("expired {} ago{who}", minutes(now() - exp)),
            Some("get a new token from your identity provider and run doctor again"),
        ),
        Some(exp) if exp - now() < 300 => check(
            "token",
            Status::Warn,
            format!("expires in {}{who}", minutes(exp - now())),
            Some("get a new token before a long command"),
        ),
        Some(exp) => {
            // The platform refuses a token that is allowed to live more than an hour (the span from
            // its `iat` to its `exp`), however young it is.
            let lives = facts.issued_at.map(|iat| exp.saturating_sub(iat));
            if let Some(span) = lives.filter(|span| *span > 3600) {
                check(
                    "token",
                    Status::Warn,
                    format!(
                        "expires in {}{who}, but it is made to live {}",
                        minutes(exp - now()),
                        minutes(span)
                    ),
                    Some(
                        "the platform refuses a token that lives more than an hour: ask your identity provider for shorter ones",
                    ),
                )
            } else {
                check(
                    "token",
                    Status::Ok,
                    format!("expires in {}{who}", minutes(exp - now())),
                    None,
                )
            }
        }
        None => check("token", Status::Warn, format!("no expiry in it{who}"), None),
    }
}

/// `major.minor` of a version, if it has them.
fn major_minor(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

fn protocol_minor(version: &str) -> Option<u32> {
    version.strip_prefix("1.")?.parse().ok()
}

/// A host and port out of an address like `http://host:port/...` or `nats://host:port`.
fn host_port(address: &str, default_port: u16) -> Option<(String, u16)> {
    let url = reqwest::Url::parse(address).ok()?;
    let host = url.host_str()?.trim_matches(['[', ']']).to_owned();
    Some((host, url.port().unwrap_or(default_port)))
}

/// Resolves and connects, and says what failed.
async fn connect(host: &str, port: u16) -> Result<(TcpStream, Duration), String> {
    let started = Instant::now();
    let attempt =
        tokio::time::timeout(Duration::from_secs(4), TcpStream::connect((host, port))).await;
    match attempt {
        Err(_) => Err(format!(
            "{host}:{port} did not answer in 4 s (a firewall, or nothing there)"
        )),
        Ok(Err(e)) => {
            let text = e.to_string().to_lowercase();
            Err(
                if text.contains("lookup")
                    || text.contains("resolve")
                    || text.contains("not known")
                    || text.contains("nodename")
                    || text.contains("no such host")
                {
                    format!("the name {host} does not resolve (DNS)")
                } else if e.kind() == std::io::ErrorKind::ConnectionRefused {
                    format!("connection refused: nothing listens at {host}:{port}")
                } else {
                    format!("could not connect to {host}:{port} ({})", e.kind())
                },
            )
        }
        Ok(Ok(stream)) => Ok((stream, started.elapsed())),
    }
}

async fn nats_check(address: &str) -> Check {
    let Some((host, port)) = host_port(address, 4222) else {
        return check(
            "broker",
            Status::Fail,
            "the address is not a URL like nats://host:4222",
            None,
        );
    };
    match connect(&host, port).await {
        Err(why) => check(
            "broker",
            Status::Fail,
            why,
            Some(
                "start the broker (the local stack does: docker compose up) or fix the address; from a container use the service name, from the host the published port",
            ),
        ),
        Ok((mut stream, took)) => {
            // A NATS server greets with `INFO {...}` before the client says anything.
            use tokio::io::AsyncReadExt as _;
            let mut greeting = [0u8; 5];
            let read =
                tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut greeting))
                    .await;
            if matches!(read, Ok(Ok(_))) && &greeting == b"INFO " {
                check(
                    "broker",
                    Status::Ok,
                    format!("{host}:{port} is a NATS server ({} ms)", took.as_millis()),
                    None,
                )
            } else {
                check(
                    "broker",
                    Status::Warn,
                    format!("{host}:{port} accepts connections and does not greet like NATS"),
                    Some(
                        "check that this is the broker's client port (4222 by default), not its monitoring port",
                    ),
                )
            }
        }
    }
}

async fn grpc_check(address: &str) -> Check {
    let Some((host, port)) = host_port(address, 443) else {
        return check(
            "worker",
            Status::Fail,
            "the address is not a URL like http://host:19000",
            None,
        );
    };
    match connect(&host, port).await {
        Err(why) => check(
            "worker",
            Status::Fail,
            why,
            Some(
                "start the execution engine (the local stack does) or fix --grpc / SAGAT_GRPC; its worker port is not the API's",
            ),
        ),
        Ok((_, took)) => check(
            "worker",
            Status::Ok,
            format!(
                "{host}:{port} accepts connections ({} ms)",
                took.as_millis()
            ),
            None,
        ),
    }
}

/// What the role may do of the things the tool does, from the permissions the platform reports.
fn abilities(permissions: &[String]) -> String {
    let has = |p: &str| permissions.iter().any(|x| x == p);
    let things = [
        (
            "publish workflows",
            has("definition:write") && has("definition:publish"),
        ),
        ("start runs", has("run:start")),
        ("operate runs", has("run:operate")),
        (
            "test and publish scripts",
            has("script:write") && has("script:publish"),
        ),
        ("serve activities (worker dev)", has("worker:execute")),
        ("move the active version", has("version:activate")),
    ];
    let can: Vec<&str> = things.iter().filter(|t| t.1).map(|t| t.0).collect();
    let cannot: Vec<&str> = things.iter().filter(|t| !t.1).map(|t| t.0).collect();
    if cannot.is_empty() {
        "can do everything the tool does".to_owned()
    } else {
        format!("can: {}; cannot: {}", can.join(", "), cannot.join(", "))
    }
}

pub(crate) async fn run(
    common: &Common,
    env: &Env,
    json: bool,
    nats: Option<&str>,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let mut checks: Vec<Check> = Vec::new();

    // The address.
    let server = common.server.clone().or_else(|| env.server.clone());
    let token_text = match &common.token_file {
        Some(path) => std::fs::read_to_string(crate::resolve(env, path))
            .ok()
            .map(|t| t.trim().to_owned()),
        None => env.token.clone(),
    };
    let token_unreadable = common.token_file.is_some() && token_text.is_none();
    let Some(server) = server else {
        checks.push(check(
            "address",
            Status::Fail,
            "no address for the platform",
            Some("pass --server <url> or set SAGAT_SERVER, e.g. http://127.0.0.1:18080/v1 for the local stack"),
        ));
        return finish(checks, json, io);
    };
    // A token that cannot travel in a header is told apart from an address that is wrong.
    let header_safe = token_text
        .as_deref()
        .is_some_and(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_graphic()));
    let token_text = if token_text.is_some() && !header_safe {
        checks.push(check(
            "token",
            Status::Fail,
            "the token is empty or has characters that cannot travel in a header (a space, a line break)",
            Some("a token is one line; check the file you gave to --token-file for a trailing line or quotes"),
        ));
        None
    } else {
        token_text
    };
    let client = match Client::new(&server, token_text.as_deref().unwrap_or("none"), "") {
        Ok(client) => {
            checks.push(check("address", Status::Ok, client.base().to_owned(), None));
            client
        }
        Err(why) => {
            checks.push(check(
                "address",
                Status::Fail,
                why,
                Some("use https://..., or http://localhost... for a platform on this machine"),
            ));
            return finish(checks, json, io);
        }
    };

    // The token.
    if checks.iter().any(|c| c.name == "token") {
        // Already told: it cannot be used.
    } else if token_unreadable {
        checks.push(check(
            "token",
            Status::Fail,
            "the token file cannot be read",
            Some("check the path given to --token-file"),
        ));
    } else {
        checks.push(token_check(token_text.as_deref()));
    }

    // Reachability and readiness: no credentials.
    let started = Instant::now();
    match client.public("/health").await {
        Ok((200, _)) => checks.push(check("reachable", Status::Ok, format!("/health answered in {} ms", started.elapsed().as_millis()), None)),
        Ok((status, _)) => checks.push(check(
            "reachable",
            Status::Fail,
            format!("/health answered {status}"),
            Some("the address is not the platform's API, or something between you and it answers instead (a proxy, a login page): the API's address ends in /v1"),
        )),
        Err(CallError::Transport(why)) => {
            checks.push(check(
                "reachable",
                Status::Fail,
                why,
                Some("check the address and that the platform is running; for the local stack, `docker compose ps` and `docs/quickstarts/README.md`"),
            ));
            return finish(checks, json, io);
        }
        Err(CallError::Refused(f)) => checks.push(check("reachable", Status::Fail, f.to_string(), None)),
    }
    match client.public("/ready").await {
        Ok((200, _)) => checks.push(check("ready", Status::Ok, "the platform's storage answers", None)),
        Ok((status, _)) => checks.push(check(
            "ready",
            Status::Fail,
            format!("/ready answered {status}: the platform is up and cannot reach its database"),
            Some("look at the platform's logs and at its database; commands that write will fail until it is ready"),
        )),
        Err(e) => checks.push(check("ready", Status::Fail, e.to_string(), None)),
    }

    // The token is accepted, and what it may do.
    let mut permissions: Vec<String> = Vec::new();
    let mut tenant = None;
    if token_text.is_none() {
        checks.push(check("credentials", Status::Skip, "no token to try", None));
    } else {
        match client.call("GET", "/auth/context", None, None).await {
            Ok((_, body)) => {
                permissions = body["permissions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.as_str().map(str::to_owned))
                    .collect();
                tenant = body["tenant"].as_str().map(str::to_owned);
                checks.push(check(
                    "credentials",
                    Status::Ok,
                    format!(
                        "the platform knows {} as {} in tenant {}; {}",
                        crate::one_line(body["subject"].as_str().unwrap_or("?")),
                        crate::one_line(body["role"].as_str().unwrap_or("?")),
                        crate::one_line(tenant.as_deref().unwrap_or("?")),
                        abilities(&permissions)
                    ),
                    None,
                ));
            }
            Err(CallError::Refused(f)) => checks.push(check(
                "credentials",
                Status::Fail,
                f.to_string(),
                Some("the token is not accepted: it may be expired, for another platform (issuer, audience), or for a user that is not a member of a tenant; get a token the platform's identity provider issues for it"),
            )),
            Err(e) => checks.push(check("credentials", Status::Fail, e.to_string(), None)),
        }
    }

    // Versions.
    if token_text.is_some() && !permissions.is_empty() {
        match client.call("GET", "/version", None, None).await {
            Ok((_, body)) => {
                let server_version = crate::one_line(body["version"].as_str().unwrap_or("?"));
                let ours = env!("CARGO_PKG_VERSION");
                let max = crate::one_line(body["workerProtocol"]["maximum"].as_str().unwrap_or("?"));
                let mut status = Status::Ok;
                let mut help = None;
                let mut detail = format!("platform {server_version}, this tool {ours}, worker protocol up to {max}");
                if major_minor(&server_version) != major_minor(ours) {
                    status = Status::Warn;
                    help = Some("the tool and the platform are not the same release: a command may be missing from the older one; use the same release of both".to_owned());
                }
                if let (Some(offered), Some(top)) = (protocol_minor(WORKER_PROTOCOL), protocol_minor(&max))
                    && top < offered
                {
                    status = Status::Warn;
                    detail.push_str(&format!("; `worker dev` offers {WORKER_PROTOCOL} and will be served at {max}"));
                }
                checks.push(Check { name: "versions", status, detail, help });
            }
            Err(CallError::Refused(f)) if f.status == 404 => checks.push(check(
                "versions",
                Status::Warn,
                format!("the platform has no /version: it is older than this tool ({})", env!("CARGO_PKG_VERSION")),
                Some("update the platform, or use the release of the tool that matches it; commands added since may not work"),
            )),
            Err(e) => checks.push(check("versions", Status::Warn, e.to_string(), None)),
        }
    }

    // The namespace.
    if token_text.is_some() && !permissions.is_empty() {
        let wanted = namespace_of(common, env).ok();
        match client
            .call("GET", "/namespaces?limit=200", None, None)
            .await
        {
            Ok((_, body)) => {
                let slugs: Vec<String> = body["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|n| n["slug"].as_str().map(str::to_owned))
                    .collect();
                match wanted {
                    Some(slug) if slugs.contains(&slug) => {
                        checks.push(check("namespace", Status::Ok, format!("{slug} exists in tenant {}", tenant.as_deref().unwrap_or("?")), None));
                    }
                    Some(slug) => checks.push(check(
                        "namespace",
                        Status::Fail,
                        format!("'{}' is not a namespace of this tenant ({})", crate::one_line(&slug), if slugs.is_empty() { "it has none".to_owned() } else { format!("it has: {}", slugs.iter().take(10).map(String::as_str).collect::<Vec<_>>().join(", ")) }),
                        Some("choose one of them with --namespace, or ask an administrator to create it (POST /workspaces/{id}/namespaces)"),
                    )),
                    None => checks.push(check(
                        "namespace",
                        Status::Warn,
                        if slugs.is_empty() { "none chosen, and the tenant has none".to_owned() } else { format!("none chosen; the tenant has: {}", slugs.iter().take(10).map(String::as_str).collect::<Vec<_>>().join(", ")) },
                        Some("pass --namespace <slug>, set SAGAT_NAMESPACE, or run next to a manifest"),
                    )),
                }
            }
            Err(e) => checks.push(check("namespace", Status::Fail, e.to_string(), None)),
        }
    }

    // The worker endpoint and the broker.
    match common.grpc.clone().or_else(|| env.grpc.clone()) {
        Some(address) => checks.push(grpc_check(&address).await),
        None => checks.push(check(
            "worker",
            Status::Skip,
            "no worker endpoint given",
            Some("pass --grpc <url> (or set SAGAT_GRPC) to check the endpoint `worker dev` and your SDK workers will use"),
        )),
    }
    if let Some(address) = nats {
        checks.push(nats_check(address).await);
    }

    // The Studio: where the runs can be seen.
    let origin = client.base().trim_end_matches("/v1").to_owned();
    if let Ok(studio) = Client::new(&origin, "none", "") {
        match studio.public("/studio/").await {
            Ok((200, _)) => checks.push(check("studio", Status::Ok, format!("{origin}/studio/"), None)),
            Ok(_) | Err(_) => checks.push(check(
                "studio",
                Status::Skip,
                "the platform does not serve the Studio here",
                Some("set SAGAT_STUDIO_DIR on the control plane to serve it (the local stack does); the runs are readable through the API either way"),
            )),
        }
    }
    finish(checks, json, io)
}

fn finish(checks: Vec<Check>, json: bool, io: &mut Io<'_>) -> Result<u8, String> {
    let failed = checks.iter().filter(|c| c.status == Status::Fail).count();
    if json {
        let items: Vec<Value> = checks
            .iter()
            .map(|c| {
                json!({
                    "name": c.name,
                    "status": c.status.word(),
                    "detail": c.detail,
                    "help": c.help,
                })
            })
            .collect();
        io.say(
            serde_json::to_string_pretty(&json!({ "ok": failed == 0, "checks": items }))
                .map_err(|e| e.to_string())?,
        );
    } else {
        for c in &checks {
            io.say(format!(
                "{:<5} {:<12} {}",
                c.status.word(),
                c.name,
                c.detail
            ));
            if c.status != Status::Ok
                && let Some(help) = &c.help
            {
                io.say(format!("      help: {help}"));
            }
        }
        match failed {
            0 => io.say("nothing to fix"),
            1 => io.say("1 thing to fix"),
            n => io.say(format!("{n} things to fix")),
        }
    }
    Ok(if failed == 0 { EXIT_DONE } else { EXIT_FAILED })
}
