//! Real provider/target/executor fixture for the same five-language database scenario.
use crate::support::worker::{NAMESPACE, Platform};
use reqwest::Method;
use sagat_application::secret_provider::SecretProvider;
use sagat_domain::identity::Role;
use sagat_infra_secrets::openbao::{OpenBao, OpenBaoConfig};
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{ContainerAsync, GenericImage},
};
use uuid::Uuid;

struct Process(std::process::Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub struct Fixture {
    runner: Option<Process>,
    provider: Arc<OpenBao>,
    admin: sqlx::PgPool,
    port: u16,
    address: String,
    token: PathBuf,
    dir: tempfile::TempDir,
    _target: ContainerAsync<Postgres>,
    _bao: ContainerAsync<GenericImage>,
}
impl Fixture {
    pub async fn start() -> Self {
        let dir = tempfile::tempdir().expect("database fixture directory");
        let bao = crate::support::bounded::start_openbao("sdk-bootstrap").await;
        let address = format!(
            "http://127.0.0.1:{}",
            bao.get_host_port_ipv4(8200).await.expect("port")
        );
        let token = dir.path().join("bao-token");
        super::write_private(&token, b"sdk-bootstrap").expect("token");
        let provider = Arc::new(
            OpenBao::new(OpenBaoConfig {
                address: address.clone(),
                mount: "secret".into(),
                token_file: token.clone(),
                timeout: Duration::from_secs(5),
                development_loopback: true,
                ca_pem: None,
            })
            .expect("provider"),
        );
        let target = crate::support::bounded::start_postgres_target().await;
        let port = target.get_host_port_ipv4(5432).await.expect("port");
        let admin = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&format!(
                "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
            ))
            .await
            .expect("target admin");
        sqlx::raw_sql("CREATE ROLE sdk_reader LOGIN PASSWORD 'sdk-database-password-canary'; CREATE TABLE effects(id bigint PRIMARY KEY, amount numeric); GRANT SELECT,INSERT ON effects TO sdk_reader;")
            .execute(&admin).await.expect("target restricted role");
        Self {
            runner: None,
            provider,
            admin,
            port,
            address,
            token,
            dir,
            _target: target,
            _bao: bao,
        }
    }

    pub fn provider(&self) -> Arc<dyn SecretProvider> {
        self.provider.clone()
    }

    pub fn engine_env(&self) -> HashMap<String, String> {
        [
            ("SAGAT_OPENBAO_ADDRESS".into(), self.address.clone()),
            (
                "SAGAT_OPENBAO_TOKEN_FILE".into(),
                self.token.to_string_lossy().into_owned(),
            ),
            ("SAGAT_OPENBAO_DEVELOPMENT".into(), "true".into()),
        ]
        .into_iter()
        .collect()
    }

    pub async fn configure(&mut self, platform: &Platform, grpc_url: &str) {
        let ns = platform.namespace_id().await;
        let credential =
            json!({"username":"sdk_reader","password":"sdk-database-password-canary"}).to_string();
        let (status, response) = platform.rest(Method::PUT, &format!("/namespaces/{ns}/secrets/database-credential"), Some(&json!({
            "value":credential,"requestId":Uuid::new_v4().to_string(),"expectedVersion":0,"requiredProvider":"openbao"
        }))).await;
        assert_eq!(status, 200, "{response}");
        // The shared typed fixture pins revision 2, so exercise explicit and unpinned references.
        for revision in 0..2 {
            let (status, response) = platform.rest(Method::PUT, &format!("/namespaces/{ns}/connections/orders-db"), Some(&json!({
                "requestId":Uuid::new_v4().to_string(),"expectedRevision":revision,"status":"ACTIVE","config":{
                    "driver":"postgresql","host":"127.0.0.1","port":self.port,"database":"postgres",
                    "tls":{"mode":"development-plaintext"},"secretName":"database-credential"
                }
            }))).await;
            assert_eq!(status, 200, "{response}");
        }
        platform
            .grant(platform.tenant, "database-runner", Role::DatabaseRunner)
            .await;
        platform.bind_everything("database-runner").await;
        let token = self.dir.path().join("runner-token");
        super::write_private(
            &token,
            super::token(platform, "database-runner", "acme").as_bytes(),
        )
        .expect("runner token");
        let limits = json!({"maxStatementBytes":16384,"maxParameters":64,"maxParameterBytes":65536,
            "maxRows":1000,"maxResultBytes":1048576,"timeoutMs":10000,"connectTimeoutMs":5000,"poolSize":4,"idleTimeoutMs":30000});
        let config = json!({"grpcUrl":grpc_url,"tokenFile":token,"name":"sdk-managed-executor",
            "maxConcurrent":2,"pollIntervalMs":100,"rpcTimeoutMs":5000,"developmentLoopback":true,
            "deployment":{"allowCidrs":["127.0.0.0/8"],"allowPorts":[self.port],"denyCidrs":[],"limits":limits},
            "namespaces":[{"id":ns,"slug":NAMESPACE,"allowCidrs":["127.0.0.0/8"],"allowPorts":[self.port],"limits":limits}]
        });
        let config_path = self.dir.path().join("runner.json");
        super::write_private(&config_path, config.to_string().as_bytes()).expect("runner config");
        let binary = std::env::var_os("SAGAT_DATABASE_RUNNER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::support::built_binary("database-runner"));
        assert!(
            binary.is_file(),
            "build database-runner before database SDK conformance"
        );
        let log = std::fs::File::create(self.dir.path().join("runner.log")).expect("runner logs");
        self.runner = Some(Process(
            std::process::Command::new(binary)
                .env_clear()
                .env("SAGAT_DATABASE_RUNNER_CONFIG", config_path)
                .env("SAGAT_DATABASE_RUNNER_METRICS_ADDR", "127.0.0.1:0")
                .stdout(Stdio::from(log.try_clone().expect("log")))
                .stderr(Stdio::from(log))
                .spawn()
                .expect("separate runner"),
        ));
    }

    /// Independent assertions are executed even when blessing goldens: failed runs or rounded values
    /// cannot become the expected output merely because five SDKs agree on them.
    pub async fn evidence(&self, platform: &Platform, out: &Value) -> Result<Value, String> {
        let observations = out["observations"]
            .as_array()
            .ok_or("missing observations")?;
        for (id, status) in [
            ("typed-done", "SUCCEEDED"),
            ("write-done", "SUCCEEDED"),
            ("unsupported-done", "FAILED"),
        ] {
            let observation = observations
                .iter()
                .find(|item| item["id"] == id)
                .ok_or("missing database observation")?;
            if observation["ok"] != true || observation["result"]["status"] != status {
                let logs =
                    std::fs::read_to_string(self.dir.path().join("runner.log")).unwrap_or_default();
                return Err(format!(
                    "unexpected {id} database outcome: {observation}; runner: {logs}"
                ));
            }
        }
        let calls = out["workerLog"]["w1"]
            .as_array()
            .ok_or("missing database consumer calls")?;
        if calls.len() != 2 {
            return Err("managed SQL must never be dispatched to an ordinary SDK worker".into());
        }
        let runs = out["serverState"]
            .as_array()
            .ok_or("missing database graph")?;
        let mut managed = 0;
        for run in runs {
            for node in run["nodes"].as_array().ok_or("missing graph nodes")? {
                let expected = match node["id"].as_str() {
                    Some("query" | "dependent" | "write" | "refused") => {
                        managed += 1;
                        "database"
                    }
                    Some("consume") => "activity",
                    _ => return Err("unexpected database graph node".into()),
                };
                if node["type"] != expected {
                    return Err("database graph lost its authored node type".into());
                }
            }
        }
        if managed != 4 {
            return Err("database graph is missing a managed operation".into());
        }
        let typed = calls
            .iter()
            .find(|call| call["steps"].get("query").is_some())
            .ok_or("no typed consumer")?;
        let expected = json!([
            {"type":"null","value":null}, {"type":"boolean","value":true},
            {"type":"integer","value":"9223372036854775807"},
            {"type":"decimal","value":"12345678901234567890.123456789"},
            {"type":"float","value":1.25}, {"type":"text","value":"café"},
            {"type":"timestamp","value":"2026-10-17T12:30:00Z"},
            {"type":"date","value":"2026-10-17"}, {"type":"binary","value":"AP8="},
            {"type":"json","value":{"order":"9007199254740993"}},
            {"type":"integer","value":"9007199254740993"}
        ]);
        if typed["steps"]["query"]["rows"][0] != expected
            || typed["steps"]["dependent"]["rows"][0][0]["value"] != "9223372036854775807"
        {
            return Err(format!("database typed result differs: {}", typed["steps"]));
        }
        let write = calls
            .iter()
            .find(|call| call["steps"].get("write").is_some())
            .ok_or("no write consumer")?;
        if write["steps"]["write"]["affectedRows"] != "1"
            || write["steps"]["write"]["rows"][0][1]["value"] != "19.90"
        {
            return Err("write result changed representation".into());
        }
        let effects: Vec<(i64, String)> =
            sqlx::query_as("SELECT id, amount::text FROM effects ORDER BY id")
                .fetch_all(&self.admin)
                .await
                .map_err(|_| "target read failed")?;
        if effects != vec![(7, "19.90".to_owned())] {
            return Err("target write was lost or repeated".into());
        }
        let failure: (String, i32) =
            sqlx::query_as("SELECT failure_code, attempt FROM steps WHERE node='refused'")
                .fetch_one(&platform.api.db.pool)
                .await
                .map_err(|_| "failure evidence unavailable")?;
        if failure != ("DATABASE_STATEMENT_DENIED".into(), 1) {
            return Err("unsupported statement retried or misclassified".into());
        }
        let public = serde_json::to_string(out).expect("json");
        if public.contains("sdk-database-password-canary") || public.contains("sdk-bootstrap") {
            return Err("a provider credential reached an SDK".into());
        }
        Ok(
            json!({"effects":[{"id":"7","amount":"19.90"}],"failure":{"code":failure.0,"attempts":failure.1},"typed":typed["steps"]["query"]}),
        )
    }
}
