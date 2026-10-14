//! Reading the blob store settings from the environment (ADR-0023).
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::time::Duration;

use sagat_infra_blobstore::{
    BlobSettings, BlobStoreConfig, ConfigError, Jurisdiction, PartSize, R2Config, Secret,
};

fn read(pairs: &[(&str, &str)]) -> Result<Option<BlobSettings>, ConfigError> {
    let env: BTreeMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    BlobSettings::from_lookup(|name| env.get(name).cloned())
}

#[test]
fn without_a_provider_there_is_no_blob_store() {
    assert!(read(&[]).expect("ok").is_none());
    assert!(
        read(&[("SAGAT_BLOB_PROVIDER", "none")])
            .expect("ok")
            .is_none()
    );
    assert!(read(&[("SAGAT_BLOB_PROVIDER", "")]).expect("ok").is_none());
}

#[test]
fn a_local_directory_and_memory_need_nothing_else() {
    let fs = read(&[
        ("SAGAT_BLOB_PROVIDER", "fs"),
        ("SAGAT_BLOB_FS_ROOT", "/var/lib/blobs"),
    ])
    .expect("ok")
    .expect("enabled");
    assert!(
        matches!(&fs.store, BlobStoreConfig::Fs { root } if root.to_str() == Some("/var/lib/blobs"))
    );
    let memory = read(&[("SAGAT_BLOB_PROVIDER", "memory")])
        .expect("ok")
        .expect("enabled");
    assert!(matches!(memory.store, BlobStoreConfig::Memory));

    // Every limit has a default.
    assert_eq!(memory.limits.max_bytes, 64 * 1024 * 1024);
    assert_eq!(memory.limits.max_pending, 64);
    assert_eq!(memory.limits.upload_ttl, Duration::from_secs(24 * 3600));
    assert_eq!(memory.limits.abandoned_after, Duration::from_secs(3600));
    assert_eq!(memory.part_size, PartSize::default());

    assert_eq!(
        read(&[("SAGAT_BLOB_PROVIDER", "fs")]).err(),
        Some(ConfigError::Missing("SAGAT_BLOB_FS_ROOT"))
    );
}

#[test]
fn s3_needs_a_bucket_and_a_region_and_may_use_the_ambient_aws_credentials() {
    let settings = read(&[
        ("SAGAT_BLOB_PROVIDER", "s3"),
        ("SAGAT_BLOB_S3_BUCKET", "sagat-prod"),
        ("SAGAT_BLOB_S3_REGION", "eu-west-1"),
    ])
    .expect("ok")
    .expect("enabled");
    let BlobStoreConfig::S3(s3) = settings.store else {
        panic!("expected S3");
    };
    assert_eq!(s3.bucket, "sagat-prod");
    assert_eq!(s3.region, "eu-west-1");
    assert!(s3.credentials.is_none(), "the AWS credential chain is used");
    assert!(s3.endpoint.is_none());
    assert!(!s3.path_style && !s3.allow_http);

    for missing in ["SAGAT_BLOB_S3_BUCKET", "SAGAT_BLOB_S3_REGION"] {
        let pairs: Vec<_> = [
            ("SAGAT_BLOB_PROVIDER", "s3"),
            ("SAGAT_BLOB_S3_BUCKET", "b"),
            ("SAGAT_BLOB_S3_REGION", "r"),
        ]
        .into_iter()
        .filter(|(k, _)| *k != missing)
        .collect();
        assert_eq!(read(&pairs).err(), Some(ConfigError::Missing(missing)));
    }
}

#[test]
fn s3_credentials_come_as_a_pair_and_never_print() {
    let both = read(&[
        ("SAGAT_BLOB_PROVIDER", "s3"),
        ("SAGAT_BLOB_S3_BUCKET", "b"),
        ("SAGAT_BLOB_S3_REGION", "r"),
        ("SAGAT_BLOB_S3_ACCESS_KEY_ID", "AKIAEXAMPLE"),
        ("SAGAT_BLOB_S3_SECRET_ACCESS_KEY", "super-secret-value"),
    ])
    .expect("ok")
    .expect("enabled");
    let printed = format!("{both:?}");
    assert!(!printed.contains("super-secret-value"), "{printed}");
    assert!(printed.contains("<redacted>"), "{printed}");

    let half = read(&[
        ("SAGAT_BLOB_PROVIDER", "s3"),
        ("SAGAT_BLOB_S3_BUCKET", "b"),
        ("SAGAT_BLOB_S3_REGION", "r"),
        ("SAGAT_BLOB_S3_ACCESS_KEY_ID", "AKIAEXAMPLE"),
    ]);
    assert_eq!(
        half.err(),
        Some(ConfigError::Incomplete(
            "SAGAT_BLOB_S3_ACCESS_KEY_ID and SAGAT_BLOB_S3_SECRET_ACCESS_KEY go together"
        ))
    );
}

#[test]
fn plain_http_to_a_store_has_to_be_asked_for_by_name() {
    let base = [
        ("SAGAT_BLOB_PROVIDER", "s3"),
        ("SAGAT_BLOB_S3_BUCKET", "b"),
        ("SAGAT_BLOB_S3_REGION", "r"),
        ("SAGAT_BLOB_S3_ENDPOINT", "http://localhost:9000"),
    ];
    assert_eq!(read(&base).err(), Some(ConfigError::InsecureEndpoint));

    let mut allowed = base.to_vec();
    allowed.push(("SAGAT_BLOB_S3_ALLOW_HTTP", "true"));
    assert!(read(&allowed).expect("ok").is_some());

    let mut https = base.to_vec();
    https[3] = ("SAGAT_BLOB_S3_ENDPOINT", "https://minio.internal:9000");
    assert!(read(&https).expect("ok").is_some());

    let mut nonsense = base.to_vec();
    nonsense[3] = ("SAGAT_BLOB_S3_ENDPOINT", "ftp://x");
    assert!(matches!(
        read(&nonsense).err(),
        Some(ConfigError::Invalid { .. })
    ));
}

#[test]
fn cloudflare_r2_is_addressed_by_account_and_jurisdiction() {
    let r2 = |jurisdiction: Option<Jurisdiction>| R2Config {
        account_id: "abc123".to_owned(),
        bucket: "b".to_owned(),
        credentials: sagat_infra_blobstore::Credentials {
            access_key_id: "k".to_owned(),
            secret_access_key: Secret::new("s"),
            session_token: None,
        },
        jurisdiction,
        endpoint_override: None,
        prefix: None,
    };

    assert_eq!(
        r2(None).endpoint(),
        "https://abc123.r2.cloudflarestorage.com"
    );
    assert_eq!(
        r2(Some(Jurisdiction::Eu)).endpoint(),
        "https://abc123.eu.r2.cloudflarestorage.com"
    );
    assert_eq!(
        r2(Some(Jurisdiction::Fedramp)).endpoint(),
        "https://abc123.fedramp.r2.cloudflarestorage.com"
    );
}

#[test]
fn cloudflare_r2_settings_are_read_in_full() {
    let settings = read(&[
        ("SAGAT_BLOB_PROVIDER", "r2"),
        ("SAGAT_BLOB_R2_ACCOUNT_ID", "abc123"),
        ("SAGAT_BLOB_R2_BUCKET", "sagat"),
        ("SAGAT_BLOB_R2_ACCESS_KEY_ID", "key"),
        ("SAGAT_BLOB_R2_SECRET_ACCESS_KEY", "secret"),
        ("SAGAT_BLOB_R2_JURISDICTION", "eu"),
        ("SAGAT_BLOB_PREFIX", "prod/"),
    ])
    .expect("ok")
    .expect("enabled");
    let BlobStoreConfig::R2(r2) = settings.store else {
        panic!("expected R2");
    };
    assert_eq!(r2.endpoint(), "https://abc123.eu.r2.cloudflarestorage.com");
    assert_eq!(
        r2.prefix.as_deref(),
        Some("prod"),
        "a trailing slash is not part of a prefix"
    );

    for missing in [
        "SAGAT_BLOB_R2_ACCOUNT_ID",
        "SAGAT_BLOB_R2_BUCKET",
        "SAGAT_BLOB_R2_ACCESS_KEY_ID",
        "SAGAT_BLOB_R2_SECRET_ACCESS_KEY",
    ] {
        let all = [
            ("SAGAT_BLOB_PROVIDER", "r2"),
            ("SAGAT_BLOB_R2_ACCOUNT_ID", "a"),
            ("SAGAT_BLOB_R2_BUCKET", "b"),
            ("SAGAT_BLOB_R2_ACCESS_KEY_ID", "k"),
            ("SAGAT_BLOB_R2_SECRET_ACCESS_KEY", "s"),
        ];
        let pairs: Vec<_> = all.into_iter().filter(|(k, _)| *k != missing).collect();
        assert_eq!(read(&pairs).err(), Some(ConfigError::Missing(missing)));
    }

    let bad = read(&[
        ("SAGAT_BLOB_PROVIDER", "r2"),
        ("SAGAT_BLOB_R2_ACCOUNT_ID", "a"),
        ("SAGAT_BLOB_R2_BUCKET", "b"),
        ("SAGAT_BLOB_R2_ACCESS_KEY_ID", "k"),
        ("SAGAT_BLOB_R2_SECRET_ACCESS_KEY", "s"),
        ("SAGAT_BLOB_R2_JURISDICTION", "mars"),
    ]);
    assert!(matches!(bad.err(), Some(ConfigError::Invalid { .. })));
}

#[test]
fn an_account_id_cannot_smuggle_a_host_into_the_endpoint() {
    for evil in ["a.evil.example", "a/b", "a@b", "a b", "", "A_B"] {
        let result = read(&[
            ("SAGAT_BLOB_PROVIDER", "r2"),
            ("SAGAT_BLOB_R2_ACCOUNT_ID", evil),
            ("SAGAT_BLOB_R2_BUCKET", "b"),
            ("SAGAT_BLOB_R2_ACCESS_KEY_ID", "k"),
            ("SAGAT_BLOB_R2_SECRET_ACCESS_KEY", "s"),
        ]);
        assert!(result.is_err(), "{evil:?} must be refused");
    }
}

#[test]
fn limits_and_part_size_can_be_tuned_within_sane_bounds() {
    let settings = read(&[
        ("SAGAT_BLOB_PROVIDER", "memory"),
        ("SAGAT_BLOB_MAX_BYTES", "1048576"),
        ("SAGAT_BLOB_MAX_PENDING", "5"),
        ("SAGAT_BLOB_UPLOAD_TTL_SECS", "600"),
        ("SAGAT_BLOB_ABANDONED_AFTER_SECS", "120"),
        ("SAGAT_BLOB_PART_SIZE_MIB", "16"),
    ])
    .expect("ok")
    .expect("enabled");
    assert_eq!(settings.limits.max_bytes, 1_048_576);
    assert_eq!(settings.limits.max_pending, 5);
    assert_eq!(settings.limits.upload_ttl, Duration::from_secs(600));
    assert_eq!(settings.limits.abandoned_after, Duration::from_secs(120));
    assert_eq!(settings.part_size.bytes(), 16 * 1024 * 1024);

    for (name, bad) in [
        ("SAGAT_BLOB_MAX_BYTES", "0"),
        ("SAGAT_BLOB_MAX_BYTES", "many"),
        ("SAGAT_BLOB_MAX_BYTES", "99999999999999"),
        ("SAGAT_BLOB_MAX_PENDING", "0"),
        ("SAGAT_BLOB_UPLOAD_TTL_SECS", "0"),
        ("SAGAT_BLOB_ABANDONED_AFTER_SECS", "0"),
        ("SAGAT_BLOB_PART_SIZE_MIB", "1"),
        ("SAGAT_BLOB_PART_SIZE_MIB", "100000"),
    ] {
        let result = read(&[("SAGAT_BLOB_PROVIDER", "memory"), (name, bad)]);
        assert!(
            matches!(result.err(), Some(ConfigError::Invalid { name: n, .. }) if n == name),
            "{name}={bad} must be refused"
        );
    }
}

#[test]
fn an_unknown_provider_names_the_ones_that_exist() {
    let error = read(&[("SAGAT_BLOB_PROVIDER", "gcs")]).expect_err("refused");

    assert!(matches!(
        error,
        ConfigError::Invalid {
            name: "SAGAT_BLOB_PROVIDER",
            ..
        }
    ));
    let message = error.to_string();
    for known in ["s3", "r2", "fs", "memory", "none"] {
        assert!(message.contains(known), "{message}");
    }
}

#[test]
fn the_part_size_has_a_floor_the_providers_impose() {
    assert!(PartSize::new(5 * 1024 * 1024 - 1).is_none());
    assert!(PartSize::new(5 * 1024 * 1024).is_some());
    assert_eq!(PartSize::default().bytes(), 8 * 1024 * 1024);
}
