//! The development environment (`FR-012-003`): what `seed-dev` leaves so that a first workflow can
//! be published the minute the stack is up, and what the development identity provider issues.

use std::time::Duration;

use sagat_control_plane::config::{Config, DatabaseConfig, Environment, NamespaceSeed, SeedConfig};
use sagat_control_plane::{RunError, SeedOutcome, seed_dev, seed_dev_namespace};
use sagat_telemetry::LogFormat;

use sagat_cli::Env;
use sagat_dev_idp::DevKey;

use crate::support::api::{Options, start_with};
use crate::support::{TestDb, migrated_db};
use crate::{Dir, Outcome, run_with};

fn config(db: &TestDb, environment: Environment) -> Config {
    Config {
        environment,
        log_format: LogFormat::Json,
        database: DatabaseConfig {
            url: db.url.clone(),
            max_connections: 5,
            statement_timeout: Duration::from_secs(3),
        },
    }
}

fn seed() -> SeedConfig {
    SeedConfig {
        tenant_slug: "dev".into(),
        admin_subject: "dev-admin".into(),
    }
}

fn namespace() -> NamespaceSeed {
    NamespaceSeed {
        workspace_slug: "dev".into(),
        namespace_slug: "payments".into(),
        environment: "dev".into(),
    }
}

async fn count(db: &TestDb, table: &'static str) -> i64 {
    let sql = match table {
        "workspaces" => "SELECT count(*) FROM workspaces",
        "namespaces" => "SELECT count(*) FROM namespaces",
        other => panic!("not a table the test counts: {other}"),
    };
    sqlx::query_scalar(sql)
        .fetch_one(&db.pool)
        .await
        .expect("count")
}

#[tokio::test]
async fn seed_dev_leaves_a_namespace_for_the_administrator_and_leaves_it_alone_the_second_time() {
    let db = migrated_db().await;
    let cfg = config(&db, Environment::Dev);
    assert_eq!(
        seed_dev(&cfg, &seed()).await.expect("tenant"),
        SeedOutcome::Created
    );

    assert_eq!(
        seed_dev_namespace(&cfg, &seed(), &namespace())
            .await
            .expect("namespace"),
        SeedOutcome::Created
    );
    assert_eq!(
        seed_dev_namespace(&cfg, &seed(), &namespace())
            .await
            .expect("again"),
        SeedOutcome::AlreadyPresent
    );
    assert_eq!(count(&db, "workspaces").await, 1);
    assert_eq!(count(&db, "namespaces").await, 1);
    let (slug, environment): (String, String) =
        sqlx::query_as("SELECT slug, environment FROM namespaces")
            .fetch_one(&db.pool)
            .await
            .expect("namespace");
    assert_eq!((slug.as_str(), environment.as_str()), ("payments", "dev"));

    // Done as the administrator would have, so it is audited as any creation is.
    let audited: Vec<(String,)> = sqlx::query_as(
        "SELECT action FROM audit_records WHERE action IN ('workspace.created','namespace.created') ORDER BY action",
    )
    .fetch_all(&db.pool)
    .await
    .expect("audit");
    assert_eq!(
        audited,
        vec![
            ("namespace.created".to_owned(),),
            ("workspace.created".to_owned(),)
        ]
    );

    // A second namespace goes into the workspace that is there.
    let second = NamespaceSeed {
        namespace_slug: "orders".into(),
        ..namespace()
    };
    assert_eq!(
        seed_dev_namespace(&cfg, &seed(), &second)
            .await
            .expect("second"),
        SeedOutcome::Created
    );
    assert_eq!(count(&db, "workspaces").await, 1);
    assert_eq!(count(&db, "namespaces").await, 2);
}

#[tokio::test]
async fn the_namespace_seed_is_refused_outside_development_and_without_the_tenant() {
    let db = migrated_db().await;
    let production = config(&db, Environment::Prod);
    let refused = seed_dev_namespace(&production, &seed(), &namespace()).await;
    assert!(matches!(refused, Err(RunError::Refused(_))), "{refused:?}");

    // No tenant yet: it says to seed first, and creates nothing.
    let dev = config(&db, Environment::Dev);
    let early = seed_dev_namespace(&dev, &seed(), &namespace()).await;
    assert!(matches!(early, Err(RunError::Refused(_))), "{early:?}");
    assert_eq!(count(&db, "workspaces").await, 0);

    // A slug or environment that is not valid is a configuration error, and creates nothing.
    seed_dev(&dev, &seed()).await.expect("tenant");
    for bad in [
        NamespaceSeed {
            namespace_slug: "Not A Slug".into(),
            ..namespace()
        },
        NamespaceSeed {
            workspace_slug: "".into(),
            ..namespace()
        },
        NamespaceSeed {
            environment: "moon".into(),
            ..namespace()
        },
    ] {
        let refused = seed_dev_namespace(&dev, &seed(), &bad).await;
        assert!(
            matches!(refused, Err(RunError::Config(_))),
            "{bad:?}: {refused:?}"
        );
    }
    assert_eq!(count(&db, "namespaces").await, 0);
}

/// The development identity provider on a port of its own, and its address.
async fn dev_idp() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let issuer = format!("http://{addr}/");
    let pairs = [
        ("SAGAT_ENV", "dev".to_owned()),
        ("SAGAT_DEV_IDP_ADDR", addr.to_string()),
        ("SAGAT_DEV_IDP_ISSUER", issuer),
    ];
    let config = sagat_dev_idp::Config::from_lookup(&|name| {
        pairs
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.clone())
    })
    .expect("config");
    let app = sagat_dev_idp::router(config, DevKey::generate().expect("key"));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

fn cli_env(server: Option<&str>, token: Option<String>, dir: &Dir) -> Env {
    Env {
        server: server.map(str::to_owned),
        token,
        namespace: Some("payments".to_owned()),
        cwd: dir.0.clone(),
    }
}

async fn ask(dir: &Dir, args: &[&str]) -> Outcome {
    run_with(cli_env(None, None, dir), args).await
}

#[tokio::test]
async fn a_token_from_the_development_provider_is_the_first_credential_of_the_first_day() {
    let idp = dev_idp().await;
    // The platform trusts that provider and no other.
    let (issuer, jwks) = (format!("{idp}/"), format!("{idp}/.well-known/jwks.json"));
    let api = start_with(Options {
        auth: Box::new(move |auth| {
            auth.issuer = issuer;
            auth.jwks_uri = jwks;
        }),
        ..Options::default()
    })
    .await;
    // What `docker compose --profile seed run --rm seed` leaves: tenant dev, its administrator
    // dev-admin and the namespace payments.
    let cfg = config(&api.db, Environment::Dev);
    seed_dev(&cfg, &seed()).await.expect("seed");
    seed_dev_namespace(&cfg, &seed(), &namespace())
        .await
        .expect("namespace");
    let dir = Dir::new();

    // The token alone on the output, so that it can be exported.
    let said = ask(
        &dir,
        &[
            "dev",
            "token",
            "--idp",
            &idp,
            "--subject",
            "dev-admin",
            "--tenant",
            "dev",
        ],
    )
    .await;
    said.expect(0)
        .mentions("a token for dev-admin of dev, valid 3600 s");
    let token = said.out.trim().to_owned();
    assert!(
        token.starts_with("eyJ") && !token.contains(char::is_whitespace),
        "{token}"
    );

    // The platform takes it: the whole check of the machine passes with it.
    run_with(
        cli_env(Some(&api.base), Some(token.clone()), &dir),
        &["doctor"],
    )
    .await
    .expect(0)
    .mentions("credentials");

    // Or in a file only its owner reads, for --token-file.
    ask(
        &dir,
        &[
            "dev",
            "token",
            "--idp",
            &idp,
            "--subject",
            "dev-admin",
            "--tenant",
            "dev",
            "--ttl",
            "600",
            "--out",
            "token.txt",
        ],
    )
    .await
    .expect(0)
    .mentions("wrote a token for dev-admin of dev")
    .mentions("valid 600 s")
    .omits("eyJ");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.0.join("token.txt"))
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    run_with(
        cli_env(Some(&api.base), None, &dir),
        &["doctor", "--token-file", "token.txt"],
    )
    .await
    .expect(0);

    // A token is a credential of someone the platform knows: the provider signs for anyone, the
    // platform lets in only its members.
    let stranger = ask(
        &dir,
        &[
            "dev",
            "token",
            "--idp",
            &idp,
            "--subject",
            "mallory",
            "--tenant",
            "dev",
        ],
    )
    .await;
    stranger.expect(0);
    run_with(
        cli_env(Some(&api.base), Some(stranger.out.trim().to_owned()), &dir),
        &["doctor"],
    )
    .await
    .expect(1);

    // And the platform does not trust a token of another provider.
    let other = api.idp.token("dev-admin", "dev").build();
    run_with(cli_env(Some(&api.base), Some(other), &dir), &["doctor"])
        .await
        .expect(1);
}

#[tokio::test]
async fn the_command_says_what_went_wrong_asking_for_a_token() {
    let idp = dev_idp().await;
    let dir = Dir::new();
    // The provider will not sign for the platform's own actors, and says so.
    ask(
        &dir,
        &[
            "dev",
            "token",
            "--idp",
            &idp,
            "--subject",
            "system:operator",
        ],
    )
    .await
    .expect(1)
    .mentions("refused")
    .omits("eyJ");
    // Nothing listening: it says where it looked and what to do.
    ask(&dir, &["dev", "token", "--idp", "http://127.0.0.1:1"])
        .await
        .expect(1)
        .mentions("no answer from the development identity provider")
        .mentions("docker compose up");
    // A provider that is not on this machine is not asked in the clear.
    ask(&dir, &["dev", "token", "--idp", "http://idp.example.com"])
        .await
        .expect(1)
        .mentions("plain http only for localhost");
    // Usage.
    ask(&dir, &["dev", "token", "--ttl", "0"]).await.expect(2);
    ask(&dir, &["dev", "token", "--ttl", "3601"])
        .await
        .expect(2);
    ask(&dir, &["dev"]).await.expect(2);
    ask(&dir, &["dev", "login"]).await.expect(2);
}
