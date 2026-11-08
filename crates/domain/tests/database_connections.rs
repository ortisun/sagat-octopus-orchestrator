//! FR-017-001,010,011,021: configuration boundaries before any external connection.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
use sagat_domain::database::{
    AccessDelegation, AdlsStorage, ConnectionConfig, ConnectionRef, DatabaseError, DatabaseLimits,
    DatabaseOperation, GcsStorage, GlueCatalog, HiveCatalog, HiveTls, IcebergCatalog,
    IcebergConfig, JdbcCatalog, JdbcEngine, LakeConfigError, LakeLimits, LakeStorage,
    OperationKind, Parameter, RestAuth, RestCatalog, S3Storage, StorageCredentials, StorageKind,
    TlsPolicy, TypedValue, WriteMode,
};
use sagat_domain::{classification::EgressCeiling, identity::Slug, secrets::SecretName};

fn config(host: &str) -> Result<ConnectionConfig, DatabaseError> {
    ConnectionConfig::postgres(
        host,
        5432,
        "orders",
        TlsPolicy::VerifyFull { ca_pem: None },
        SecretName::parse("orders-access").expect("name"),
        DatabaseLimits::default(),
    )
}
fn operation(
    statement: &str,
    parameters: Vec<Parameter>,
    limits: DatabaseLimits,
) -> Result<DatabaseOperation, DatabaseError> {
    DatabaseOperation::new(
        ConnectionRef::new(Slug::parse("orders-db").expect("name"), None).expect("reference"),
        OperationKind::Query,
        statement,
        parameters,
        limits,
    )
}

#[test]
fn target_is_a_bare_host_and_never_a_credential_url_or_driver_options() {
    for host in ["db.example.com", "db", "10.23.4.8", "2001:db8::5"] {
        assert!(config(host).is_ok());
    }
    for host in [
        "",
        "postgres://user:secret@db",
        "db:5432",
        "db/path",
        "db?sslmode=disable",
        "db\n",
        "-db.example",
        "db..example",
        "localhost%00",
        "[::1]",
    ] {
        assert_eq!(config(host), Err(DatabaseError::InvalidHost));
    }
    assert_eq!(
        ConnectionRef::new(Slug::parse("orders-db").expect("name"), Some(0)),
        Err(DatabaseError::InvalidRevision)
    );
}

#[test]
fn finite_limits_are_intersected_and_never_interpreted_as_unlimited() {
    let request = DatabaseLimits {
        max_rows: 4,
        ..DatabaseLimits::default()
    };
    let policy = DatabaseLimits {
        timeout_ms: 900,
        ..DatabaseLimits::default()
    };
    let effective = request.intersect(policy);
    assert_eq!((effective.max_rows, effective.timeout_ms), (4, 900));
    let mut invalid = DatabaseLimits {
        max_rows: 0,
        ..DatabaseLimits::default()
    };
    assert_eq!(invalid.validate(), Err(DatabaseError::InvalidLimits));
    invalid.max_rows = 10_001;
    assert!(invalid.validate().is_err());
    invalid = DatabaseLimits::default();
    invalid.max_result_bytes = 4 * 1024 * 1024 + 1;
    assert!(invalid.validate().is_err());
    invalid = DatabaseLimits::default();
    invalid.pool_size = 17;
    assert!(invalid.validate().is_err());
}

#[test]
fn operation_content_is_bounded_and_never_in_debug_or_validation_errors() {
    let canary = "private-content-do-not-log";
    let valid = operation(
        &format!("SELECT '{canary}'"),
        vec![Parameter::Literal(TypedValue::Text(canary.into()))],
        DatabaseLimits::default(),
    )
    .expect("operation");
    assert!(!format!("{valid:?}").contains(canary));
    assert!(operation("", vec![], DatabaseLimits::default()).is_err());
    assert!(
        operation(
            &"x".repeat(16 * 1024 + 1),
            vec![],
            DatabaseLimits::default()
        )
        .is_err()
    );
    assert!(
        operation(
            "SELECT $1",
            vec![Parameter::Literal(TypedValue::Text("x".repeat(65 * 1024)))],
            DatabaseLimits::default()
        )
        .is_err()
    );
    assert!(
        operation(
            "SELECT 1",
            vec![Parameter::Literal(TypedValue::Null); 65],
            DatabaseLimits::default()
        )
        .is_err()
    );
    assert!(operation("SELECT\0 1", vec![], DatabaseLimits::default()).is_err());
    assert!(TypedValue::float(f64::INFINITY).is_err());
    assert!(
        TypedValue::Integer("1; DROP TABLE orders".into())
            .validate()
            .is_err()
    );
    assert!(TypedValue::Decimal("NaN".into()).validate().is_err());
    assert!(
        TypedValue::Integer("9223372036854775807".into())
            .validate()
            .is_ok()
    );
    assert!(
        TypedValue::Decimal("12345678901234567890.123456789".into())
            .validate()
            .is_ok()
    );
}

#[test]
fn managed_node_policies_and_cache_weight_cannot_be_bypassed() {
    use sagat_domain::{
        saga::EffectPolicy,
        workflow::{Node, NodeId, WorkflowDefinition},
    };
    let build = |node| {
        WorkflowDefinition::new(
            Slug::parse("database-flow").expect("name"),
            NodeId::parse("query").expect("id"),
            vec![node],
        )
    };
    let make = |size| {
        Node::database(
            NodeId::parse("query").expect("id"),
            operation(&"x".repeat(size), vec![], DatabaseLimits::default()).expect("operation"),
        )
    };
    let small = build(make(10)).expect("small");
    let large = build(make(10_010)).expect("large");
    assert_eq!(large.footprint() - small.footprint(), 10_000);
    let mut write = operation(
        "UPDATE orders SET done = true",
        vec![],
        DatabaseLimits::default(),
    )
    .expect("operation");
    write.kind = OperationKind::Execute;
    let node = Node::database(NodeId::parse("query").expect("id"), write);
    assert!(build(node.clone()).is_ok());
    assert!(build(node.with_effect(EffectPolicy::NONE)).is_err());
    let mut operation =
        operation("SELECT 1", vec![], DatabaseLimits::default()).expect("operation");
    operation.limits.max_rows = 0;
    assert!(
        build(Node::database(
            NodeId::parse("query").expect("id"),
            operation
        ))
        .is_err()
    );
}

// ---- The egress ceiling of a connection (feature 035: FR-035-005, D035-040) -------------------------------------

#[test]
fn a_connection_without_a_ceiling_reads_as_internal() {
    let connection = config("db.example.com").expect("a valid connection");
    assert_eq!(connection.egress_ceiling(), EgressCeiling::Internal);
}

#[test]
fn a_ceiling_is_set_on_a_connection_and_read_back() {
    for ceiling in [
        EgressCeiling::Public,
        EgressCeiling::Internal,
        EgressCeiling::Pii,
    ] {
        let connection = config("db.example.com")
            .expect("a valid connection")
            .with_egress_ceiling(ceiling);
        assert_eq!(connection.egress_ceiling(), ceiling);
        assert_eq!(
            connection.validate(),
            Ok(()),
            "a ceiling does not make a connection invalid"
        );
    }
}

/// Changing the ceiling is a change of the connection: the revision is another value.
#[test]
fn two_connections_that_differ_only_in_the_ceiling_are_different() {
    let internal = config("db.example.com").expect("a valid connection");
    let public = internal.clone().with_egress_ceiling(EgressCeiling::Public);
    assert_ne!(internal, public);
}

#[test]
fn setting_pii_needs_the_reveal_grant_and_the_other_ceilings_do_not() {
    let connection = config("db.example.com").expect("a valid connection");
    for ceiling in [EgressCeiling::Public, EgressCeiling::Internal] {
        for holds_grant in [false, true] {
            let set = connection.clone().with_egress_ceiling(ceiling);
            assert_eq!(
                set.authorize_egress_ceiling(holds_grant),
                Ok(()),
                "{ceiling:?}, grant {holds_grant}"
            );
        }
    }
    let pii = connection.with_egress_ceiling(EgressCeiling::Pii);
    assert_eq!(
        pii.authorize_egress_ceiling(false),
        Err(DatabaseError::RevealGrantRequired)
    );
    assert_eq!(pii.authorize_egress_ceiling(true), Ok(()));
}

// -- the Iceberg connection (feature 035, T035-038, data-model.md section 1) ------------------------------------------

fn secret(name: &str) -> SecretName {
    SecretName::parse(name).expect("a secret name")
}

fn rest() -> RestCatalog {
    RestCatalog {
        uri: "https://catalog.example/api".to_owned(),
        warehouse: "analytics".to_owned(),
        prefix: None,
        auth: RestAuth::OAuth2 {
            token_uri: "https://catalog.example/oauth/token".to_owned(),
            scope: None,
        },
        access_delegation: AccessDelegation::None,
        remote_signing: false,
    }
}

fn s3() -> S3Storage {
    S3Storage {
        region: "eu-west-1".to_owned(),
        endpoint: None,
        path_style: false,
        role_arn: None,
        external_id: None,
    }
}

/// A REST catalog and S3 storage with static credentials: the delivery's first increment.
fn lake() -> IcebergConfig {
    IcebergConfig {
        catalog: IcebergCatalog::Rest(rest()),
        storage: LakeStorage {
            kind: StorageKind::S3(s3()),
            credentials: StorageCredentials::Static,
        },
        warehouse_locations: vec!["s3://lake/warehouse/".to_owned()],
        allowed_modes: vec![WriteMode::Append],
        catalog_secret: Some(secret("catalog-oauth")),
        storage_secret: Some(secret("lake-s3")),
        egress_ceiling: EgressCeiling::default(),
        limits: LakeLimits::default(),
    }
}

fn reason(config: &IcebergConfig) -> Option<LakeConfigError> {
    match ConnectionConfig::iceberg(config.clone()) {
        Ok(_) => None,
        Err(DatabaseError::Lake(reason)) => Some(reason),
        Err(other) => panic!("not a lake reason: {other:?}"),
    }
}

fn with_rest(change: impl FnOnce(&mut RestCatalog)) -> IcebergConfig {
    let mut config = lake();
    let IcebergCatalog::Rest(catalog) = &mut config.catalog else {
        panic!("a REST catalog")
    };
    change(catalog);
    config
}

#[test]
fn a_rest_and_s3_connection_builds_with_the_documented_defaults() {
    let connection = ConnectionConfig::iceberg(lake()).expect("a valid connection");

    assert_eq!(connection.driver(), "iceberg");
    assert_eq!(connection.egress_ceiling(), EgressCeiling::Internal);
    assert_eq!(
        connection
            .secrets()
            .into_iter()
            .map(SecretName::as_str)
            .collect::<Vec<_>>(),
        ["catalog-oauth", "lake-s3"]
    );
    // the SQL accessors are for a SQL connection
    assert_eq!(connection.sql_secret(), None);
    assert_eq!(connection.sql_limits(), None);
    assert!(!connection.requires_development());
    assert!(!connection.uses_unauthenticated_metastore());
}

#[test]
fn a_postgres_connection_keeps_its_driver_secret_and_limits() {
    let connection = config("db.example.com").expect("a valid connection");

    assert_eq!(connection.driver(), "postgresql");
    assert_eq!(
        connection.sql_secret().map(SecretName::as_str),
        Some("orders-access")
    );
    assert_eq!(connection.sql_limits(), Some(DatabaseLimits::default()));
    assert_eq!(connection.secrets().len(), 1);
}

/// "`catalog.type` is `rest` | `glue` | `hive` | `jdbc`": each of the four shapes carries its own members.
#[test]
fn each_of_the_four_catalog_types_builds_with_its_own_members() {
    let mut glue = lake();
    glue.catalog = IcebergCatalog::Glue(GlueCatalog {
        region: "eu-west-1".to_owned(),
        catalog_id: "123456789012".to_owned(),
        role_arn: Some("arn:aws:iam::123456789012:role/lake".to_owned()),
        external_id: Some("tenant-a".to_owned()),
    });
    let mut hive = lake();
    hive.catalog = IcebergCatalog::Hive(HiveCatalog {
        uris: vec!["thrift://hive.example:9083".to_owned()],
        tls: HiveTls::Required,
    });
    hive.catalog_secret = None;
    let mut jdbc = lake();
    jdbc.catalog = IcebergCatalog::Jdbc(JdbcCatalog {
        host: "db.example.com".to_owned(),
        port: 5432,
        database: "iceberg".to_owned(),
        tls: TlsPolicy::VerifyFull { ca_pem: None },
        engine: JdbcEngine::Postgresql,
    });

    for (name, config) in [
        ("rest", lake()),
        ("glue", glue),
        ("hive", hive),
        ("jdbc", jdbc),
    ] {
        assert_eq!(reason(&config), None, "{name}");
    }
}

/// "REST `uri` is HTTPS outside development": plain HTTP builds, and says it is for development only.
#[test]
fn a_rest_catalog_over_plain_http_is_for_development_only() {
    let plain = with_rest(|catalog| catalog.uri = "http://catalog.local:8181".to_owned());
    let connection = ConnectionConfig::iceberg(plain).expect("it builds");

    assert!(connection.requires_development());

    let plain_token = with_rest(|catalog| {
        catalog.auth = RestAuth::OAuth2 {
            token_uri: "http://catalog.local:8181/token".to_owned(),
            scope: None,
        };
    });
    assert!(
        ConnectionConfig::iceberg(plain_token)
            .expect("it builds")
            .requires_development()
    );
}

/// "A URI with user info or query credentials fails `VALIDATION_FAILED`, reason `CREDENTIAL_IN_URI`."
#[test]
fn a_uri_with_user_info_or_a_query_is_refused_as_a_credential_in_a_uri() {
    for uri in [
        "https://user:secret@catalog.example/api",
        "https://user@catalog.example/api",
        "https://catalog.example/api?token=secret",
        "https://catalog.example/api?access_key=1",
        "https://catalog.example/api#token=secret",
        "https://catalog.example:secret@catalog.example/api",
    ] {
        let by_uri = with_rest(|catalog| catalog.uri = uri.to_owned());
        assert_eq!(
            reason(&by_uri),
            Some(LakeConfigError::CredentialInUri),
            "{uri}"
        );
        let by_token_uri = with_rest(|catalog| {
            catalog.auth = RestAuth::OAuth2 {
                token_uri: uri.to_owned(),
                scope: None,
            };
        });
        assert_eq!(
            reason(&by_token_uri),
            Some(LakeConfigError::CredentialInUri),
            "token {uri}"
        );
        let mut by_endpoint = lake();
        by_endpoint.storage.kind = StorageKind::S3(S3Storage {
            endpoint: Some(uri.to_owned()),
            ..s3()
        });
        assert_eq!(
            reason(&by_endpoint),
            Some(LakeConfigError::CredentialInUri),
            "endpoint {uri}"
        );
    }
    assert_eq!(
        LakeConfigError::CredentialInUri.reason(),
        "CREDENTIAL_IN_URI"
    );
}

#[test]
fn a_uri_that_is_not_an_http_address_is_refused() {
    for uri in [
        "",
        "catalog.example",
        "ftp://catalog.example",
        "file:///etc/passwd",
        "https://",
        "https:///path",
        "https://catalog example/api",
        "https://catalog.example/api\n",
        "https://catalog.example:0/api",
        "https://catalog.example:65536/api",
        "https://catalog.example:port/api",
        "https://catalog.example:/api",
        "https://[2001:db8::1/api",
        "javascript:alert(1)",
        &format!("https://catalog.example/{}", "a".repeat(2048)),
    ] {
        let config = with_rest(|catalog| catalog.uri = uri.to_owned());
        assert_eq!(
            reason(&config),
            Some(LakeConfigError::InvalidUri),
            "{uri:?}"
        );
    }
}

#[test]
fn the_rest_warehouse_prefix_scope_and_signing_members_are_bounded() {
    for warehouse in ["", "a\nb", &"w".repeat(1025)] {
        assert_eq!(
            reason(&with_rest(
                |catalog| catalog.warehouse = warehouse.to_owned()
            )),
            Some(LakeConfigError::InvalidCatalog),
            "{warehouse:?}"
        );
    }
    for prefix in ["../x", "a b", "a?b", &"p".repeat(256)] {
        assert_eq!(
            reason(&with_rest(
                |catalog| catalog.prefix = Some(prefix.to_owned())
            )),
            Some(LakeConfigError::InvalidCatalog),
            "{prefix:?}"
        );
    }
    assert_eq!(
        reason(&with_rest(|catalog| {
            catalog.auth = RestAuth::OAuth2 {
                token_uri: "https://catalog.example/token".to_owned(),
                scope: Some("s".repeat(1025)),
            };
        })),
        Some(LakeConfigError::InvalidCatalog)
    );
    assert_eq!(
        reason(&with_rest(|catalog| {
            catalog.auth = RestAuth::SigV4 {
                region: String::new(),
                signing_name: "glue".to_owned(),
            };
        })),
        Some(LakeConfigError::InvalidCatalog)
    );
    // the members that are valid
    for change in [
        |catalog: &mut RestCatalog| catalog.prefix = Some("tenant-a/warehouse".to_owned()),
        |catalog: &mut RestCatalog| catalog.remote_signing = true,
        |catalog: &mut RestCatalog| {
            catalog.auth = RestAuth::SigV4 {
                region: "eu-west-1".to_owned(),
                signing_name: "glue".to_owned(),
            };
        },
        |catalog: &mut RestCatalog| catalog.auth = RestAuth::Bearer,
    ] {
        assert_eq!(reason(&with_rest(change)), None);
    }
}

/// "Hive `uris` are 1-4 entries of `thrift://host:port`, with `tls: none` only when the deployment sets
/// `allowUnauthenticatedMetastore`."
#[test]
fn hive_uris_are_one_to_four_thrift_addresses_and_tls_none_is_flagged_for_the_deployment() {
    let hive = |uris: Vec<&str>, tls| {
        let mut config = lake();
        config.catalog = IcebergCatalog::Hive(HiveCatalog {
            uris: uris.into_iter().map(str::to_owned).collect(),
            tls,
        });
        config.catalog_secret = None;
        config
    };
    let one = "thrift://hive.example:9083";
    assert_eq!(reason(&hive(vec![one], HiveTls::Required)), None);
    assert_eq!(reason(&hive(vec![one; 4], HiveTls::Required)), None);
    for bad in [
        vec![],
        vec![one; 5],
        vec!["http://hive.example:9083"],
        vec!["thrift://hive.example"],
        vec!["thrift://hive.example:0"],
        vec!["thrift://hive.example:65536"],
        vec!["thrift://hive.example:port"],
        vec!["thrift://user@hive.example:9083"],
        vec!["thrift://hive.example:9083/db"],
        vec!["thrift://hive.example:9083?x=1"],
        vec!["thrift://:9083"],
        vec!["thrift://hive example:9083"],
    ] {
        assert_eq!(
            reason(&hive(bad.clone(), HiveTls::Required)),
            Some(LakeConfigError::InvalidCatalog),
            "{bad:?}"
        );
    }
    let none = ConnectionConfig::iceberg(hive(vec![one], HiveTls::None)).expect("it builds");
    assert!(none.uses_unauthenticated_metastore());
    // the deployment flag gates it, not the namespace's environment
    assert!(!none.requires_development());
    let required =
        ConnectionConfig::iceberg(hive(vec![one], HiveTls::Required)).expect("it builds");
    assert!(!required.uses_unauthenticated_metastore());
}

/// "JDBC `engine` is `postgresql` (others refused in this delivery)": the type has one engine, so another cannot
/// be built. Its target is checked like a PostgreSQL connection's.
#[test]
fn a_jdbc_catalog_has_a_valid_target_and_only_the_postgresql_engine() {
    let jdbc = |host: &str, port: u16, database: &str, tls| {
        let mut config = lake();
        config.catalog = IcebergCatalog::Jdbc(JdbcCatalog {
            host: host.to_owned(),
            port,
            database: database.to_owned(),
            tls,
            engine: JdbcEngine::Postgresql,
        });
        config
    };
    let verify = || TlsPolicy::VerifyFull { ca_pem: None };
    assert_eq!(
        reason(&jdbc("db.example.com", 5432, "iceberg", verify())),
        None
    );
    assert_eq!(
        reason(&jdbc("postgres://u:p@db", 5432, "iceberg", verify())),
        Some(LakeConfigError::InvalidCatalog)
    );
    assert_eq!(
        reason(&jdbc("db.example.com", 0, "iceberg", verify())),
        Some(LakeConfigError::InvalidCatalog)
    );
    assert_eq!(
        reason(&jdbc("db.example.com", 5432, "", verify())),
        Some(LakeConfigError::InvalidCatalog)
    );
    let plaintext = ConnectionConfig::iceberg(jdbc(
        "db.example.com",
        5432,
        "iceberg",
        TlsPolicy::DevelopmentPlaintext,
    ))
    .expect("it builds");
    assert!(plaintext.requires_development());
}

#[test]
fn a_glue_catalog_needs_a_region_and_a_catalog_id_within_their_bounds() {
    let glue = |region: &str, catalog_id: &str, role: Option<String>| {
        let mut config = lake();
        config.catalog = IcebergCatalog::Glue(GlueCatalog {
            region: region.to_owned(),
            catalog_id: catalog_id.to_owned(),
            role_arn: role,
            external_id: None,
        });
        config
    };
    assert_eq!(reason(&glue("eu-west-1", "123456789012", None)), None);
    assert_eq!(
        reason(&glue("", "123456789012", None)),
        Some(LakeConfigError::InvalidCatalog)
    );
    assert_eq!(
        reason(&glue("eu-west-1", "", None)),
        Some(LakeConfigError::InvalidCatalog)
    );
    assert_eq!(
        reason(&glue("eu-west-1", &"1".repeat(256), None)),
        Some(LakeConfigError::InvalidCatalog)
    );
    assert_eq!(
        reason(&glue("eu-west-1", "123456789012", Some("r".repeat(2049)))),
        Some(LakeConfigError::InvalidCatalog)
    );
}

#[test]
fn the_storage_shapes_are_bounded() {
    let with = |kind: StorageKind, location: &str| {
        let mut config = lake();
        config.storage.kind = kind;
        config.warehouse_locations = vec![location.to_owned()];
        config
    };
    // S3
    assert_eq!(
        reason(&with(
            StorageKind::S3(S3Storage {
                path_style: true,
                ..s3()
            }),
            "s3://lake/w/"
        )),
        None
    );
    assert_eq!(
        reason(&with(
            StorageKind::S3(S3Storage {
                region: String::new(),
                ..s3()
            }),
            "s3://lake/w/"
        )),
        Some(LakeConfigError::InvalidStorage)
    );
    assert_eq!(
        reason(&with(
            StorageKind::S3(S3Storage {
                region: "r".repeat(64),
                ..s3()
            }),
            "s3://lake/w/"
        )),
        Some(LakeConfigError::InvalidStorage)
    );
    // GCS: an endpoint is an emulator, for development
    let gcs = StorageKind::Gcs(GcsStorage {
        project: Some("p".to_owned()),
        endpoint: None,
    });
    assert_eq!(reason(&with(gcs, "gs://lake/w/")), None);
    let emulator = StorageKind::Gcs(GcsStorage {
        project: None,
        endpoint: Some("http://localhost:4443".to_owned()),
    });
    let connection = ConnectionConfig::iceberg(with(emulator, "gs://lake/w/")).expect("it builds");
    assert!(connection.requires_development());
    // ADLS
    let adls = |account: &str| {
        StorageKind::Adls(AdlsStorage {
            account: account.to_owned(),
            endpoint_suffix: None,
        })
    };
    assert_eq!(
        reason(&with(
            adls("lakeaccount"),
            "abfss://data@lakeaccount.dfs.core.windows.net/w/"
        )),
        None
    );
    for bad in ["ab", "A_B", &"a".repeat(25), "acc ount"] {
        assert_eq!(
            reason(&with(
                adls(bad),
                "abfss://data@lakeaccount.dfs.core.windows.net/w/"
            )),
            Some(LakeConfigError::InvalidStorage),
            "{bad:?}"
        );
    }
}

/// "`storage.credentials: vended` is REST with `vended-credentials` only", and "no `storageSecret` when credentials are
/// `vended`". A static credential needs the secret that holds it.
#[test]
fn vended_credentials_need_a_rest_catalog_that_delegates_access_and_no_storage_secret() {
    let vended_rest = || {
        let mut config =
            with_rest(|catalog| catalog.access_delegation = AccessDelegation::VendedCredentials);
        config.storage.credentials = StorageCredentials::Vended;
        config.storage_secret = None;
        config
    };
    assert_eq!(reason(&vended_rest()), None);

    let mut with_secret = vended_rest();
    with_secret.storage_secret = Some(secret("lake-s3"));
    assert_eq!(
        reason(&with_secret),
        Some(LakeConfigError::StorageSecretWithVended)
    );

    let mut no_delegation = vended_rest();
    if let IcebergCatalog::Rest(catalog) = &mut no_delegation.catalog {
        catalog.access_delegation = AccessDelegation::None;
    }
    assert_eq!(
        reason(&no_delegation),
        Some(LakeConfigError::VendedNeedsDelegation)
    );

    let mut on_glue = vended_rest();
    on_glue.catalog = IcebergCatalog::Glue(GlueCatalog {
        region: "eu-west-1".to_owned(),
        catalog_id: "123456789012".to_owned(),
        role_arn: None,
        external_id: None,
    });
    assert_eq!(
        reason(&on_glue),
        Some(LakeConfigError::VendedNeedsDelegation)
    );

    let mut missing = lake();
    missing.storage_secret = None;
    assert_eq!(
        reason(&missing),
        Some(LakeConfigError::StorageSecretRequired)
    );
}

#[test]
fn a_catalog_secret_is_required_where_the_catalog_authenticates_and_refused_where_it_does_not() {
    let mut no_secret = lake();
    no_secret.catalog_secret = None;
    assert_eq!(
        reason(&no_secret),
        Some(LakeConfigError::CatalogSecretRequired)
    );

    let mut hive = lake();
    hive.catalog = IcebergCatalog::Hive(HiveCatalog {
        uris: vec!["thrift://hive.example:9083".to_owned()],
        tls: HiveTls::Required,
    });
    // `lake()` still holds a catalog secret, and Hive has none ("Hive: none (TLS or a private network)")
    assert_eq!(reason(&hive), Some(LakeConfigError::CatalogSecretForbidden));
    hive.catalog_secret = None;
    assert_eq!(reason(&hive), None);
}

/// "`warehouseLocations` are 1-16 URI prefixes (`s3://`, `gs://`, `abfss://`)"; every location the platform writes must
/// start with one of them (FR-035-004).
#[test]
fn warehouse_locations_are_one_to_sixteen_distinct_prefixes_of_the_storage_scheme() {
    let locations = |list: Vec<String>| {
        let mut config = lake();
        config.warehouse_locations = list;
        config
    };
    let many = |count: usize| {
        (0..count)
            .map(|n| format!("s3://bucket-{n}/w/"))
            .collect::<Vec<_>>()
    };
    assert_eq!(reason(&locations(many(1))), None);
    assert_eq!(reason(&locations(many(16))), None);
    assert_eq!(
        reason(&locations(many(0))),
        Some(LakeConfigError::InvalidLocations)
    );
    assert_eq!(
        reason(&locations(many(17))),
        Some(LakeConfigError::InvalidLocations)
    );
    assert_eq!(
        reason(&locations(vec![
            "s3://lake/w/".to_owned(),
            "s3://lake/w/".to_owned()
        ])),
        Some(LakeConfigError::InvalidLocations)
    );
    for bad in [
        "",
        "s3://",
        "s3:///w/",
        "lake/w/",
        "https://lake/w/",
        "s3://lake/../other/",
        "s3://lake/w/./x",
        "s3://lake/%2e%2e/other",
        "s3://lake/w/?x=1",
        "s3://lake/w/#frag",
        "s3://user@lake/w/",
        "s3://lake/w with space/",
        "s3://lake/w\n",
        &format!("s3://lake/{}", "a".repeat(1024)),
    ] {
        assert_eq!(
            reason(&locations(vec![bad.to_owned()])),
            Some(LakeConfigError::InvalidLocations),
            "{bad:?}"
        );
    }
    // the scheme must be the storage type's
    assert_eq!(
        reason(&locations(vec!["gs://lake/w/".to_owned()])),
        Some(LakeConfigError::LocationSchemeMismatch)
    );
    assert_eq!(
        reason(&locations(vec![
            "abfss://c@acct.dfs.core.windows.net/w/".to_owned()
        ])),
        Some(LakeConfigError::LocationSchemeMismatch)
    );
}

/// "`allowedModes` is a subset of `append`, `overwrite`, `upsert`; default `[append]`."
#[test]
fn allowed_modes_are_a_non_empty_set_of_the_three_modes() {
    let modes = |list: Vec<WriteMode>| {
        let mut config = lake();
        config.allowed_modes = list;
        config
    };
    for good in [
        vec![WriteMode::Append],
        vec![WriteMode::Overwrite],
        vec![WriteMode::Upsert],
        vec![WriteMode::Append, WriteMode::Overwrite, WriteMode::Upsert],
    ] {
        assert_eq!(reason(&modes(good)), None);
    }
    assert_eq!(reason(&modes(vec![])), Some(LakeConfigError::InvalidModes));
    assert_eq!(
        reason(&modes(vec![WriteMode::Append, WriteMode::Append])),
        Some(LakeConfigError::InvalidModes)
    );
    assert_eq!(IcebergConfig::default_modes(), vec![WriteMode::Append]);
    assert_eq!(
        [WriteMode::Append, WriteMode::Overwrite, WriteMode::Upsert].map(WriteMode::as_str),
        ["append", "overwrite", "upsert"]
    );
}

#[test]
fn the_limits_of_a_write_are_inside_their_ceilings_and_zero_is_not_unlimited() {
    let limits = |change: fn(&mut LakeLimits)| {
        let mut config = lake();
        change(&mut config.limits);
        config
    };
    assert_eq!(
        LakeLimits::default(),
        LakeLimits {
            max_record_bytes: None,
            max_records: None,
            timeout_ms: None
        }
    );
    assert_eq!(reason(&limits(|l| l.max_record_bytes = Some(1024))), None);
    assert_eq!(
        reason(&limits(|l| l.max_record_bytes = Some(2_147_483_648))),
        None
    );
    assert_eq!(reason(&limits(|l| l.max_records = Some(100_000_000))), None);
    assert_eq!(reason(&limits(|l| l.timeout_ms = Some(3_600_000))), None);
    for bad in [
        limits(|l| l.max_record_bytes = Some(0)),
        limits(|l| l.max_record_bytes = Some(1023)),
        limits(|l| l.max_record_bytes = Some(2_147_483_649)),
        limits(|l| l.max_records = Some(0)),
        limits(|l| l.max_records = Some(100_000_001)),
        limits(|l| l.timeout_ms = Some(0)),
        limits(|l| l.timeout_ms = Some(999)),
        limits(|l| l.timeout_ms = Some(3_600_001)),
    ] {
        assert_eq!(reason(&bad), Some(LakeConfigError::InvalidLimits));
    }
}

/// "Every variant, `Postgres` included, gains `egressCeiling`" with the rules of T035-009.
#[test]
fn the_iceberg_connection_carries_an_egress_ceiling_with_the_same_rules() {
    let connection = ConnectionConfig::iceberg(lake()).expect("a valid connection");
    assert_eq!(connection.egress_ceiling(), EgressCeiling::Internal);

    let public = connection
        .clone()
        .with_egress_ceiling(EgressCeiling::Public);
    assert_eq!(public.egress_ceiling(), EgressCeiling::Public);
    assert_ne!(
        connection, public,
        "a different ceiling is a different connection"
    );

    for holds_grant in [false, true] {
        assert_eq!(public.authorize_egress_ceiling(holds_grant), Ok(()));
    }
    let pii = connection.with_egress_ceiling(EgressCeiling::Pii);
    assert_eq!(
        pii.authorize_egress_ceiling(false),
        Err(DatabaseError::RevealGrantRequired)
    );
    assert_eq!(pii.authorize_egress_ceiling(true), Ok(()));
}

#[test]
fn every_lake_reason_has_a_stable_name_and_the_debug_form_shows_no_secret_value() {
    let reasons = [
        LakeConfigError::InvalidUri,
        LakeConfigError::CredentialInUri,
        LakeConfigError::InvalidCatalog,
        LakeConfigError::InvalidStorage,
        LakeConfigError::VendedNeedsDelegation,
        LakeConfigError::StorageSecretWithVended,
        LakeConfigError::StorageSecretRequired,
        LakeConfigError::CatalogSecretRequired,
        LakeConfigError::CatalogSecretForbidden,
        LakeConfigError::InvalidLocations,
        LakeConfigError::LocationSchemeMismatch,
        LakeConfigError::InvalidModes,
        LakeConfigError::InvalidLimits,
    ];
    let names: std::collections::BTreeSet<_> =
        reasons.iter().map(|reason| reason.reason()).collect();
    assert_eq!(names.len(), reasons.len(), "one name each");
    for name in &names {
        assert!(
            name.bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte == b'_'),
            "{name}"
        );
    }
    for reason in reasons {
        assert!(!DatabaseError::Lake(reason).to_string().is_empty());
    }
    // a connection holds references to secrets and never their values, and its Debug form is safe to log
    let shown = format!("{:?}", ConnectionConfig::iceberg(lake()).expect("valid"));
    assert!(shown.contains("catalog-oauth") && shown.contains("lake-s3"));
}

// ---- The secrets of a connection and the role each plays (feature 035, D035-139) -------------------------------------------

#[test]
fn each_secret_of_a_connection_has_a_role_and_they_come_in_a_fixed_order() {
    use sagat_domain::database::SecretRole;
    fn slots(config: &ConnectionConfig) -> Vec<(SecretRole, String)> {
        config
            .secret_slots()
            .into_iter()
            .map(|(role, secret)| (role, secret.as_str().to_owned()))
            .collect()
    }
    // a PostgreSQL connection has its one secret, the primary one
    let postgres = config("db.example.com").expect("a valid connection");
    assert_eq!(
        slots(&postgres),
        vec![(SecretRole::Primary, "orders-access".to_owned())]
    );
    // an Iceberg connection has the catalog's and the storage's, in that order, and either or both or neither
    let both = ConnectionConfig::iceberg(lake()).expect("a lake connection");
    assert_eq!(
        slots(&both),
        vec![
            (SecretRole::Catalog, "catalog-oauth".to_owned()),
            (SecretRole::Storage, "lake-s3".to_owned())
        ]
    );
    // REST with vended storage: the catalog's secret alone
    let mut catalog_only = lake();
    catalog_only.catalog = IcebergCatalog::Rest(RestCatalog {
        access_delegation: AccessDelegation::VendedCredentials,
        ..rest()
    });
    catalog_only.storage.credentials = StorageCredentials::Vended;
    catalog_only.storage_secret = None;
    assert_eq!(
        slots(&ConnectionConfig::iceberg(catalog_only).expect("a lake connection")),
        vec![(SecretRole::Catalog, "catalog-oauth".to_owned())]
    );
    // Hive (no catalog secret) with static storage: the storage secret alone
    let mut storage_only = lake();
    storage_only.catalog = IcebergCatalog::Hive(HiveCatalog {
        uris: vec!["thrift://hive.example:9083".to_owned()],
        tls: HiveTls::Required,
    });
    storage_only.catalog_secret = None;
    assert_eq!(
        slots(&ConnectionConfig::iceberg(storage_only).expect("a lake connection")),
        vec![(SecretRole::Storage, "lake-s3".to_owned())]
    );
    // a valid connection always has at least one: REST needs the catalog's, Hive needs the storage's, and vended storage needs REST
    // the slots are the secrets, in the same order
    assert_eq!(
        both.secret_slots()
            .into_iter()
            .map(|(_, secret)| secret)
            .collect::<Vec<_>>(),
        both.secrets()
    );
}

#[test]
fn a_secret_role_is_one_of_three_closed_names_and_nothing_else() {
    use sagat_domain::database::SecretRole;
    for role in [
        SecretRole::Primary,
        SecretRole::Catalog,
        SecretRole::Storage,
    ] {
        assert_eq!(SecretRole::parse(role.as_str()), Some(role));
    }
    assert_eq!(
        [
            SecretRole::Primary,
            SecretRole::Catalog,
            SecretRole::Storage
        ]
        .map(SecretRole::as_str),
        ["primary", "catalog", "storage"]
    );
    for refused in ["", "Primary", "sql", "catalog ", "STORAGE", "*"] {
        assert_eq!(SecretRole::parse(refused), None, "{refused:?}");
    }
}

#[test]
fn a_kind_of_managed_step_belongs_to_one_binding_profile() {
    use sagat_domain::{binding::BindingProfile, database::ManagedKind};
    assert_eq!(ManagedKind::Sql.profile(), BindingProfile::Database);
    assert_eq!(ManagedKind::Lake.profile(), BindingProfile::Lake);
}
