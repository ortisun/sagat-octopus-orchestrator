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
//! **What it refuses, before a container is started.** Everything the host cannot open: a catalog that is not REST, storage
//! that is not S3, SigV4, remote signing, an assumed role, a catalog that delegates credentials to storage that does not use
//! them (or the other way round), and a `storage` secret for storage whose credentials the catalog vends (G035-048,
//! `connection.py`). It does not decide whether a *mode* is allowed: the allowed modes, the limits and the egress ceiling are
//! carried through for the driver, and an egress ceiling that is not set is a protocol anomaly and is refused, not read as the
//! domain's default for a stored revision that predates it.
//!
//! **Bounded before cloned.** At most two credential entries, sixteen warehouse locations of at most 2048 bytes, and a secret
//! document of at most 16 KiB; anything over is refused before it is copied or parsed.
//!
//! **Secrets.** The bytes of the map are zeroised in place as soon as they are read, whatever the outcome, and the values are
//! parsed straight into [`Zeroizing`] strings (a name twice, or a value that is not text, fails at parse time and leaves no
//! plain copy of a value in a `serde_json::Value`). **What this does not reach:** `serde_json` copies a string that contains
//! an escape into a scratch buffer of its own, and the message of a type error repeats the value it met (`invalid type: string
//! "S", expected a map`); both are dropped unscrubbed. A secret that contains `"`, `\` or a control character, and a document
//! of the wrong shape, therefore leave a short-lived copy in freed heap memory that this module cannot zero (a hand-written
//! parser or a scrubbing allocator could; neither is in this slice). `infra-lake` takes the secrets as `String`s and moves
//! them into `Zeroizing` ones, scrubs the body it builds from them, and returns the request line as a `Zeroizing` buffer
//! (G035-063, D035-122).

use std::collections::BTreeMap;
use std::fmt;

use sagat_api_grpc::pb::{self, iceberg_connection::Catalog, lake_storage::Kind};
use sagat_domain::classification::EgressCeiling;
use serde::de::{self, Deserialize, Deserializer, MapAccess, Visitor};
use serde_json::{Map, Value, json};
use zeroize::{Zeroize as _, Zeroizing};

/// The longest secret document the runner reads (the host reads the same bound, `_MAX_SECRET` of `connection.py`, per value).
const MAX_SECRET_BYTES: usize = 16 * 1024;
/// A connection has at most two secrets: the catalog's and the storage's.
const MAX_SECRETS: usize = 2;
/// The host and the policy read at most sixteen warehouse locations of at most 2048 bytes.
const MAX_LOCATIONS: usize = 16;
const MAX_LOCATION_BYTES: usize = 2048;

/// A secret document: text to text, every value zeroised on drop. A newtype, not an alias of the map, so that a `{:?}` of
/// it (in a log line, an `assert!` message, a `tracing` field) shows how many names it has and nothing else: the
/// `Debug` of [`Zeroizing`] prints the value it holds.
#[derive(Default)]
pub struct Secret(BTreeMap<String, Zeroizing<String>>);

impl Secret {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(|value| value.as_str())
    }

    /// The names and values, in the order of the names.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Secret(<{} redacted>)", self.0.len())
    }
}

/// Why a connection cannot be read. A closed kind: it names the part, never a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConvertError {
    #[error("the connection is not complete or not well formed")]
    Shape,
    #[error("the lake runner cannot open this kind or combination of connection yet")]
    Unsupported,
    #[error("a secret of the connection is not a document the runner reads")]
    Secret,
}

/// The ceilings of one write, as the connection carries them (`0` is "not set": the next ceiling up applies).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_record_bytes: Option<u32>,
    pub max_records: Option<u32>,
    pub timeout_ms: Option<u32>,
}

/// An Iceberg connection in the form the host and the relay's policy read.
pub struct Converted {
    /// The catalog document as the platform stores it (`{"type":"rest","rest":{...}}`).
    pub catalog: Value,
    /// The storage document as the platform stores it (`{"type":"s3","s3":{...},"credentials":"static"|"vended"}`).
    pub storage: Value,
    pub warehouse_locations: Vec<String>,
    pub catalog_secret: Secret,
    pub storage_secret: Secret,
    pub allowed_modes: Vec<String>,
    /// The highest level of data the connection may receive.
    pub egress_ceiling: EgressCeiling,
    pub limits: Limits,
}

/// No secret, and nothing the document says: the shape of the thing.
impl fmt::Debug for Converted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Converted")
            .field("warehouse_locations", &self.warehouse_locations.len())
            .field("allowed_modes", &self.allowed_modes)
            .field("egress_ceiling", &self.egress_ceiling)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// A value of a secret document, parsed straight into a zeroising string: no plain copy of it is made here.
struct Text(Zeroizing<String>);

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Reads;
        impl Visitor<'_> for Reads {
            type Value = Text;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("text")
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<Text, E> {
                Ok(Text(Zeroizing::new(text.to_owned())))
            }

            fn visit_string<E: de::Error>(self, text: String) -> Result<Text, E> {
                Ok(Text(Zeroizing::new(text)))
            }
        }
        deserializer.deserialize_string(Reads)
    }
}

/// A secret document: an object of text to text, with no name twice.
struct Document(Secret);

impl<'de> Deserialize<'de> for Document {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Reads;
        impl<'de> Visitor<'de> for Reads {
            type Value = Document;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object of text to text")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Document, A::Error> {
                let mut document = BTreeMap::new();
                while let Some(name) = map.next_key::<String>()? {
                    let value: Text = map.next_value()?;
                    if document.insert(name, value.0).is_some() {
                        return Err(de::Error::custom("a name twice"));
                    }
                }
                Ok(Document(Secret(document)))
            }
        }
        deserializer.deserialize_map(Reads)
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
    // what the host does not open: remote signing, and SigV4 (an AWS-signed catalog)
    if catalog.remote_signing {
        return Err(ConvertError::Unsupported);
    }
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
        pb::LakeRestAuth::Sigv4 => return Err(ConvertError::Unsupported),
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
    members.insert("remoteSigning".to_owned(), json!(false));
    Ok(json!({"type": "rest", "rest": members}))
}

fn storage(storage: &pb::LakeStorage) -> Result<Value, ConvertError> {
    let s3 = match storage.kind.as_ref().ok_or(ConvertError::Shape)? {
        Kind::S3(s3) => s3,
        Kind::Gcs(_) | Kind::Adls(_) => return Err(ConvertError::Unsupported),
    };
    // an assumed role needs STS, which the host does not reach
    if set(&s3.role_arn).is_some() || set(&s3.external_id).is_some() {
        return Err(ConvertError::Unsupported);
    }
    let mut members = Map::new();
    members.insert("region".to_owned(), json!(required(&s3.region)?));
    if let Some(endpoint) = set(&s3.endpoint) {
        members.insert("endpoint".to_owned(), json!(endpoint));
    }
    members.insert("pathStyle".to_owned(), json!(s3.path_style));
    Ok(json!({
        "type": "s3",
        "s3": members,
        "credentials": if storage.vended_credentials { "vended" } else { "static" },
    }))
}

/// One secret document: a JSON object of text to text, at most 16 KiB, no name twice. The bytes are zeroised whatever the
/// outcome.
fn secret(bytes: &mut Vec<u8>) -> Result<Secret, ConvertError> {
    let parsed = if bytes.len() > MAX_SECRET_BYTES {
        Err(ConvertError::Secret)
    } else {
        serde_json::from_slice::<Document>(bytes)
            .map(|document| document.0)
            .map_err(|_| ConvertError::Secret)
    };
    bytes.zeroize();
    parsed
}

fn limit(value: u32) -> Option<u32> {
    (value != 0).then_some(value)
}

fn ceiling(value: i32) -> Result<EgressCeiling, ConvertError> {
    match pb::EgressCeiling::try_from(value).map_err(|_| ConvertError::Shape)? {
        pb::EgressCeiling::Public => Ok(EgressCeiling::Public),
        pb::EgressCeiling::Internal => Ok(EgressCeiling::Internal),
        pb::EgressCeiling::Pii => Ok(EgressCeiling::Pii),
        pb::EgressCeiling::Unspecified => Err(ConvertError::Shape),
    }
}

/// Zeroises every secret entry of the message, in place.
fn scrub(connection: &mut pb::IcebergConnection) {
    for bytes in connection.credentials.values_mut() {
        bytes.zeroize();
    }
}

/// Reads the connection of a lake attempt. The secret bytes of `connection` are zeroised in place, also when this fails.
///
/// # Errors
/// [`ConvertError`] for a connection that is not complete, a kind or combination the lake runner cannot open yet, a message
/// that is larger than a connection can be, and a secret that is not a document of text to text.
pub fn convert(connection: &mut pb::IcebergConnection) -> Result<Converted, ConvertError> {
    // bounded before anything is copied or parsed; the bytes of what is refused are gone all the same
    if connection.credentials.len() > MAX_SECRETS {
        scrub(connection);
        return Err(ConvertError::Secret);
    }
    // the secrets first, so that their bytes are gone whatever else is wrong with the connection
    let mut secrets: BTreeMap<String, Result<Secret, ConvertError>> = BTreeMap::new();
    for (name, bytes) in &mut connection.credentials {
        secrets.insert(name.clone(), secret(bytes));
    }
    if connection.warehouse_locations.len() > MAX_LOCATIONS
        || connection
            .warehouse_locations
            .iter()
            .any(|location| location.len() > MAX_LOCATION_BYTES)
    {
        return Err(ConvertError::Shape);
    }
    let catalog_message = match connection.catalog.as_ref().ok_or(ConvertError::Shape)? {
        Catalog::Rest(catalog) => catalog,
        Catalog::Glue(_) | Catalog::Hive(_) | Catalog::Jdbc(_) => {
            return Err(ConvertError::Unsupported);
        }
    };
    let storage_message = connection.storage.as_ref().ok_or(ConvertError::Shape)?;
    let catalog = rest(catalog_message)?;
    let storage_document = storage(storage_message)?;
    // The catalog delegates the storage credentials exactly when the storage uses delegated ones (`connection.py`): a
    // pair that disagrees fails in the host after a container has been started.
    if catalog_message.vended_credentials != storage_message.vended_credentials {
        return Err(ConvertError::Unsupported);
    }
    let mut take = |name: &str| secrets.remove(name).unwrap_or_else(|| Ok(Secret::new()));
    let catalog_secret = take("catalog")?;
    let storage_secret = take("storage")?;
    if !secrets.is_empty() {
        // a secret the connection does not have would sit in the runner for nothing
        return Err(ConvertError::Secret);
    }
    // credentials the catalog vends do not travel: a storage secret sent anyway would sit in the sandbox for nothing
    if storage_message.vended_credentials && !storage_secret.is_empty() {
        return Err(ConvertError::Secret);
    }
    let limits = connection.limits.as_ref();
    Ok(Converted {
        catalog,
        storage: storage_document,
        warehouse_locations: connection.warehouse_locations.clone(),
        catalog_secret,
        storage_secret,
        allowed_modes: connection.allowed_modes.clone(),
        egress_ceiling: ceiling(connection.egress_ceiling)?,
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
            egress_ceiling: pb::EgressCeiling::Internal as i32,
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
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
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
    fn a_compatible_store_keeps_its_endpoint_and_its_style() {
        let mut message = connection();
        message.storage = Some(pb::LakeStorage {
            kind: Some(Kind::S3(pb::LakeS3Storage {
                region: "us-east-1".into(),
                endpoint: "https://minio.internal:9000".into(),
                path_style: true,
                ..pb::LakeS3Storage::default()
            })),
            vended_credentials: false,
        });
        let converted = convert(&mut message).unwrap();
        assert_eq!(
            converted.storage["s3"],
            json!({"region": "us-east-1", "endpoint": "https://minio.internal:9000", "pathStyle": true})
        );
    }

    /// What the host cannot open is refused here, with a name, before a container is started (`connection.py`).
    #[test]
    fn what_the_host_cannot_open_is_refused_before_a_container_is_started() {
        let sigv4 = pb::IcebergRestCatalog {
            auth: pb::LakeRestAuth::Sigv4 as i32,
            region: "eu-west-1".into(),
            signing_name: "execute-api".into(),
            ..rest_catalog()
        };
        let remote_signing = pb::IcebergRestCatalog {
            remote_signing: true,
            ..rest_catalog()
        };
        for catalog in [sigv4, remote_signing] {
            let mut message = pb::IcebergConnection {
                catalog: Some(Catalog::Rest(catalog)),
                ..connection()
            };
            assert_eq!(
                convert(&mut message).unwrap_err(),
                ConvertError::Unsupported
            );
        }
        for (role_arn, external_id) in [("arn:aws:iam::123456789012:role/lake", ""), ("", "ext")] {
            let mut message = connection();
            message.storage = Some(pb::LakeStorage {
                kind: Some(Kind::S3(pb::LakeS3Storage {
                    region: "us-east-1".into(),
                    role_arn: role_arn.into(),
                    external_id: external_id.into(),
                    ..pb::LakeS3Storage::default()
                })),
                vended_credentials: false,
            });
            assert_eq!(
                convert(&mut message).unwrap_err(),
                ConvertError::Unsupported
            );
        }
    }

    /// The catalog vends the storage credentials exactly when the storage uses vended ones; a pair that disagrees would
    /// fail in the host after a container has been started.
    #[test]
    fn a_catalog_and_a_storage_that_disagree_about_vended_credentials_are_refused() {
        let mut catalog_only = connection();
        catalog_only.catalog = Some(Catalog::Rest(pb::IcebergRestCatalog {
            vended_credentials: true,
            ..rest_catalog()
        }));
        let mut storage_only = connection();
        storage_only.storage = Some(pb::LakeStorage {
            vended_credentials: true,
            ..s3()
        });
        for mut message in [catalog_only, storage_only] {
            assert_eq!(
                convert(&mut message).unwrap_err(),
                ConvertError::Unsupported
            );
        }
    }

    /// Credentials the catalog vends do not travel: a storage secret sent anyway would sit in the sandbox for nothing.
    #[test]
    fn a_storage_secret_for_vended_storage_is_refused() {
        let mut message = connection();
        message.catalog = Some(Catalog::Rest(pb::IcebergRestCatalog {
            vended_credentials: true,
            ..rest_catalog()
        }));
        message.storage = Some(pb::LakeStorage {
            vended_credentials: true,
            ..s3()
        });
        // `secrets()` still carries the `storage` entry
        assert_eq!(convert(&mut message).unwrap_err(), ConvertError::Secret);
        assert!(message.credentials.values().all(Vec::is_empty));
    }

    #[test]
    fn the_egress_ceiling_is_carried_and_one_that_is_not_set_is_refused() {
        for (wire, expected) in [
            (pb::EgressCeiling::Public, EgressCeiling::Public),
            (pb::EgressCeiling::Internal, EgressCeiling::Internal),
            (pb::EgressCeiling::Pii, EgressCeiling::Pii),
        ] {
            let mut message = connection();
            message.egress_ceiling = wire as i32;
            assert_eq!(convert(&mut message).unwrap().egress_ceiling, expected);
        }
        // not set is a protocol anomaly, not the domain's default for a revision that predates the ceiling; a number the
        // runner does not know is the same
        for wire in [pb::EgressCeiling::Unspecified as i32, 99, -1] {
            let mut message = connection();
            message.egress_ceiling = wire;
            assert_eq!(convert(&mut message).unwrap_err(), ConvertError::Shape);
            assert!(message.credentials.values().all(Vec::is_empty));
        }
    }

    /// The host and the policy read at most sixteen locations of 2048 bytes; more is refused before it is cloned.
    #[test]
    fn the_warehouse_locations_are_bounded() {
        let mut at_the_bound = connection();
        at_the_bound.warehouse_locations = (0..MAX_LOCATIONS)
            .map(|index| format!("s3://lake/{index}/{}", "x".repeat(MAX_LOCATION_BYTES - 16)))
            .collect();
        assert!(
            at_the_bound
                .warehouse_locations
                .iter()
                .all(|location| location.len() <= MAX_LOCATION_BYTES)
        );
        assert_eq!(
            convert(&mut at_the_bound)
                .unwrap()
                .warehouse_locations
                .len(),
            MAX_LOCATIONS
        );

        let mut too_many = connection();
        too_many.warehouse_locations = vec!["s3://lake/w/".into(); MAX_LOCATIONS + 1];
        let mut too_long = connection();
        too_long.warehouse_locations = vec!["x".repeat(MAX_LOCATION_BYTES + 1)];
        for mut message in [too_many, too_long] {
            assert_eq!(convert(&mut message).unwrap_err(), ConvertError::Shape);
            assert!(message.credentials.values().all(Vec::is_empty));
        }
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

    /// A name twice is refused at parse time: `serde_json::Value` would keep the last one and drop the first without a trace,
    /// and a document that says two things is not one the platform stored.
    #[test]
    fn a_secret_that_names_a_value_twice_is_refused() {
        let mut message = connection();
        message.credentials.insert(
            "catalog".to_owned(),
            br#"{"clientId":"a","clientId":"b","clientSecret":"s"}"#.to_vec(),
        );
        assert_eq!(convert(&mut message).unwrap_err(), ConvertError::Secret);
        assert!(message.credentials.values().all(Vec::is_empty));
    }

    #[test]
    fn a_secret_the_connection_does_not_have_is_refused() {
        // two entries, so that the bound on the number of entries is not what refuses it: the name is
        for extra in ["extra", "Catalog", ""] {
            let mut message = connection();
            message.credentials.remove("storage");
            message
                .credentials
                .insert(extra.to_owned(), br#"{"k":"v"}"#.to_vec());
            assert_eq!(message.credentials.len(), 2);
            assert_eq!(convert(&mut message).unwrap_err(), ConvertError::Secret);
            assert!(message.credentials.values().all(Vec::is_empty));
        }
    }

    /// More entries than a connection can have are refused before any is parsed, and the bytes of all of them are gone.
    #[test]
    fn more_secrets_than_a_connection_can_have_are_refused_and_scrubbed() {
        let mut message = connection();
        for name in ["one", "two", "three"] {
            message
                .credentials
                .insert(name.to_owned(), br#"{"k":"v"}"#.to_vec());
        }
        assert!(message.credentials.len() > MAX_SECRETS);
        assert_eq!(convert(&mut message).unwrap_err(), ConvertError::Secret);
        assert!(message.credentials.values().all(Vec::is_empty));
    }

    /// A document at the bound is read, one byte over is not (`_MAX_SECRET` of the host is the same bound).
    #[test]
    fn a_secret_of_exactly_the_bound_is_read_and_one_byte_over_is_not() {
        let filler = |length: usize| format!(r#"{{"k":"{}"}}"#, "x".repeat(length));
        let overhead = filler(0).len();
        let mut at_the_bound = connection();
        at_the_bound.credentials.insert(
            "catalog".to_owned(),
            filler(MAX_SECRET_BYTES - overhead).into_bytes(),
        );
        assert_eq!(at_the_bound.credentials["catalog"].len(), MAX_SECRET_BYTES);
        assert!(convert(&mut at_the_bound).is_ok());

        let mut over = connection();
        over.credentials.insert(
            "catalog".to_owned(),
            filler(MAX_SECRET_BYTES - overhead + 1).into_bytes(),
        );
        assert_eq!(convert(&mut over).unwrap_err(), ConvertError::Secret);
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

        // refused for what is not a secret at all: the unsupported kinds and the combinations
        let mut unsupported = pb::IcebergConnection {
            catalog: Some(Catalog::Glue(pb::IcebergGlueCatalog::default())),
            ..connection()
        };
        assert_eq!(
            convert(&mut unsupported).unwrap_err(),
            ConvertError::Unsupported
        );
        assert!(unsupported.credentials.values().all(Vec::is_empty));
    }

    /// The bytes of the message are gone, but the parsed values must be reachable only through the type that zeroises them
    /// and shows nothing: a `{:?}` of a secret is the way a value reaches a log.
    #[test]
    fn a_secret_shows_how_many_names_it_has_and_nothing_else() {
        let converted = convert(&mut connection()).unwrap();
        let shown = format!(
            "{:?} {:?}",
            converted.catalog_secret, converted.storage_secret
        );
        assert_eq!(shown, "Secret(<2 redacted>) Secret(<2 redacted>)");
        for value in [CATALOG_SECRET, STORAGE_SECRET, "AKIAEXAMPLE", "client-1"] {
            assert!(!shown.contains(value), "{shown}");
        }
        assert_eq!(
            converted.catalog_secret.get("clientSecret"),
            Some(CATALOG_SECRET)
        );
        assert_eq!(converted.catalog_secret.get("missing"), None);
        assert_eq!(converted.catalog_secret.len(), 2);
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
