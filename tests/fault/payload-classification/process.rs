//! Actual control-plane process death around the mandatory disclosure audit commit.
#![allow(clippy::expect_used, clippy::panic)]

use std::fs::File;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use reqwest::Client;
use sagat_application::reveal_grants::RevealGrantService;
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::oneshot;

use super::support;
use super::support::engine::{ctx, document, end_node, world};
use super::support::oidc::{AUDIENCE, ISSUER, TestIdp};

const CANARY: &str = "process-crash-pii-canary";

struct Server {
    child: Child,
    address: SocketAddr,
}

impl Server {
    async fn start(binary: &Path, database: &str, idp: &TestIdp, log: &Path) -> Self {
        let address = TcpListener::bind("127.0.0.1:0")
            .expect("reserve server address")
            .local_addr()
            .expect("server address");
        let output = File::create(log).expect("process log");
        let child = Command::new(binary)
            .arg("serve")
            .env("SAGAT_ENV", "dev")
            .env("DATABASE_URL", database)
            .env("SAGAT_BIND_ADDR", address.to_string())
            .env("SAGAT_ADMIN_ADDR", "off")
            .env("SAGAT_DB_MAX_CONNECTIONS", "8")
            .env("SAGAT_MIGRATE_ON_START", "false")
            .env("SAGAT_OIDC_ISSUER", ISSUER)
            .env("SAGAT_OIDC_AUDIENCE", AUDIENCE)
            .env("SAGAT_OIDC_JWKS_URI", idp.jwks_url())
            .env("SAGAT_ALLOW_INSECURE_JWKS_HTTP", "true")
            .env("OTEL_SDK_DISABLED", "true")
            .stdout(Stdio::from(output.try_clone().expect("clone log")))
            .stderr(Stdio::from(output))
            .spawn()
            .expect("start actual control-plane binary");
        let mut server = Self { child, address };
        let client = Client::new();
        for _ in 0..300 {
            assert!(
                server.child.try_wait().expect("process state").is_none(),
                "control plane exited; inspect the retained process log"
            );
            if client
                .get(format!("http://{address}/v1/ready"))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                return server;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("control plane did not become ready; inspect the retained process log");
    }

    fn kill(&mut self) {
        self.child.kill().expect("kill control-plane process");
        self.child.wait().expect("reap killed control plane");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn request(address: SocketAddr, run: String, token: String) -> reqwest::Result<String> {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("HTTP client")
        .post(format!(
            "http://{address}/v1/runs/{run}/payload-disclosures"
        ))
        .bearer_auth(token)
        .json(&json!({"source":"input","reveal":["pii"]}))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await
}

async fn audit_count(pool: &sqlx::PgPool, run: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_records \
         WHERE action='payload.pii_revealed' AND resource_id=$1",
    )
    .bind(run)
    .fetch_one(pool)
    .await
    .expect("mandatory committed audit count")
}

/// Accept the real request and hold every response byte until the test cuts the connection.
/// Seeing the upstream HTTP response is a post-commit barrier, not a simulated database commit.
async fn hold_response(
    upstream: SocketAddr,
) -> (
    SocketAddr,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("response-loss proxy");
    let address = listener.local_addr().expect("proxy address");
    let (seen, observed) = oneshot::channel();
    let (cut, release) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (client, _) = listener.accept().await.expect("client request");
        let server = TcpStream::connect(upstream).await.expect("upstream");
        let (mut client_read, client_write) = client.into_split();
        let (mut server_read, mut server_write) = server.into_split();
        let forward = tokio::spawn(async move {
            let _ = tokio::io::copy(&mut client_read, &mut server_write).await;
        });
        let mut buffer = [0_u8; 8192];
        assert!(server_read.read(&mut buffer).await.expect("HTTP response") > 0);
        seen.send(()).expect("notify post-commit response barrier");
        let _ = release.await;
        // No response byte has been forwarded to the caller.
        drop(client_write);
        drop(server_read);
        forward.abort();
        let _ = forward.await;
    });
    (address, observed, cut, task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "build the real control plane and set SAGAT_TEST_CONTROL_PLANE and SAGAT_PROCESS_FAULT_LOG_DIR"]
async fn process_crashes_before_and_after_commit_preserve_audit_and_lose_no_authority() {
    sagat_api_http::auth::install_crypto_provider();
    let binary = PathBuf::from(
        std::env::var_os("SAGAT_TEST_CONTROL_PLANE").expect("actual binary is required"),
    );
    assert!(binary.is_file(), "actual control-plane binary must exist");
    let logs = PathBuf::from(
        std::env::var_os("SAGAT_PROCESS_FAULT_LOG_DIR")
            .expect("retained log directory is required"),
    );
    std::fs::create_dir_all(&logs).expect("retained process logs");
    let world = world().await;
    let idp = TestIdp::start().await;
    let tenant: String = sqlx::query_scalar("SELECT slug FROM tenants WHERE id=$1")
        .bind(world.admin.tenant_id().as_uuid())
        .fetch_one(&world.db.pool)
        .await
        .expect("tenant slug");
    let token = idp.token(world.admin.subject().as_str(), &tenant).build();
    let mut source = document("process-fault", "done", vec![end_node("done")]);
    source["inputSchema"] = json!({
        "type":"object", "x-sagat-classification":"pii",
        "properties":{"personal":{"type":"string"}},"additionalProperties":false
    });
    let (workflow, _) = world.publish(source).await;
    let run = world
        .start(workflow, &json!({"personal":CANARY}))
        .await
        .to_string();
    let grants =
        RevealGrantService::new(world.store.clone(), sagat_application::clock::SystemClock);
    let scope = grants
        .prepare(&world.admin, world.namespace.id(), &ctx())
        .await
        .expect("scope");
    grants
        .grant(&scope, world.admin.subject().as_str(), &ctx())
        .await
        .expect("grant");

    sqlx::raw_sql(
        "CREATE FUNCTION pause_reveal() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.action='payload.pii_revealed' THEN \
         PERFORM pg_advisory_xact_lock(16005001); END IF; RETURN NEW; END $$; \
         CREATE TRIGGER pause_reveal BEFORE INSERT ON audit_records \
         FOR EACH ROW EXECUTE FUNCTION pause_reveal();",
    )
    .execute(&world.db.pool)
    .await
    .expect("pre-commit barrier trigger");
    let mut barrier = world.db.pool.acquire().await.expect("barrier connection");
    sqlx::query("SELECT pg_advisory_lock(16005001)")
        .execute(&mut *barrier)
        .await
        .expect("hold pre-commit barrier");
    let mut server = Server::start(
        &binary,
        &world.db.url,
        &idp,
        &logs.join("before-commit.log"),
    )
    .await;
    let response = tokio::spawn(request(server.address, run.clone(), token.clone()));
    support::eventually("disclosure is blocked before audit commit", || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory' AND NOT granted \
             AND database=(SELECT oid FROM pg_database WHERE datname=current_database()))",
        )
        .fetch_one(&world.db.pool)
        .await
        .expect("observe actual lock wait")
        .then_some(())
    })
    .await;
    assert_eq!(audit_count(&world.db.pool, &run).await, 0);
    server.kill();
    assert!(
        response.await.expect("request task").is_err(),
        "killed request returned a body"
    );
    sqlx::query("SELECT pg_advisory_unlock(16005001)")
        .execute(&mut *barrier)
        .await
        .expect("release pre-commit barrier");
    drop(barrier);
    assert_eq!(audit_count(&world.db.pool, &run).await, 0);
    sqlx::raw_sql("DROP TRIGGER pause_reveal ON audit_records; DROP FUNCTION pause_reveal();")
        .execute(&world.db.pool)
        .await
        .expect("remove test-only barrier");

    // A new OS process necessarily starts with a fresh in-process validator cache.
    let mut server = Server::start(
        &binary,
        &world.db.url,
        &idp,
        &logs.join("after-restart.log"),
    )
    .await;
    let answer = request(server.address, run.clone(), token.clone())
        .await
        .expect("restart reveal");
    assert!(
        answer.contains(CANARY),
        "restart did not return the authorized value"
    );
    assert_eq!(audit_count(&world.db.pool, &run).await, 1);
    let (proxy, observed, cut, proxy_task) = hold_response(server.address).await;
    let lost = tokio::spawn(request(proxy, run.clone(), token.clone()));
    tokio::time::timeout(Duration::from_secs(10), observed)
        .await
        .expect("post-commit barrier timeout")
        .expect("upstream responded");
    assert_eq!(audit_count(&world.db.pool, &run).await, 2);
    server.kill();
    cut.send(()).expect("cut held response");
    proxy_task.await.expect("response-loss proxy task");
    assert!(
        lost.await.expect("lost response task").is_err(),
        "proxy returned held values"
    );
    assert_eq!(audit_count(&world.db.pool, &run).await, 2);

    let mut server = Server::start(&binary, &world.db.url, &idp, &logs.join("retry.log")).await;
    let answer = request(server.address, run.clone(), token)
        .await
        .expect("retry after lost response");
    assert!(
        answer.contains(CANARY),
        "retry did not return the authorized value"
    );
    assert_eq!(audit_count(&world.db.pool, &run).await, 3);
    server.kill();
    for name in ["before-commit.log", "after-restart.log", "retry.log"] {
        let log = std::fs::read(logs.join(name)).expect("retained process log");
        assert!(
            !log.windows(CANARY.len())
                .any(|window| window == CANARY.as_bytes()),
            "process log leaked a value"
        );
    }
}
