//! `dev-idp serve | healthcheck | version`.

use std::process::ExitCode;

use sagat_dev_idp::{Config, DevKey, router};

const USAGE: &str = "\
dev-idp: a development identity provider. It signs a token for any subject of any tenant:
it only runs when SAGAT_ENV=dev.

USAGE:
    dev-idp serve          answer GET /.well-known/jwks.json and POST /token
    dev-idp healthcheck    exit 0 when the provider on SAGAT_DEV_IDP_ADDR answers
    dev-idp version

ENVIRONMENT:
    SAGAT_ENV                    must be dev
    SAGAT_DEV_IDP_ADDR           where to listen                        [127.0.0.1:9100]
    SAGAT_DEV_IDP_EXPOSED        true to listen on a non-loopback address (a container)
    SAGAT_DEV_IDP_ISSUER         the iss of the tokens, as SAGAT_OIDC_ISSUER spells it
                                                                        [http://127.0.0.1:<port>/]
    SAGAT_DEV_IDP_AUDIENCE       the aud of the tokens                  [sagat-octopus-orchestrator]
    SAGAT_DEV_IDP_TENANT_CLAIM   the claim that carries the tenant      [tenant]
    SAGAT_DEV_IDP_KEY_FILE       keep the signing key here (mode 600), so that tokens outlive a restart
    SAGAT_DEV_IDP_HOSTS          other host:port names a request may use (a published port), comma separated
";

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") => run(false).await,
        Some("healthcheck") => run(true).await,
        Some("version") => {
            println!("dev-idp {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("help" | "--help" | "-h") | None => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: '{other}' is not a command\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

async fn run(probe: bool) -> ExitCode {
    let config = match Config::from_lookup(&env) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    if probe {
        return healthcheck(&config).await;
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let key = match &config.key_file {
        Some(path) => DevKey::load_or_create(path),
        None => DevKey::generate(),
    };
    let key = match key {
        Ok(key) => key,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(1);
        }
    };
    let listener = match tokio::net::TcpListener::bind(config.addr).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("error: cannot listen on {}: {error}", config.addr);
            return ExitCode::from(1);
        }
    };
    tracing::info!(
        addr = %config.addr, issuer = %config.issuer, kid = %key.kid(),
        persistent_key = config.key_file.is_some(),
        "development identity provider: it signs a token for anyone who asks; never use it for real data"
    );
    let served = axum::serve(listener, router(config, key))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

/// A plain request to the provider's own `/livez`: the image has no shell tooling.
async fn healthcheck(config: &Config) -> ExitCode {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let port = config.addr.port();
    let attempt = async {
        let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
        socket
            .write_all(
                format!(
                    "GET /livez HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await?;
        let mut answer = Vec::new();
        socket.read_to_end(&mut answer).await?;
        Ok::<_, std::io::Error>(answer.starts_with(b"HTTP/1.1 200"))
    };
    match tokio::time::timeout(std::time::Duration::from_secs(3), attempt).await {
        Ok(Ok(true)) => ExitCode::SUCCESS,
        _ => ExitCode::from(1),
    }
}
