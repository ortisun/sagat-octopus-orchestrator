//! What the tests of the lake modules share: a connection as the server sends it, the settings of a runner on a temporary
//! work root, and the document the attempt-input corpus is built on.
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sagat_api_grpc::pb::{self, iceberg_connection::Catalog, lake_storage::Kind};
use serde_json::Value;

use super::connection::{Converted, convert};
use super::settings::Settings;

/// The catalog's secret, as it is in the message: the tests look for it where it must be and where it must not.
pub const SECRET: &str = "client-secret-canary";

pub fn settings(work_root: &Path) -> Settings {
    Settings {
        work_root: work_root.to_path_buf(),
        sandbox_socket: "/run/sagat/lake-sandbox.sock".into(),
        sandbox_uid: 10_002,
        slots: 8,
        relay_allow: vec!["0.0.0.0/0".parse().unwrap()],
        relay_deny: vec!["10.0.0.0/8".parse().unwrap()],
        max_bytes: 1 << 20,
        max_records: 1000,
        max_commit_retries: 4,
        max_decoded_bytes: 17_179_869_184,
        max_metadata_bytes: 67_108_864,
        host_start: Duration::from_secs(30),
        kill_limit: Duration::from_secs(20),
        large_above_bytes: None,
    }
}

pub fn message() -> pb::IcebergConnection {
    pb::IcebergConnection {
        catalog: Some(Catalog::Rest(pb::IcebergRestCatalog {
            uri: "https://catalog.example/api".into(),
            warehouse: "s3://lake/warehouse/".into(),
            prefix: "tenant-a".into(),
            auth: pb::LakeRestAuth::Oauth2 as i32,
            token_uri: "https://auth.example/token".into(),
            scope: "catalog".into(),
            ..pb::IcebergRestCatalog::default()
        })),
        storage: Some(pb::LakeStorage {
            kind: Some(Kind::S3(pb::LakeS3Storage {
                region: "eu-west-1".into(),
                ..pb::LakeS3Storage::default()
            })),
            vended_credentials: false,
        }),
        warehouse_locations: vec!["s3://lake/warehouse/".into()],
        allowed_modes: vec!["append".into()],
        egress_ceiling: pb::EgressCeiling::Internal as i32,
        credentials: HashMap::from([
            (
                "catalog".to_owned(),
                format!(r#"{{"clientId":"client-1","clientSecret":"{SECRET}"}}"#).into_bytes(),
            ),
            (
                "storage".to_owned(),
                br#"{"accessKeyId":"AKIAEXAMPLE","secretAccessKey":"storage-secret-canary"}"#
                    .to_vec(),
            ),
        ]),
        ..pb::IcebergConnection::default()
    }
}

/// What `ResolveDatabase` answers for a lake step of the namespace `namespace_id`.
pub fn resolved_database(namespace_id: &str) -> pb::ResolvedDatabase {
    pb::ResolvedDatabase {
        tenant_id: uuid::Uuid::from_u128(1).to_string(),
        namespace_id: namespace_id.into(),
        connection_name: "lake-main".into(),
        connection_id: uuid::Uuid::from_u128(7).to_string(),
        connection_revision: 3,
        iceberg: Some(message()),
        ..pb::ResolvedDatabase::default()
    }
}

pub fn converted() -> Converted {
    convert(&mut message()).unwrap()
}

/// The document that `tests/contract/iceberg-writes/fixtures/attempt-input/cases.json` is built on.
pub fn base() -> Value {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/iceberg-writes/fixtures/attempt-input/cases.json");
    let corpus: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    corpus["base"].clone()
}

/// The text of the base document after `edit`.
pub fn input_json(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut document = base();
    edit(&mut document);
    serde_json::to_vec(&document).unwrap()
}

/// The lease of a probe as the server writes it (`fixtures/probe-input/probe.json`) after `edit`.
pub fn probe_input_json(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/iceberg-writes/fixtures/probe-input/probe.json");
    let mut document: Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    edit(&mut document);
    serde_json::to_vec(&document).unwrap()
}

/// The user this test runs as: a scripted host in the test's own process is reported by the kernel as that peer.
pub fn my_uid() -> u32 {
    use std::os::unix::fs::MetadataExt as _;
    tempfile::tempfile().unwrap().metadata().unwrap().uid()
}
