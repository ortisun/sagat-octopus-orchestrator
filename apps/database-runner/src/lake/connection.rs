//! The Iceberg connection of a lake attempt, as the runner reads it from `ResolveDatabase` (feature 035, T035-035,
//! T035-044, D035-115; `specs/001-platform-foundation/contracts/worker.proto`, `contracts/worker-protocol.md`).
//!
//! The server answers with the typed message `IcebergConnection`: the pinned non-secret settings of one revision and
//! `credentials`, a map of one entry per secret of the connection (`catalog`, `storage`), each the JSON document of the
//! data model (`{"clientId","clientSecret"}`, `{"token"}`, `{"accessKeyId","secretAccessKey","sessionToken"?}`). The host
//! reads the documents as the platform *stores* them (`crates/infra-postgres/src/database/iceberg.rs`, `to_json`), so this is
//! the place that turns the one into the other, and it is checked against the wire fixture that the Rust builder and the
//! Python host both read (`tests/contract/iceberg-writes/fixtures/requests/iceberg-write-append.json`).
//!
//! **What it does not do.** It converts a REST catalog over S3, the only kind the host opens (G035-048); any other kind is
//! `Unsupported`, with a name, before a container is started. It does not decide whether a kind is *allowed* (the allowed
//! modes and the egress ceiling are carried through for the driver) and it does not check the shape of the secrets beyond
//! "a JSON object of text to text": the host and the relay's policy check what they use.
//!
//! **Secrets.** The bytes of the map are zeroised as soon as they are parsed, and the parsed values are held in
//! [`Zeroizing`] strings, so the runner keeps no copy that outlives the attempt but the ones it hands on (and those are the
//! next slice's: `infra-lake` holds them as plain strings until its manifest takes `zeroize`). A [`Converted`] shows none of
//! them, and an error repeats nothing of the message.

use std::collections::BTreeMap;

use sagat_api_grpc::pb::{self, iceberg_connection::Catalog, lake_storage::Kind};
use serde_json::{Map, Value, json};
use zeroize::{Zeroize as _, Zeroizing};

/// The longest secret document the runner reads (the host reads the same bound, `_MAX_SECRET` of `connection.py`, per value).
const MAX_SECRET_BYTES: usize = 16 * 1024;

/// A secret document: text to text, every value zeroised on drop.
pub(crate) type Secret = BTreeMap<String, Zeroizing<String>>;

/// Why a connection cannot be read. A closed kind: it names the part, never a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the connection is not complete or not well formed")]
    Shape,
    #[error("the lake runner cannot open this kind of connection yet")]
    Unsupported,
    #[error("a secret of the connection is not a document the runner reads")]
    Secret,
}

/// The ceilings of one write, as the connection carries them (`0` is "not set": the next ceiling up applies).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    pub max_record_bytes: Option<u32>,
    pub max_records: Option<u32>,
    pub timeout_ms: Option<u32>,
}

/// An Iceberg connection in the form the host and the relay's policy read.
pub(crate) struct Converted {
    /// The catalog document as the platform stores it (`{"type":"rest","rest":{...}}`).
    pub catalog: Value,
    /// The storage document as the platform stores it (`{"type":"s3","s3":{...},"credentials":"static"|"vended"}`).
    pub storage: Value,
    pub warehouse_locations: Vec<String>,
    pub catalog_secret: Secret,
    pub storage_secret: Secret,
    pub allowed_modes: Vec<String>,
    pub limits: Limits,
}

/// No secret, and nothing the document says: the shape of the thing.
impl std::fmt::Debug for Converted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Converted")
            .field("warehouse_locations", &self.warehouse_locations.len())
            .field("allowed_modes", &self.allowed_modes)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// A text member that is set: the protobuf has no "unset" for a string, so empty is.
fn set(text: &str) -> Option<&str> {
    (!text.is_empty()).then_some(text)
}

fn required(text: &str) -> Result<&str, ConvertError> {
    set(text).ok_or(ConvertError::Shape)
}

fn rest(catalog: &pb::IcebergRestCatalog) -> Result<Value, ConvertError> {
    let mut members = Map::new();
    members.insert("uri".to_owned(), json!(required(&catalog.uri)?));
    members.insert("warehouse".to_owned(), json!(required(&catalog.warehouse)?));
    if let Some(prefix) = set(&catalog.prefix) {
        members.insert("prefix".to_owned(), json!(prefix));
    }
    let auth = match pb::LakeRestAuth::try_from(catalog.auth).map_err(|_| ConvertError::Shape)? {
        pb::LakeRestAuth::Oauth2 => {
            let mut auth = Map::new();
            auth.insert("type".to_owned(), json!("oauth2"));
            auth.insert("tokenUri".to_owned(), json!(required(&catalog.token_uri)?));
            if let Some(scope) = set(&catalog.scope) {
                auth.insert("scope".to_owned(), json!(scope));
            }
            Value::Object(auth)
        }
        pb::LakeRestAuth::Bearer => json!({"type": "bearer"}),
        pb::LakeRestAuth::Sigv4 => json!({
            "type": "sigv4",
            "region": required(&catalog.region)?,
            "signingName": required(&catalog.signing_name)?,
        }),
        pb::LakeRestAuth::Unspecified => return Err(ConvertError::Shape),
    };
    members.insert("auth".to_owned(), auth);
    members.insert(
        "accessDelegation".to_owned(),
        json!(if catalog.vended_credentials {
            "vended-credentials"
        } else {
            "none"
        }),
    );
    members.insert("remoteSigning".to_owned(), json!(catalog.remote_signing));
    Ok(json!({"type": "rest", "rest": members}))
}

fn storage(storage: &pb::LakeStorage) -> Result<Value, ConvertError> {
    let s3 = match storage.kind.as_ref().ok_or(ConvertError::Shape)? {
        Kind::S3(s3) => s3,
        Kind::Gcs(_) | Kind::Adls(_) => return Err(ConvertError::Unsupported),
    };
    let mut members = Map::new();
    members.insert("region".to_owned(), json!(required(&s3.region)?));
    if let Some(endpoint) = set(&s3.endpoint) {
        members.insert("endpoint".to_owned(), json!(endpoint));
    }
    members.insert("pathStyle".to_owned(), json!(s3.path_style));
    if let Some(role) = set(&s3.role_arn) {
        members.insert("roleArn".to_owned(), json!(role));
    }
    if let Some(external) = set(&s3.external_id) {
        members.insert("externalId".to_owned(), json!(external));
    }
    Ok(json!({
        "type": "s3",
        "s3": members,
        "credentials": if storage.vended_credentials { "vended" } else { "static" },
    }))
}

/// One secret document: a JSON object of text to text, at most 16 KiB. The bytes are zeroised whatever the outcome.
fn secret(bytes: &mut Vec<u8>) -> Result<Secret, ConvertError> {
    let parsed = if bytes.len() > MAX_SECRET_BYTES {
        Err(ConvertError::Secret)
    } else {
        serde_json::from_slice::<Map<String, Value>>(bytes).map_err(|_| ConvertError::Secret)
    };
    bytes.zeroize();
    let mut document = Secret::new();
    for (name, value) in parsed? {
        let Value::String(text) = value else {
            return Err(ConvertError::Secret);
        };
        document.insert(name, Zeroizing::new(text));
    }
    Ok(document)
}

fn limit(value: u32) -> Option<u32> {
    (value != 0).then_some(value)
}

/// Reads the connection of a lake attempt. The secret bytes of `connection` are zeroised in place, also when this fails.
///
/// # Errors
/// [`ConvertError`] for a connection that is not complete, a kind of catalog or storage the lake runner cannot open yet, and
/// a secret that is not a document of text to text.
pub(crate) fn convert(connection: &mut pb::IcebergConnection) -> Result<Converted, ConvertError> {
    // the secrets first, so that their bytes are gone whatever else is wrong with the connection
    let mut secrets: BTreeMap<String, Result<Secret, ConvertError>> = BTreeMap::new();
    for (name, bytes) in &mut connection.credentials {
        secrets.insert(name.clone(), secret(bytes));
    }
    let catalog = match connection.catalog.as_ref().ok_or(ConvertError::Shape)? {
        Catalog::Rest(catalog) => rest(catalog)?,
        Catalog::Glue(_) | Catalog::Hive(_) | Catalog::Jdbc(_) => {
            return Err(ConvertError::Unsupported);
        }
    };
    let storage = storage(connection.storage.as_ref().ok_or(ConvertError::Shape)?)?;
    let mut take = |name: &str| secrets.remove(name).unwrap_or_else(|| Ok(Secret::new()));
    let catalog_secret = take("catalog")?;
    let storage_secret = take("storage")?;
    if !secrets.is_empty() {
        // a secret the connection does not have would sit in the runner for nothing
        return Err(ConvertError::Secret);
    }
    let limits = connection.limits.as_ref();
    Ok(Converted {
        catalog,
        storage,
        warehouse_locations: connection.warehouse_locations.clone(),
        catalog_secret,
        storage_secret,
        allowed_modes: connection.allowed_modes.clone(),
        limits: Limits {
            max_record_bytes: limits.and_then(|limits| limit(limits.max_record_bytes)),
            max_records: limits.and_then(|limits| limit(limits.max_records)),
            timeout_ms: limits.and_then(|limits| limit(limits.timeout_ms)),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    const CATALOG_SECRET: &str = "client-secret-canary";
    const STORAGE_SECRET: &str = "storage-secret-canary";

    fn rest_catalog() -> pb::IcebergRestCatalog {
        pb::IcebergRestCatalog {
            uri: "https://catalog.example/api".into(),
            warehouse: "s3://lake/warehouse/".into(),
            prefix: "tenant-a".into(),
            auth: pb::LakeRestAuth::Oauth2 as i32,
            token_uri: "https://auth.example/token".into(),
            scope: "catalog".into(),
            ..pb::IcebergRestCatalog::default()
        }
    }

    fn s3() -> pb::LakeStorage {
        pb::LakeStorage {
            kind: Some(Kind::S3(pb::LakeS3Storage {
                region: "eu-west-1".into(),
                ..pb::LakeS3Storage::default()
            })),
            vended_credentials: false,
        }
    }

    fn secrets() -> HashMap<String, Vec<u8>> {
        HashMap::from([
            (
                "catalog".to_owned(),
                format!(r#"{{"clientId":"client-1","clientSecret":"{CATALOG_SECRET}"}}"#)
                    .into_bytes(),
            ),
            (
                "storage".to_owned(),
                format!(r#"{{"accessKeyId":"AKIAEXAMPLE","secretAccessKey":"{STORAGE_SECRET}"}}"#)
                    .into_bytes(),
            ),
        ])
    }

    fn connection() -> pb::IcebergConnection {
        pb::IcebergConnection {
            catalog: Some(Catalog::Rest(rest_catalog())),
            storage: Some(s3()),
            warehouse_locations: vec!["s3://lake/warehouse/".into()],
            allowed_modes: vec!["append".into()],
            credentials: secrets(),
            ..pb::IcebergConnection::default()
        }
    }

    fn fixture() -> Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../tests/contract/iceberg-writes/fixtures/requests/iceberg-write-append.json",
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_default()).unwrap_or_default()
    }

    fn plain(secret: &Secret) -> BTreeMap<String, String> {
        secret
            .iter()
            .map(|(name, value)| (name.clone(), value.to_string()))
            .collect()
    }

    /// The message the server sends becomes exactly the documents the Rust builder and the Python host read: the three
    /// representations (the stored JSON of the domain, the protobuf, the request line) agree on this connection.
    #[test]
    fn the_message_becomes_the_documents_of_the_wire_fixture() {
        let converted = convert(&mut connection()).unwrap();
        let wire = fixture();
        assert_eq!(converted.catalog, wire["connection"]["catalog"]);
        assert_eq!(converted.storage, wire["connection"]["storage"]);
        assert_eq!(converted.warehouse_locations, vec!["s3://lake/warehouse/"]);
        let credentials = &wire["connection"]["credentials"];
        let expected = |name: &str| -> BTreeMap<String, String> {
            serde_json::from_value(credentials[name].clone()).unwrap_or_default()
        };
        assert_eq!(plain(&converted.catalog_secret), expected("catalog"));
        assert_eq!(plain(&converted.storage_secret), expected("storage"));
    }

    #[test]
    fn the_modes_and_the_limits_are_carried_and_zero_is_not_set() {
        let mut message = connection();
        message.limits = Some(pb::LakeLimits {
            max_record_bytes: 4096,
            max_records: 0,
            timeout_ms: 60_000,
        });
        let converted = convert(&mut message).unwrap();
        assert_eq!(converted.allowed_modes, vec!["append"]);
        assert_eq!(
            converted.limits,
            Limits {
                max_record_bytes: Some(4096),
                max_records: None,
                timeout_ms: Some(60_000)
            }
        );
        let converted = convert(&mut connection()).unwrap();
        assert_eq!(
            converted.limits,
            Limits {
                max_record_bytes: None,
                max_records: None,
                timeout_ms: None
            }
        );
    }

    #[test]
    fn a_bearer_catalog_and_vended_storage_are_the_documents_the_domain_stores() {
        let mut message = connection();
        message.catalog = Some(Catalog::Rest(pb::IcebergRestCatalog {
            auth: pb::LakeRestAuth::Bearer as i32,
            token_uri: String::new(),
            scope: String::new(),
            prefix: String::new(),
            vended_credentials: true,
            ..rest_catalog()
        }));
        message.storage = Some(pb::LakeStorage {
            vended_credentials: true,
            ..s3()
        });
        message.credentials = HashMap::from([("catalog".to_owned(), br#"{"token":"t"}"#.to_vec())]);
        let converted = convert(&mut message).unwrap();
        assert_eq!(
            converted.catalog,
            json!({"type": "rest", "rest": {
                "uri": "https://catalog.example/api", "warehouse": "s3://lake/warehouse/",
                "auth": {"type": "bearer"}, "accessDelegation": "vended-credentials", "remoteSigning": false
            }})
        );
        assert_eq!(
            converted.storage,
            json!({"type": "s3", "s3": {"region": "eu-west-1", "pathStyle": false}, "credentials": "vended"})
        );
        assert!(converted.storage_secret.is_empty());
    }

    #[test]
    fn a_compatible_store_keeps_its_endpoint_its_style_and_its_role() {
        let mut message = connection();
        message.storage = Some(pb::LakeStorage {
            kind: Some(Kind::S3(pb::LakeS3Storage {
                region: "us-east-1".into(),
                endpoint: "https://minio.internal:9000".into(),
                path_style: true,
                role_arn: "arn:aws:iam::123456789012:role/lake".into(),
                external_id: "ext".into(),
            })),
            vended_credentials: false,
        });
        let converted = convert(&mut message).unwrap();
        assert_eq!(
            converted.storage["s3"],
            json!({"region": "us-east-1", "endpoint": "https://minio.internal:9000", "pathStyle": true,
                   "roleArn": "arn:aws:iam::123456789012:role/lake", "externalId": "ext"})
        );
    }

    #[test]
    fn sigv4_is_converted_and_left_to_the_policy_to_refuse() {
        let mut message = connection();
        message.catalog = Some(Catalog::Rest(pb::IcebergRestCatalog {
            auth: pb::LakeRestAuth::Sigv4 as i32,
            region: "eu-west-1".into(),
            signing_name: "execute-api".into(),
            ..rest_catalog()
        }));
        let converted = convert(&mut message).unwrap();
        assert_eq!(
            converted.catalog["rest"]["auth"],
            json!({"type": "sigv4", "region": "eu-west-1", "signingName": "execute-api"})
        );
    }

    #[test]
    fn a_connection_that_is_not_complete_is_refused() {
        let broken: Vec<pb::IcebergConnection> = vec![
            pb::IcebergConnection {
                catalog: None,
                ..connection()
            },
            pb::IcebergConnection {
                storage: None,
                ..connection()
            },
            pb::IcebergConnection {
                storage: Some(pb::LakeStorage {
                    kind: None,
                    vended_credentials: false,
                }),
                ..connection()
            },
            pb::IcebergConnection {
                catalog: Some(Catalog::Rest(pb::IcebergRestCatalog {
                    uri: String::new(),
                    ..rest_catalog()
                })),
                ..connection()
            },
            pb::IcebergConnection {
                catalog: Some(Catalog::Rest(pb::IcebergRestCatalog {
                    warehouse: String::new(),
                    ..rest_catalog()
                })),
                ..connection()
            },
            pb::IcebergConnection {
                catalog: Some(Catalog::Rest(pb::IcebergRestCatalog {
                    auth: pb::LakeRestAuth::Unspecified as i32,
                    ..rest_catalog()
                })),
                ..connection()
            },
            pb::IcebergConnection {
                catalog: Some(Catalog::Rest(pb::IcebergRestCatalog {
                    auth: 99,
                    ..rest_catalog()
                })),
                ..connection()
            },
            pb::IcebergConnection {
                catalog: Some(Catalog::Rest(pb::IcebergRestCatalog {
                    token_uri: String::new(),
                    ..rest_catalog()
                })),
                ..connection()
            },
            pb::IcebergConnection {
                storage: Some(pb::LakeStorage {
                    kind: Some(Kind::S3(pb::LakeS3Storage::default())),
                    vended_credentials: false,
                }),
                ..connection()
            },
        ];
        for mut message in broken {
            assert_eq!(convert(&mut message).unwrap_err(), ConvertError::Shape);
        }
    }

    #[test]
    fn a_kind_the_lake_runner_cannot_open_is_refused_with_a_name() {
        let kinds = [
            Catalog::Glue(pb::IcebergGlueCatalog::default()),
            Catalog::Hive(pb::IcebergHiveCatalog::default()),
            Catalog::Jdbc(pb::IcebergJdbcCatalog::default()),
        ];
        for catalog in kinds {
            let mut message = pb::IcebergConnection {
                catalog: Some(catalog),
                ..connection()
            };
            assert_eq!(
                convert(&mut message).unwrap_err(),
                ConvertError::Unsupported
            );
        }
        for kind in [
            Kind::Gcs(pb::LakeGcsStorage::default()),
            Kind::Adls(pb::LakeAdlsStorage::default()),
        ] {
            let mut message = pb::IcebergConnection {
                storage: Some(pb::LakeStorage {
                    kind: Some(kind),
                    vended_credentials: false,
                }),
                ..connection()
            };
            assert_eq!(
                convert(&mut message).unwrap_err(),
                ConvertError::Unsupported
            );
        }
    }

    #[test]
    fn a_secret_that_is_not_a_document_of_text_to_text_is_refused() {
        let oversized = format!(r#"{{"k":"{}"}}"#, "x".repeat(MAX_SECRET_BYTES));
        for bad in [
            b"not json".to_vec(),
            b"[]".to_vec(),
            b"\"text\"".to_vec(),
            br#"{"k":1}"#.to_vec(),
            br#"{"k":null}"#.to_vec(),
            br#"{"k":{"nested":"x"}}"#.to_vec(),
            vec![0xff, 0xfe],
            oversized.into_bytes(),
        ] {
            let mut message = connection();
            message.credentials.insert("catalog".to_owned(), bad);
            assert_eq!(convert(&mut message).unwrap_err(), ConvertError::Secret);
        }
    }

    #[test]
    fn a_secret_the_connection_does_not_have_is_refused() {
        let mut message = connection();
        message
            .credentials
            .insert("extra".to_owned(), br#"{"k":"v"}"#.to_vec());
        assert_eq!(convert(&mut message).unwrap_err(), ConvertError::Secret);
    }

    #[test]
    fn a_connection_with_no_secret_has_empty_documents() {
        let mut message = connection();
        message.credentials.clear();
        let converted = convert(&mut message).unwrap();
        assert!(converted.catalog_secret.is_empty() && converted.storage_secret.is_empty());
    }

    /// The runner keeps no copy of the secret bytes: they are zeroised as they are parsed, also when the connection is
    /// refused for another reason and also when a secret is refused.
    #[test]
    fn the_secret_bytes_of_the_message_are_gone_after_the_conversion_whatever_its_outcome() {
        let mut ok = connection();
        convert(&mut ok).unwrap();
        assert!(ok.credentials.values().all(Vec::is_empty));

        let mut incomplete = pb::IcebergConnection {
            catalog: None,
            ..connection()
        };
        assert!(convert(&mut incomplete).is_err());
        assert!(incomplete.credentials.values().all(Vec::is_empty));

        let mut unreadable = connection();
        unreadable
            .credentials
            .insert("catalog".to_owned(), b"not json".to_vec());
        assert!(convert(&mut unreadable).is_err());
        assert!(unreadable.credentials.values().all(Vec::is_empty));
    }

    #[test]
    fn nothing_shown_by_debug_or_an_error_holds_a_secret_or_an_address() {
        let converted = convert(&mut connection()).unwrap();
        let shown = format!("{converted:?}");
        for secret in [
            CATALOG_SECRET,
            STORAGE_SECRET,
            "AKIAEXAMPLE",
            "catalog.example",
        ] {
            assert!(!shown.contains(secret), "{shown}");
        }
        let mut message = connection();
        message.credentials.insert(
            "catalog".to_owned(),
            format!("{{\"k\":1,\"s\":\"{CATALOG_SECRET}\"}}").into_bytes(),
        );
        let error = convert(&mut message).unwrap_err();
        assert!(!format!("{error:?} {error}").contains(CATALOG_SECRET));
    }
}
