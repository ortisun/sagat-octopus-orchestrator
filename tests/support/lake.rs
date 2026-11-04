//! Disposable fixtures for the Apache Iceberg write steps and the step-output destinations (feature 035).
//!
//! Each test starts its own containers from the images pinned in `infra/docker/compose.lake.yaml` (D035-041),
//! on a private network that goes away with them. Nothing here talks to a service started by hand or to a
//! container of another project (AGENTS.md rule 16).
//!
//! The platform never creates a bucket, a namespace or a table: a missing table fails the step (D035-017).
//! [`LakeAdmin`] is the fixture admin client that does it for the tests, with plain HTTP against the object
//! store (S3 signature version 4, below) and the REST catalog.
//!
//! First step: an S3-compatible store (RustFS) and the Iceberg REST reference catalog. The GCS, Azure, Glue,
//! Hive, JDBC, Lakekeeper and ClickHouse fixtures of the compose file are added with the first test that needs
//! each of them.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use chrono::Utc;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use testcontainers_modules::testcontainers::{
    ContainerAsync, GenericImage, ImageExt, core::IntoContainerPort, runners::AsyncRunner,
};

use super::bounded::{START_LIMIT, within};

/// Object store and REST catalog images: tag and digest, as in `compose.lake.yaml`.
pub const RUSTFS: (&str, &str) = (
    "rustfs/rustfs",
    "1.0.1@sha256:1803faef57627e2d9c2e7d89d655d712ddded5389040054987163043fecb6a3c",
);
pub const ICEBERG_REST: (&str, &str) = (
    "apache/iceberg-rest-fixture",
    "1.10.1@sha256:f7d679d30ac9c640bdeb2c015dff533cd3c8f1c7d491ebcb5d436f9a42db1d6f",
);

/// Development credentials, the same for everyone; they protect nothing.
pub const ACCESS_KEY: &str = "lakeaccess";
pub const SECRET_KEY: &str = "lakesecret-dev-only";
pub const REGION: &str = "us-east-1";
/// The bucket the REST fixture's warehouse (`s3://warehouse/`) points at.
pub const BUCKET: &str = "warehouse";

/// Polls `url` until it answers with a success status. A service that never does ends the stage in a panic that
/// names it (see `bounded.rs`), instead of a hung test.
async fn wait_ready(url: &str, what: &str) {
    let client = reqwest::Client::new();
    within(format!("{what} answers {url}"), START_LIMIT, async {
        loop {
            if let Ok(response) = client.get(url).send().await
                && response.status().is_success()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await;
}

/// A running object store and REST catalog. Dropping it removes the containers.
pub struct RestLake {
    _store: ContainerAsync<GenericImage>,
    _catalog: ContainerAsync<GenericImage>,
    /// `http://127.0.0.1:<port>`: how the test process reaches the object store.
    pub s3_endpoint: String,
    /// `http://127.0.0.1:<port>`: how the test process reaches the REST catalog.
    pub catalog_uri: String,
}

impl RestLake {
    /// Starts the object store, then the catalog (which needs it), and creates the warehouse bucket.
    pub async fn start() -> Self {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let network = format!("sagat-lake-{suffix}");
        let store_name = format!("rustfs-{suffix}");

        let store = GenericImage::new(RUSTFS.0, RUSTFS.1)
            .with_exposed_port(9000.tcp())
            .with_env_var("RUSTFS_ACCESS_KEY", ACCESS_KEY)
            .with_env_var("RUSTFS_SECRET_KEY", SECRET_KEY)
            .with_env_var("RUSTFS_ADDRESS", ":9000")
            .with_env_var("RUSTFS_CONSOLE_ENABLE", "false")
            .with_network(&network)
            .with_container_name(&store_name);
        let store = within(
            "pull and start the RustFS fixture",
            START_LIMIT,
            store.start(),
        )
        .await
        .unwrap_or_else(|error| panic!("start RustFS: {error}"));
        let s3_port = store
            .get_host_port_ipv4(9000.tcp())
            .await
            .expect("RustFS port");
        let s3_endpoint = format!("http://127.0.0.1:{s3_port}");
        wait_ready(&format!("{s3_endpoint}/health"), "RustFS").await;

        // The catalog writes table metadata to the bucket, so the bucket exists before it starts.
        LakeAdmin::new(&s3_endpoint, "http://127.0.0.1:0")
            .create_bucket(BUCKET)
            .await;

        let catalog = GenericImage::new(ICEBERG_REST.0, ICEBERG_REST.1)
            .with_exposed_port(8181.tcp())
            .with_env_var("CATALOG_WAREHOUSE", format!("s3://{BUCKET}/"))
            .with_env_var("CATALOG_IO__IMPL", "org.apache.iceberg.aws.s3.S3FileIO")
            .with_env_var("CATALOG_S3_ENDPOINT", format!("http://{store_name}:9000"))
            .with_env_var("CATALOG_S3_PATH__STYLE__ACCESS", "true")
            .with_env_var("AWS_REGION", REGION)
            .with_env_var("AWS_ACCESS_KEY_ID", ACCESS_KEY)
            .with_env_var("AWS_SECRET_ACCESS_KEY", SECRET_KEY)
            .with_network(&network);
        let catalog = within(
            "pull and start the Iceberg REST fixture",
            START_LIMIT,
            catalog.start(),
        )
        .await
        .unwrap_or_else(|error| panic!("start the Iceberg REST fixture: {error}"));
        let catalog_port = catalog
            .get_host_port_ipv4(8181.tcp())
            .await
            .expect("REST catalog port");
        wait_ready(
            &format!("http://127.0.0.1:{catalog_port}/v1/config"),
            "the Iceberg REST fixture",
        )
        .await;

        Self {
            _store: store,
            _catalog: catalog,
            s3_endpoint,
            catalog_uri: format!("http://127.0.0.1:{catalog_port}"),
        }
    }

    pub fn admin(&self) -> LakeAdmin {
        LakeAdmin::new(&self.s3_endpoint, &self.catalog_uri)
    }
}

/// The fixture admin client: creates what the platform must never create.
pub struct LakeAdmin {
    http: reqwest::Client,
    s3_endpoint: String,
    catalog_uri: String,
}

/// A column of a fixture table (Iceberg primitive types only).
pub struct Column {
    pub id: u32,
    pub name: &'static str,
    pub kind: &'static str,
    pub required: bool,
}

impl LakeAdmin {
    pub fn new(s3_endpoint: &str, catalog_uri: &str) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("http client"),
            s3_endpoint: s3_endpoint.trim_end_matches('/').to_owned(),
            catalog_uri: catalog_uri.trim_end_matches('/').to_owned(),
        }
    }

    /// `PUT /<bucket>` with a signed request. An existing bucket owned by the caller is not an error.
    ///
    /// RustFS answers `/health` before its storage layer is ready and refuses the bucket with
    /// `503 Service not ready: waiting for storage_quorum` until it is. That refusal, and only that one, is
    /// waited out; the stage ends in a panic naming it if the store never becomes ready.
    pub async fn create_bucket(&self, bucket: &str) {
        within(
            format!("the object store accepts the bucket {bucket}"),
            START_LIMIT,
            async {
                loop {
                    let (status, body) = self.put_bucket(bucket).await;
                    if status.is_success() || body.contains("BucketAlreadyOwnedByYou") {
                        return;
                    }
                    assert!(
                        status == reqwest::StatusCode::SERVICE_UNAVAILABLE,
                        "create bucket {bucket}: {status} {body}"
                    );
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            },
        )
        .await;
    }

    async fn put_bucket(&self, bucket: &str) -> (reqwest::StatusCode, String) {
        let host = self
            .s3_endpoint
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        let now = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let authorization = sigv4_authorization(&Signing {
            method: "PUT",
            host,
            path: &format!("/{bucket}"),
            payload_sha256: EMPTY_PAYLOAD_SHA256,
            amz_date: &now,
            access_key: ACCESS_KEY,
            secret_key: SECRET_KEY,
            region: REGION,
            service: "s3",
        });
        let response = self
            .http
            .put(format!("{}/{bucket}", self.s3_endpoint))
            .header("x-amz-date", &now)
            .header("x-amz-content-sha256", EMPTY_PAYLOAD_SHA256)
            .header("authorization", authorization)
            .send()
            .await
            .expect("create bucket request");
        let status = response.status();
        (status, response.text().await.unwrap_or_default())
    }

    /// Creates a namespace in the REST catalog.
    pub async fn create_namespace(&self, namespace: &str) {
        let response = self
            .http
            .post(format!("{}/v1/namespaces", self.catalog_uri))
            .json(&json!({ "namespace": [namespace], "properties": {} }))
            .send()
            .await
            .expect("create namespace request");
        let status = response.status();
        assert!(
            status.is_success(),
            "create namespace {namespace}: {status} {}",
            response.text().await.unwrap_or_default()
        );
    }

    /// Creates an unpartitioned table and returns the catalog's answer (`metadata-location`, `metadata`).
    pub async fn create_table(&self, namespace: &str, table: &str, columns: &[Column]) -> Value {
        let fields: Vec<Value> = columns
            .iter()
            .map(|c| json!({ "id": c.id, "name": c.name, "required": c.required, "type": c.kind }))
            .collect();
        let response = self
            .http
            .post(format!(
                "{}/v1/namespaces/{namespace}/tables",
                self.catalog_uri
            ))
            .json(&json!({
                "name": table,
                "schema": { "type": "struct", "schema-id": 0, "fields": fields },
            }))
            .send()
            .await
            .expect("create table request");
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        assert!(
            status.is_success(),
            "create table {namespace}.{table}: {status} {text}"
        );
        serde_json::from_str(&text).expect("create table answer is JSON")
    }

    /// What the catalog answers for `GET /v1/namespaces/<namespace>/tables/<table>`, or `None` if it is missing.
    pub async fn load_table(&self, namespace: &str, table: &str) -> Option<Value> {
        let response = self
            .http
            .get(format!(
                "{}/v1/namespaces/{namespace}/tables/{table}",
                self.catalog_uri
            ))
            .send()
            .await
            .expect("load table request");
        match response.status().as_u16() {
            404 => None,
            200 => Some(response.json().await.expect("load table answer is JSON")),
            other => panic!(
                "load table {namespace}.{table}: {other} {}",
                response.text().await.unwrap_or_default()
            ),
        }
    }
}

// ---- S3 signature version 4 ------------------------------------------------------------------------------

/// SHA-256 of an empty payload, as S3 wants it in `x-amz-content-sha256`.
pub const EMPTY_PAYLOAD_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// What a request is signed over. Only `host`, `x-amz-content-sha256` and `x-amz-date` are signed.
pub struct Signing<'a> {
    pub method: &'a str,
    pub host: &'a str,
    pub path: &'a str,
    pub payload_sha256: &'a str,
    pub amz_date: &'a str,
    pub access_key: &'a str,
    pub secret_key: &'a str,
    pub region: &'a str,
    pub service: &'a str,
}

/// The `Authorization` header value for a request with no query string and the three signed headers above.
pub fn sigv4_authorization(s: &Signing<'_>) -> String {
    let date = &s.amz_date[..8];
    let scope = format!("{date}/{}/{}/aws4_request", s.region, s.service);
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";
    let canonical = format!(
        "{}\n{}\n\nhost:{}\nx-amz-content-sha256:{}\nx-amz-date:{}\n\n{signed_headers}\n{}",
        s.method, s.path, s.host, s.payload_sha256, s.amz_date, s.payload_sha256
    );
    signature_header(s, date, &scope, signed_headers, &canonical)
}

/// Same, for a request that signs only `host` and `x-amz-date` (the form of the AWS documentation's examples).
pub fn sigv4_authorization_host_date(s: &Signing<'_>) -> String {
    let date = &s.amz_date[..8];
    let scope = format!("{date}/{}/{}/aws4_request", s.region, s.service);
    let signed_headers = "host;x-amz-date";
    let canonical = format!(
        "{}\n{}\n\nhost:{}\nx-amz-date:{}\n\n{signed_headers}\n{}",
        s.method, s.path, s.host, s.amz_date, s.payload_sha256
    );
    signature_header(s, date, &scope, signed_headers, &canonical)
}

fn signature_header(
    s: &Signing<'_>,
    date: &str,
    scope: &str,
    signed_headers: &str,
    canonical: &str,
) -> String {
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        s.amz_date,
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let k_date = hmac_sha256(format!("AWS4{}", s.secret_key).as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, s.region.as_bytes());
    let k_service = hmac_sha256(&k_region, s.service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex(&hmac_sha256(&k_signing, to_sign.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        s.access_key
    )
}

/// HMAC-SHA256 (RFC 2104) over `sha2`, so this module needs no further dependency.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > block.len() {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|byte| byte ^ 0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(block.map(|byte| byte ^ 0x5c));
    outer.update(inner.finalize());
    let mut out = [0u8; 32];
    out.copy_from_slice(&outer.finalize());
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
