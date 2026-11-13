//! The stored form of an Iceberg connection (feature 035): the JSON of `database_connection_revisions.config`.
//!
//! It is the shape of the `IcebergConnectionConfig` schema of the OpenAPI contract, member by member and in camelCase,
//! so what the API shows and what is stored are one document. The egress ceiling is written by the caller, and only
//! when it is not the default. A reader ignores a member it does not know, so a newer build can add one (additive).

use super::{tls_from, tls_json};
use crate::identity::corrupt;
use sagat_application::error::StoreError;
use sagat_domain::{
    classification::EgressCeiling,
    database::{
        AccessDelegation, AdlsStorage, ConnectionConfig, GcsStorage, GlueCatalog, HiveCatalog,
        HiveTls, IcebergCatalog, IcebergConfig, JdbcCatalog, JdbcEngine, LakeLimits, LakeStorage,
        RestAuth, RestCatalog, S3Storage, StorageCredentials, StorageKind, WriteMode,
    },
    secrets::SecretName,
};
use serde_json::{Map, Value, json};

const SETTINGS: &str = "database connection settings";

/// `object` with the members that are present: an absent optional member is not written as `null`.
fn put(object: &mut Map<String, Value>, name: &str, value: Option<Value>) {
    if let Some(value) = value {
        object.insert(name.to_owned(), value);
    }
}

fn text(value: &Option<String>) -> Option<Value> {
    value.as_deref().map(Value::from)
}

pub(super) fn to_json(config: &IcebergConfig) -> Value {
    let catalog = match &config.catalog {
        IcebergCatalog::Rest(rest) => {
            let mut members = Map::new();
            members.insert("uri".to_owned(), json!(rest.uri));
            members.insert("warehouse".to_owned(), json!(rest.warehouse));
            put(&mut members, "prefix", text(&rest.prefix));
            members.insert(
                "auth".to_owned(),
                match &rest.auth {
                    RestAuth::OAuth2 { token_uri, scope } => {
                        let mut auth = Map::new();
                        auth.insert("type".to_owned(), json!("oauth2"));
                        auth.insert("tokenUri".to_owned(), json!(token_uri));
                        put(&mut auth, "scope", text(scope));
                        Value::Object(auth)
                    }
                    RestAuth::Bearer => json!({"type": "bearer"}),
                    RestAuth::SigV4 {
                        region,
                        signing_name,
                    } => {
                        json!({"type": "sigv4", "region": region, "signingName": signing_name})
                    }
                },
            );
            members.insert(
                "accessDelegation".to_owned(),
                json!(match rest.access_delegation {
                    AccessDelegation::None => "none",
                    AccessDelegation::VendedCredentials => "vended-credentials",
                }),
            );
            members.insert("remoteSigning".to_owned(), json!(rest.remote_signing));
            json!({"type": "rest", "rest": members})
        }
        IcebergCatalog::Glue(glue) => {
            let mut members = Map::new();
            members.insert("region".to_owned(), json!(glue.region));
            members.insert("catalogId".to_owned(), json!(glue.catalog_id));
            put(&mut members, "roleArn", text(&glue.role_arn));
            put(&mut members, "externalId", text(&glue.external_id));
            json!({"type": "glue", "glue": members})
        }
        IcebergCatalog::Hive(hive) => json!({
            "type": "hive",
            "hive": {"uris": hive.uris, "tls": match hive.tls { HiveTls::Required => "required", HiveTls::None => "none" }},
        }),
        IcebergCatalog::Jdbc(jdbc) => json!({
            "type": "jdbc",
            "jdbc": {
                "host": jdbc.host, "port": jdbc.port, "database": jdbc.database,
                "tls": tls_json(&jdbc.tls), "engine": match jdbc.engine { JdbcEngine::Postgresql => "postgresql" },
            },
        }),
    };
    let credentials = match config.storage.credentials {
        StorageCredentials::Static => "static",
        StorageCredentials::Vended => "vended",
    };
    let mut storage = match &config.storage.kind {
        StorageKind::S3(s3) => {
            let mut members = Map::new();
            members.insert("region".to_owned(), json!(s3.region));
            put(&mut members, "endpoint", text(&s3.endpoint));
            members.insert("pathStyle".to_owned(), json!(s3.path_style));
            put(&mut members, "roleArn", text(&s3.role_arn));
            put(&mut members, "externalId", text(&s3.external_id));
            json!({"type": "s3", "s3": members})
        }
        StorageKind::Gcs(gcs) => {
            let mut members = Map::new();
            put(&mut members, "project", text(&gcs.project));
            put(&mut members, "endpoint", text(&gcs.endpoint));
            json!({"type": "gcs", "gcs": members})
        }
        StorageKind::Adls(adls) => {
            let mut members = Map::new();
            members.insert("account".to_owned(), json!(adls.account));
            put(&mut members, "endpointSuffix", text(&adls.endpoint_suffix));
            json!({"type": "adls", "adls": members})
        }
    };
    storage["credentials"] = json!(credentials);
    let mut limits = Map::new();
    put(
        &mut limits,
        "maxRecordBytes",
        config.limits.max_record_bytes.map(Value::from),
    );
    put(
        &mut limits,
        "maxRecords",
        config.limits.max_records.map(Value::from),
    );
    put(
        &mut limits,
        "timeoutMs",
        config.limits.timeout_ms.map(Value::from),
    );
    let mut json = json!({
        "driver": "iceberg",
        "catalog": catalog,
        "storage": storage,
        "warehouseLocations": config.warehouse_locations,
        "allowedModes": config.allowed_modes.iter().map(|mode| mode.as_str()).collect::<Vec<_>>(),
        "limits": limits,
    });
    if let Some(name) = &config.catalog_secret {
        json["catalogSecret"] = json!(name.as_str());
    }
    if let Some(name) = &config.storage_secret {
        json["storageSecret"] = json!(name.as_str());
    }
    json
}

fn string(value: &Value) -> Result<String, StoreError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| corrupt(SETTINGS))
}

fn optional(value: &Value) -> Result<Option<String>, StoreError> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        _ => Err(corrupt(SETTINGS)),
    }
}

fn boolean(value: &Value, default: bool) -> Result<bool, StoreError> {
    match value {
        Value::Null => Ok(default),
        Value::Bool(flag) => Ok(*flag),
        _ => Err(corrupt(SETTINGS)),
    }
}

fn number(value: &Value) -> Result<u32, StoreError> {
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| corrupt(SETTINGS))
}

fn optional_number(value: &Value) -> Result<Option<u32>, StoreError> {
    match value {
        Value::Null => Ok(None),
        other => number(other).map(Some),
    }
}

fn secret(value: &Value) -> Result<Option<SecretName>, StoreError> {
    optional(value)?
        .map(|name| SecretName::parse(&name).map_err(|_| corrupt("database secret reference")))
        .transpose()
}

fn catalog_from(value: &Value) -> Result<IcebergCatalog, StoreError> {
    match value["type"].as_str() {
        Some("rest") => {
            let rest = &value["rest"];
            let auth = &rest["auth"];
            let auth = match auth["type"].as_str() {
                Some("oauth2") => RestAuth::OAuth2 {
                    token_uri: string(&auth["tokenUri"])?,
                    scope: optional(&auth["scope"])?,
                },
                Some("bearer") => RestAuth::Bearer,
                Some("sigv4") => RestAuth::SigV4 {
                    region: string(&auth["region"])?,
                    signing_name: string(&auth["signingName"])?,
                },
                _ => return Err(corrupt(SETTINGS)),
            };
            let access_delegation = match rest["accessDelegation"].as_str() {
                None | Some("none") => AccessDelegation::None,
                Some("vended-credentials") => AccessDelegation::VendedCredentials,
                _ => return Err(corrupt(SETTINGS)),
            };
            Ok(IcebergCatalog::Rest(RestCatalog {
                uri: string(&rest["uri"])?,
                warehouse: string(&rest["warehouse"])?,
                prefix: optional(&rest["prefix"])?,
                auth,
                access_delegation,
                remote_signing: boolean(&rest["remoteSigning"], false)?,
            }))
        }
        Some("glue") => {
            let glue = &value["glue"];
            Ok(IcebergCatalog::Glue(GlueCatalog {
                region: string(&glue["region"])?,
                catalog_id: string(&glue["catalogId"])?,
                role_arn: optional(&glue["roleArn"])?,
                external_id: optional(&glue["externalId"])?,
            }))
        }
        Some("hive") => {
            let hive = &value["hive"];
            let uris = hive["uris"]
                .as_array()
                .ok_or_else(|| corrupt(SETTINGS))?
                .iter()
                .map(string)
                .collect::<Result<Vec<_>, _>>()?;
            let tls = match hive["tls"].as_str() {
                Some("required") => HiveTls::Required,
                Some("none") => HiveTls::None,
                _ => return Err(corrupt(SETTINGS)),
            };
            Ok(IcebergCatalog::Hive(HiveCatalog { uris, tls }))
        }
        Some("jdbc") => {
            let jdbc = &value["jdbc"];
            if jdbc["engine"].as_str() != Some("postgresql") {
                return Err(corrupt(SETTINGS));
            }
            Ok(IcebergCatalog::Jdbc(JdbcCatalog {
                host: string(&jdbc["host"])?,
                port: jdbc["port"]
                    .as_u64()
                    .and_then(|n| u16::try_from(n).ok())
                    .ok_or_else(|| corrupt(SETTINGS))?,
                database: string(&jdbc["database"])?,
                tls: tls_from(&jdbc["tls"])?,
                engine: JdbcEngine::Postgresql,
            }))
        }
        _ => Err(corrupt(SETTINGS)),
    }
}

fn storage_from(value: &Value) -> Result<LakeStorage, StoreError> {
    let kind = match value["type"].as_str() {
        Some("s3") => {
            let s3 = &value["s3"];
            StorageKind::S3(S3Storage {
                region: string(&s3["region"])?,
                endpoint: optional(&s3["endpoint"])?,
                path_style: boolean(&s3["pathStyle"], false)?,
                role_arn: optional(&s3["roleArn"])?,
                external_id: optional(&s3["externalId"])?,
            })
        }
        Some("gcs") => {
            let gcs = &value["gcs"];
            StorageKind::Gcs(GcsStorage {
                project: optional(&gcs["project"])?,
                endpoint: optional(&gcs["endpoint"])?,
            })
        }
        Some("adls") => {
            let adls = &value["adls"];
            StorageKind::Adls(AdlsStorage {
                account: string(&adls["account"])?,
                endpoint_suffix: optional(&adls["endpointSuffix"])?,
            })
        }
        _ => return Err(corrupt(SETTINGS)),
    };
    let credentials = match value["credentials"].as_str() {
        None | Some("static") => StorageCredentials::Static,
        Some("vended") => StorageCredentials::Vended,
        _ => return Err(corrupt(SETTINGS)),
    };
    Ok(LakeStorage { kind, credentials })
}

/// The connection of a stored document, checked again against every rule of the domain: a document that does not pass
/// is corrupt, and is not used.
pub(super) fn from_json(value: &Value) -> Result<ConnectionConfig, StoreError> {
    let modes = match &value["allowedModes"] {
        Value::Null => IcebergConfig::default_modes(),
        Value::Array(modes) => modes
            .iter()
            .map(|mode| match mode.as_str() {
                Some("append") => Ok(WriteMode::Append),
                Some("overwrite") => Ok(WriteMode::Overwrite),
                Some("upsert") => Ok(WriteMode::Upsert),
                _ => Err(corrupt(SETTINGS)),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(corrupt(SETTINGS)),
    };
    let limits = &value["limits"];
    let config = IcebergConfig {
        catalog: catalog_from(&value["catalog"])?,
        storage: storage_from(&value["storage"])?,
        warehouse_locations: value["warehouseLocations"]
            .as_array()
            .ok_or_else(|| corrupt(SETTINGS))?
            .iter()
            .map(string)
            .collect::<Result<Vec<_>, _>>()?,
        allowed_modes: modes,
        catalog_secret: secret(&value["catalogSecret"])?,
        storage_secret: secret(&value["storageSecret"])?,
        egress_ceiling: EgressCeiling::default(),
        limits: LakeLimits {
            max_record_bytes: optional_number(&limits["maxRecordBytes"])?,
            max_records: optional_number(&limits["maxRecords"])?,
            timeout_ms: optional_number(&limits["timeoutMs"])?,
        },
    };
    // what was saved stays readable when the rules of saving tighten (O035-010)
    ConnectionConfig::iceberg_stored(config).map_err(|_| corrupt(SETTINGS))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::database::{config_from, config_json};

    type Edits = Vec<(&'static str, Box<dyn Fn(&mut Value)>)>;

    fn secret_name(name: &str) -> SecretName {
        SecretName::parse(name).expect("a secret name")
    }

    /// REST and S3 with static credentials, the first increment, with every optional member absent.
    fn rest_s3() -> IcebergConfig {
        IcebergConfig {
            catalog: IcebergCatalog::Rest(RestCatalog {
                uri: "https://catalog.example/api".to_owned(),
                warehouse: "analytics".to_owned(),
                prefix: None,
                auth: RestAuth::OAuth2 {
                    token_uri: "https://catalog.example/oauth/token".to_owned(),
                    scope: None,
                },
                access_delegation: AccessDelegation::None,
                remote_signing: false,
            }),
            storage: LakeStorage {
                kind: StorageKind::S3(S3Storage {
                    region: "eu-west-1".to_owned(),
                    endpoint: None,
                    path_style: false,
                    role_arn: None,
                    external_id: None,
                }),
                credentials: StorageCredentials::Static,
            },
            warehouse_locations: vec!["s3://lake/warehouse/".to_owned()],
            allowed_modes: vec![WriteMode::Append],
            catalog_secret: Some(secret_name("catalog-oauth")),
            storage_secret: Some(secret_name("lake-s3")),
            egress_ceiling: EgressCeiling::default(),
            limits: LakeLimits::default(),
        }
    }

    fn connection(config: IcebergConfig) -> ConnectionConfig {
        ConnectionConfig::iceberg(config).expect("a valid connection")
    }

    /// The stored document of a REST and S3 connection, written by hand from the contract and not by `to_json`: a
    /// change of the stored shape fails here, and a revision written by an earlier build stays readable.
    #[test]
    fn a_rest_and_s3_connection_is_stored_as_the_document_of_the_contract() {
        let expected = json!({
            "driver": "iceberg",
            "catalog": {
                "type": "rest",
                "rest": {
                    "uri": "https://catalog.example/api",
                    "warehouse": "analytics",
                    "auth": {"type": "oauth2", "tokenUri": "https://catalog.example/oauth/token"},
                    "accessDelegation": "none",
                    "remoteSigning": false
                }
            },
            "storage": {"type": "s3", "s3": {"region": "eu-west-1", "pathStyle": false}, "credentials": "static"},
            "warehouseLocations": ["s3://lake/warehouse/"],
            "allowedModes": ["append"],
            "catalogSecret": "catalog-oauth",
            "storageSecret": "lake-s3",
            "limits": {}
        });
        let config = connection(rest_s3());

        assert_eq!(config_json(&config), expected);
        assert_eq!(config_from(&expected).expect("it reads"), config);
    }

    #[test]
    fn every_catalog_and_storage_shape_survives_a_round_trip() {
        let mut shapes = Vec::new();
        let mut glue = rest_s3();
        glue.catalog = IcebergCatalog::Glue(GlueCatalog {
            region: "eu-west-1".to_owned(),
            catalog_id: "123456789012".to_owned(),
            role_arn: Some("arn:aws:iam::123456789012:role/lake".to_owned()),
            external_id: Some("tenant-a".to_owned()),
        });
        shapes.push(glue);
        let mut hive = rest_s3();
        hive.catalog = IcebergCatalog::Hive(HiveCatalog {
            uris: vec![
                "thrift://hive-1.example:9083".to_owned(),
                "thrift://hive-2.example:9083".to_owned(),
            ],
            tls: HiveTls::None,
        });
        hive.catalog_secret = None;
        shapes.push(hive);
        let mut jdbc = rest_s3();
        jdbc.catalog = IcebergCatalog::Jdbc(JdbcCatalog {
            host: "db.example.com".to_owned(),
            port: 5432,
            database: "iceberg".to_owned(),
            tls: sagat_domain::database::TlsPolicy::VerifyFull { ca_pem: None },
            engine: JdbcEngine::Postgresql,
        });
        shapes.push(jdbc);
        let mut sigv4 = rest_s3();
        if let IcebergCatalog::Rest(rest) = &mut sigv4.catalog {
            rest.prefix = Some("tenant-a/warehouse".to_owned());
            rest.remote_signing = true;
            rest.auth = RestAuth::SigV4 {
                region: "eu-west-1".to_owned(),
                signing_name: "glue".to_owned(),
            };
        }
        shapes.push(sigv4);
        let mut bearer = rest_s3();
        if let IcebergCatalog::Rest(rest) = &mut bearer.catalog {
            rest.auth = RestAuth::Bearer;
        }
        shapes.push(bearer);
        let mut vended = rest_s3();
        if let IcebergCatalog::Rest(rest) = &mut vended.catalog {
            rest.access_delegation = AccessDelegation::VendedCredentials;
            rest.auth = RestAuth::OAuth2 {
                token_uri: "https://catalog.example/oauth/token".to_owned(),
                scope: Some("catalog".to_owned()),
            };
        }
        vended.storage.credentials = StorageCredentials::Vended;
        vended.storage_secret = None;
        shapes.push(vended);
        let mut gcs = rest_s3();
        gcs.storage.kind = StorageKind::Gcs(GcsStorage {
            project: Some("project".to_owned()),
            endpoint: Some("http://localhost:4443".to_owned()),
        });
        gcs.warehouse_locations = vec!["gs://lake/w/".to_owned()];
        shapes.push(gcs);
        let mut adls = rest_s3();
        adls.storage.kind = StorageKind::Adls(AdlsStorage {
            account: "lakeaccount".to_owned(),
            endpoint_suffix: Some("core.windows.net".to_owned()),
        });
        adls.warehouse_locations =
            vec!["abfss://data@lakeaccount.dfs.core.windows.net/w/".to_owned()];
        shapes.push(adls);
        let mut s3_compatible = rest_s3();
        s3_compatible.storage.kind = StorageKind::S3(S3Storage {
            region: "us-east-1".to_owned(),
            endpoint: Some("http://localhost:9000".to_owned()),
            path_style: true,
            role_arn: Some("arn:aws:iam::1:role/r".to_owned()),
            external_id: Some("external".to_owned()),
        });
        s3_compatible.allowed_modes =
            vec![WriteMode::Upsert, WriteMode::Append, WriteMode::Overwrite];
        s3_compatible.limits = LakeLimits {
            max_record_bytes: Some(4096),
            max_records: Some(500),
            timeout_ms: Some(60_000),
        };
        shapes.push(s3_compatible);

        for shape in shapes {
            let config = connection(shape);
            let stored = config_json(&config);
            assert_eq!(config_from(&stored).expect("it reads"), config, "{stored}");
        }
    }

    /// O035-010: a plain `http` address names the loopback **when a connection is saved**. A revision that an earlier build saved with another
    /// host (a development namespace could) stays readable: it must not turn into a corrupt row that fails a listing, an admission or a
    /// replay, and it still says that it is for development only.
    #[test]
    fn a_revision_stored_before_plain_http_named_the_loopback_stays_readable() {
        let mut stored = config_json(&connection(rest_s3()));
        stored["catalog"]["rest"]["uri"] = json!("http://catalog.local:8181/api");

        let read = config_from(&stored).expect("an earlier build saved it, and it stays readable");

        assert!(read.requires_development());
    }

    /// The order of the modes is kept: the replay of a write compares the stored document with the new one.
    #[test]
    fn the_modes_keep_the_order_they_were_written_in() {
        let mut shape = rest_s3();
        shape.allowed_modes = vec![WriteMode::Upsert, WriteMode::Append];
        let stored = config_json(&connection(shape));

        assert_eq!(stored["allowedModes"], json!(["upsert", "append"]));
    }

    #[test]
    fn the_default_ceiling_is_not_written_and_another_one_is() {
        let internal = config_json(&connection(rest_s3()));
        assert!(internal.get("egressCeiling").is_none());

        let pii = config_json(&connection(rest_s3()).with_egress_ceiling(EgressCeiling::Pii));
        assert_eq!(pii["egressCeiling"], "pii");
        assert_eq!(
            config_from(&pii).expect("it reads").egress_ceiling(),
            EgressCeiling::Pii
        );
    }

    #[test]
    fn a_document_without_the_optional_members_reads_with_their_defaults() {
        let minimal = json!({
            "driver": "iceberg",
            "catalog": {"type": "rest", "rest": {
                "uri": "https://catalog.example/api", "warehouse": "analytics",
                "auth": {"type": "bearer"}
            }},
            "storage": {"type": "s3", "s3": {"region": "eu-west-1"}},
            "warehouseLocations": ["s3://lake/w/"],
            "catalogSecret": "catalog-token",
            "storageSecret": "lake-s3"
        });

        let config = config_from(&minimal).expect("it reads");

        let ConnectionConfig::Iceberg(iceberg) = &config else {
            panic!("an Iceberg connection")
        };
        assert_eq!(iceberg.allowed_modes, vec![WriteMode::Append]);
        assert_eq!(iceberg.egress_ceiling, EgressCeiling::Internal);
        assert_eq!(iceberg.limits, LakeLimits::default());
        assert_eq!(iceberg.storage.credentials, StorageCredentials::Static);
        match &iceberg.catalog {
            IcebergCatalog::Rest(rest) => {
                assert_eq!(rest.access_delegation, AccessDelegation::None);
                assert!(!rest.remote_signing);
            }
            other => panic!("not REST: {other:?}"),
        }
    }

    #[test]
    fn a_member_this_build_does_not_know_is_ignored() {
        let mut stored = config_json(&connection(rest_s3()));
        stored["futureMember"] = json!({"anything": 1});
        stored["catalog"]["rest"]["futureMember"] = json!(true);

        assert_eq!(
            config_from(&stored).expect("it reads"),
            connection(rest_s3())
        );
    }

    /// A stored document that breaks a rule of the domain is corrupt and is not used, whatever wrote it.
    #[test]
    fn a_corrupt_document_is_refused() {
        let good = config_json(&connection(rest_s3()));
        let edits: Edits = vec![
            (
                "an unknown driver",
                Box::new(|v| v["driver"] = json!("clickhouse")),
            ),
            (
                "no driver",
                Box::new(|v| {
                    v.as_object_mut().unwrap().remove("driver");
                }),
            ),
            (
                "an unknown catalog type",
                Box::new(|v| v["catalog"]["type"] = json!("nessie")),
            ),
            (
                "no catalog",
                Box::new(|v| {
                    v.as_object_mut().unwrap().remove("catalog");
                }),
            ),
            (
                "an unknown auth",
                Box::new(|v| v["catalog"]["rest"]["auth"]["type"] = json!("basic")),
            ),
            (
                "no token endpoint",
                Box::new(|v| {
                    v["catalog"]["rest"]["auth"]
                        .as_object_mut()
                        .unwrap()
                        .remove("tokenUri");
                }),
            ),
            (
                "an unknown delegation",
                Box::new(|v| v["catalog"]["rest"]["accessDelegation"] = json!("sometimes")),
            ),
            (
                "remote signing as a string",
                Box::new(|v| v["catalog"]["rest"]["remoteSigning"] = json!("yes")),
            ),
            (
                "an unknown storage type",
                Box::new(|v| v["storage"]["type"] = json!("hdfs")),
            ),
            (
                "unknown credentials",
                Box::new(|v| v["storage"]["credentials"] = json!("ambient")),
            ),
            (
                "no locations",
                Box::new(|v| {
                    v.as_object_mut().unwrap().remove("warehouseLocations");
                }),
            ),
            (
                "a location that is not a string",
                Box::new(|v| v["warehouseLocations"] = json!([5])),
            ),
            (
                "a location outside the scheme",
                Box::new(|v| v["warehouseLocations"] = json!(["gs://lake/w/"])),
            ),
            (
                "an unknown mode",
                Box::new(|v| v["allowedModes"] = json!(["merge"])),
            ),
            (
                "modes that are not a list",
                Box::new(|v| v["allowedModes"] = json!("append")),
            ),
            (
                "no mode at all",
                Box::new(|v| v["allowedModes"] = json!([])),
            ),
            (
                "a secret that is not a name",
                Box::new(|v| v["catalogSecret"] = json!("Not A Name")),
            ),
            (
                "a secret that is a number",
                Box::new(|v| v["storageSecret"] = json!(7)),
            ),
            (
                "no storage secret for static credentials",
                Box::new(|v| {
                    v.as_object_mut().unwrap().remove("storageSecret");
                }),
            ),
            (
                "a limit as a string",
                Box::new(|v| v["limits"]["timeoutMs"] = json!("fast")),
            ),
            (
                "a limit above its ceiling",
                Box::new(|v| v["limits"]["maxRecords"] = json!(1_000_000_000)),
            ),
            (
                "an unknown ceiling",
                Box::new(|v| v["egressCeiling"] = json!("secret")),
            ),
        ];
        for (name, edit) in edits {
            let mut stored = good.clone();
            edit(&mut stored);
            assert!(config_from(&stored).is_err(), "{name}: {stored}");
        }
    }

    #[test]
    fn a_stored_jdbc_catalog_with_another_engine_is_corrupt() {
        let mut jdbc = rest_s3();
        jdbc.catalog = IcebergCatalog::Jdbc(JdbcCatalog {
            host: "db.example.com".to_owned(),
            port: 5432,
            database: "iceberg".to_owned(),
            tls: sagat_domain::database::TlsPolicy::VerifyFull { ca_pem: None },
            engine: JdbcEngine::Postgresql,
        });
        let mut stored = config_json(&connection(jdbc));
        stored["catalog"]["jdbc"]["engine"] = json!("mysql");

        assert!(config_from(&stored).is_err());
    }
}
