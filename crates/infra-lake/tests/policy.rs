//! What the sandbox may reach, derived from the pinned connection (feature 035, T035-035, D035-049, D035-114).
//!
//! The relay allows a name only if the connection declared it, or if it is an official regional endpoint of the
//! connection's storage. The driver builds that allowance from the connection the platform stored, so this is the place
//! where a hostile or careless document becomes a list of hosts. It is closed: a connection the host cannot open, a URL that
//! could name another host than it seems to, a bucket that is not a bucket, all fail here, before a container exists.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use sagat_infra_lake::policy::{PolicyError, endpoint_policy};
use sagat_infra_lake::protocol::Connection;
use sagat_infra_lake::relay::EndpointPolicy;
use serde_json::{Value, json};

fn rest(uri: &str) -> Value {
    json!({
        "type": "rest",
        "rest": {
            "uri": uri,
            "warehouse": "s3://lake/warehouse/",
            "auth": {"type": "bearer"},
            "accessDelegation": "none",
            "remoteSigning": false
        }
    })
}

fn s3(region: &str) -> Value {
    json!({"type": "s3", "s3": {"region": region, "pathStyle": false}, "credentials": "static"})
}

fn policy(
    catalog: &Value,
    storage: &Value,
    locations: &[&str],
) -> Result<EndpointPolicy, PolicyError> {
    let locations: Vec<String> = locations.iter().map(|text| (*text).to_owned()).collect();
    endpoint_policy(catalog, storage, &locations)
}

fn lake() -> Vec<&'static str> {
    vec!["s3://lake/warehouse/"]
}

// -- the catalog ------------------------------------------------------------------------------------------------------

#[test]
fn a_catalog_over_https_is_declared_on_its_port_and_nothing_else_of_its_host() {
    let allowed = policy(
        &rest("https://catalog.example/api/v1"),
        &s3("eu-west-1"),
        &lake(),
    )
    .unwrap();
    assert!(allowed.allows("catalog.example", 443));
    assert!(!allowed.allows("catalog.example", 80));
    assert!(!allowed.allows("catalog.example", 8443));
    assert!(!allowed.allows("other.example", 443));
    // a sub-domain is another host
    assert!(!allowed.allows("evil.catalog.example", 443));
}

#[test]
fn a_catalog_on_another_port_is_declared_on_that_port() {
    let allowed = policy(
        &rest("https://catalog.example:8443/"),
        &s3("eu-west-1"),
        &lake(),
    )
    .unwrap();
    assert!(allowed.allows("catalog.example", 8443));
    assert!(!allowed.allows("catalog.example", 443));
}

#[test]
fn the_host_is_compared_as_the_relay_compares_it() {
    let allowed = policy(
        &rest("https://Catalog.EXAMPLE./x"),
        &s3("eu-west-1"),
        &lake(),
    )
    .unwrap();
    assert!(allowed.allows("catalog.example", 443));
}

#[test]
fn an_address_literal_is_declared_as_it_is_and_an_ipv6_one_in_brackets() {
    let allowed = policy(
        &rest("https://203.0.113.9:8443/"),
        &s3("eu-west-1"),
        &lake(),
    )
    .unwrap();
    assert!(allowed.allows("203.0.113.9", 8443));
    let allowed = policy(
        &rest("https://[2001:db8::1]:8443/"),
        &s3("eu-west-1"),
        &lake(),
    )
    .unwrap();
    assert!(allowed.allows("[2001:db8::1]", 8443));
}

#[test]
fn the_token_endpoint_of_oauth2_is_declared_with_the_catalog() {
    let catalog = json!({
        "type": "rest",
        "rest": {
            "uri": "https://catalog.example",
            "warehouse": "s3://lake/",
            "auth": {"type": "oauth2", "tokenUri": "https://auth.example:9443/token", "scope": "catalog"},
            "accessDelegation": "none",
            "remoteSigning": false
        }
    });
    let allowed = policy(&catalog, &s3("eu-west-1"), &lake()).unwrap();
    assert!(allowed.allows("catalog.example", 443));
    assert!(allowed.allows("auth.example", 9443));
    assert!(!allowed.allows("auth.example", 443));
}

#[test]
fn a_token_endpoint_that_is_not_https_is_refused_even_on_the_loopback() {
    let catalog = json!({
        "type": "rest",
        "rest": {
            "uri": "https://catalog.example",
            "warehouse": "s3://lake/",
            "auth": {"type": "oauth2", "tokenUri": "http://localhost:8080/token"},
            "accessDelegation": "none",
            "remoteSigning": false
        }
    });
    assert_eq!(
        policy(&catalog, &s3("eu-west-1"), &lake()).unwrap_err(),
        PolicyError::Url
    );
}

#[test]
fn plain_http_is_for_the_loopback_alone() {
    for good in [
        "http://localhost:8181",
        "http://127.0.0.1:8181/x",
        "http://[::1]:8181",
    ] {
        assert!(
            policy(&rest(good), &s3("eu-west-1"), &lake()).is_ok(),
            "{good}"
        );
    }
    for bad in [
        "http://catalog.example",
        "http://10.0.0.5:8181",
        "http://localhost.evil.example",
    ] {
        assert_eq!(
            policy(&rest(bad), &s3("eu-west-1"), &lake()).unwrap_err(),
            PolicyError::Url,
            "{bad}"
        );
    }
}

/// A URL that could name another host than the one it seems to, or carry something a host would act on, is refused: the
/// allowance is built from this text, and a parser that disagrees with the client's about it is how a check is bypassed.
#[test]
fn a_url_that_could_mean_another_host_is_refused() {
    for bad in [
        "",
        "catalog.example",
        "ftp://catalog.example",
        "https://",
        "https:///path",
        "https://user@catalog.example",
        "https://user:secret@catalog.example",
        "https://catalog.example@evil.example",
        "https://catalog.example?x=1",
        "https://catalog.example/path?x=1",
        "https://catalog.example#fragment",
        "https://catalog.example:abc",
        "https://catalog.example:0",
        "https://catalog.example:65536",
        "https://catalog.example:",
        "https://catalog .example",
        "https://catalog.example\n",
        "https://catalog.example\u{0}",
        "https://catalog\u{e9}.example",
        "https://catalog.example\\evil.example",
        "https://[2001:db8::1",
        "https://2001:db8::1:443/",
        " https://catalog.example",
        "HTTPS://catalog.example ",
    ] {
        assert_eq!(
            policy(&rest(bad), &s3("eu-west-1"), &lake()).unwrap_err(),
            PolicyError::Url,
            "{bad:?}"
        );
    }
    let long = format!("https://{}.example", "a".repeat(2100));
    assert_eq!(
        policy(&rest(&long), &s3("eu-west-1"), &lake()).unwrap_err(),
        PolicyError::Url
    );
}

// -- the storage ------------------------------------------------------------------------------------------------------

#[test]
fn s3_allows_the_official_regional_endpoints_of_its_region_and_its_buckets() {
    let allowed = policy(
        &rest("https://catalog.example"),
        &s3("eu-west-1"),
        &["s3://lake/warehouse/", "s3://archive/old/"],
    )
    .unwrap();
    for host in [
        "s3.eu-west-1.amazonaws.com",
        "s3.dualstack.eu-west-1.amazonaws.com",
        "lake.s3.eu-west-1.amazonaws.com",
        "archive.s3.eu-west-1.amazonaws.com",
    ] {
        assert!(allowed.allows(host, 443), "{host}");
    }
    for host in [
        "s3.us-east-1.amazonaws.com",
        "other.s3.eu-west-1.amazonaws.com",
        "s3.amazonaws.com",
        "evil.example",
    ] {
        assert!(!allowed.allows(host, 443), "{host}");
    }
    assert!(!allowed.allows("s3.eu-west-1.amazonaws.com", 80));
}

#[test]
fn a_bucket_with_a_dot_is_reached_path_style_only() {
    let allowed = policy(
        &rest("https://catalog.example"),
        &s3("eu-west-1"),
        &["s3://my.lake/warehouse/"],
    )
    .unwrap();
    assert!(allowed.allows("s3.eu-west-1.amazonaws.com", 443));
    assert!(!allowed.allows("my.lake.s3.eu-west-1.amazonaws.com", 443));
}

#[test]
fn the_locations_of_the_warehouse_name_the_buckets_once_each() {
    let allowed = policy(
        &rest("https://catalog.example"),
        &s3("eu-west-1"),
        &["s3://lake/a/", "s3://lake/b/", "s3://lake"],
    )
    .unwrap();
    assert!(allowed.allows("lake.s3.eu-west-1.amazonaws.com", 443));
}

#[test]
fn an_s3_compatible_store_is_declared_and_aws_is_not_added_beside_it() {
    let storage = json!({
        "type": "s3",
        "s3": {"region": "us-east-1", "endpoint": "https://minio.internal:9000", "pathStyle": true},
        "credentials": "static"
    });
    let allowed = policy(&rest("https://catalog.example"), &storage, &lake()).unwrap();
    assert!(allowed.allows("minio.internal", 9000));
    assert!(!allowed.allows("s3.us-east-1.amazonaws.com", 443));
    assert!(!allowed.allows("lake.s3.us-east-1.amazonaws.com", 443));
}

#[test]
fn an_s3_endpoint_over_plain_http_is_refused_whatever_the_host() {
    for endpoint in ["http://localhost:9000", "http://minio.internal:9000"] {
        let storage = json!({
            "type": "s3",
            "s3": {"region": "us-east-1", "endpoint": endpoint, "pathStyle": true},
            "credentials": "static"
        });
        assert_eq!(
            policy(&rest("https://catalog.example"), &storage, &lake()).unwrap_err(),
            PolicyError::Url,
            "{endpoint}"
        );
    }
}

#[test]
fn a_region_or_a_bucket_that_is_not_one_is_refused() {
    for region in [
        "",
        "EU-WEST-1",
        "eu west 1",
        "../x",
        "eu-west-1.evil.example",
    ] {
        assert_eq!(
            policy(&rest("https://catalog.example"), &s3(region), &lake()).unwrap_err(),
            PolicyError::Storage,
            "{region:?}"
        );
    }
    for location in [
        "s3://",
        "s3:///key",
        "s3://UPPER/key",
        "s3://a/key",
        "s3://bucket..name/key",
        "s3://-bucket/key",
        "s3://lake@evil.example/key",
        "s3://lake:80/key",
    ] {
        assert_eq!(
            policy(
                &rest("https://catalog.example"),
                &s3("eu-west-1"),
                &[location]
            )
            .unwrap_err(),
            PolicyError::Locations,
            "{location:?}"
        );
    }
}

#[test]
fn a_warehouse_with_no_location_or_one_that_is_not_s3_is_refused() {
    assert_eq!(
        policy(&rest("https://catalog.example"), &s3("eu-west-1"), &[]).unwrap_err(),
        PolicyError::Locations
    );
    for location in [
        "gs://lake/x",
        "abfss://c@a.dfs.core.windows.net/x",
        "file:///tmp/x",
        "lake/x",
    ] {
        assert_eq!(
            policy(
                &rest("https://catalog.example"),
                &s3("eu-west-1"),
                &[location]
            )
            .unwrap_err(),
            PolicyError::Locations,
            "{location}"
        );
    }
}

// -- what the host cannot open ------------------------------------------------------------------------------------------

/// The host serves a REST catalog over S3 and nothing else yet (`connection.py`, G035-048). The allowance of a connection it
/// cannot open would be an allowance for nothing, so it is refused here with a name, before a container is started.
#[test]
fn a_connection_the_host_cannot_open_is_refused_here() {
    let glue =
        json!({"type": "glue", "glue": {"region": "eu-west-1", "catalogId": "123456789012"}});
    let hive = json!({"type": "hive", "hive": {"uris": ["thrift://hms:9083"], "tls": "required"}});
    let jdbc = json!({"type": "jdbc", "jdbc": {"host": "db", "port": 5432, "database": "c", "engine": "postgresql"}});
    for catalog in [glue, hive, jdbc] {
        assert_eq!(
            policy(&catalog, &s3("eu-west-1"), &lake()).unwrap_err(),
            PolicyError::Unsupported
        );
    }
    let gcs = json!({"type": "gcs", "gcs": {}, "credentials": "static"});
    let adls = json!({"type": "adls", "adls": {"account": "acct"}, "credentials": "static"});
    for storage in [gcs, adls] {
        assert_eq!(
            policy(&rest("https://catalog.example"), &storage, &lake()).unwrap_err(),
            PolicyError::Unsupported
        );
    }
}

#[test]
fn an_assumed_role_remote_signing_and_sigv4_are_refused_here() {
    let role = json!({
        "type": "s3",
        "s3": {"region": "eu-west-1", "pathStyle": false, "roleArn": "arn:aws:iam::123456789012:role/lake"},
        "credentials": "static"
    });
    assert_eq!(
        policy(&rest("https://catalog.example"), &role, &lake()).unwrap_err(),
        PolicyError::Unsupported
    );
    let mut signing = rest("https://catalog.example");
    signing["rest"]["remoteSigning"] = json!(true);
    assert_eq!(
        policy(&signing, &s3("eu-west-1"), &lake()).unwrap_err(),
        PolicyError::Unsupported
    );
    let mut sigv4 = rest("https://catalog.example");
    sigv4["rest"]["auth"] =
        json!({"type": "sigv4", "region": "eu-west-1", "signingName": "execute-api"});
    assert_eq!(
        policy(&sigv4, &s3("eu-west-1"), &lake()).unwrap_err(),
        PolicyError::Unsupported
    );
}

#[test]
fn a_document_that_is_not_the_stored_shape_is_refused() {
    for catalog in [
        json!(null),
        json!([]),
        json!({"type": "rest"}),
        json!({"type": "rest", "rest": {}}),
        json!({"type": "rest", "rest": {"uri": 7}}),
        json!({"rest": {"uri": "https://catalog.example"}}),
    ] {
        assert!(
            policy(&catalog, &s3("eu-west-1"), &lake()).is_err(),
            "{catalog}"
        );
    }
    for storage in [
        json!(null),
        json!({"type": "s3"}),
        json!({"type": "s3", "s3": {}}),
    ] {
        assert!(
            policy(&rest("https://catalog.example"), &storage, &lake()).is_err(),
            "{storage}"
        );
    }
}

#[test]
fn no_error_repeats_what_it_was_given() {
    let secret = "https://user:hunter2@catalog.example";
    let error = policy(&rest(secret), &s3("eu-west-1"), &lake()).unwrap_err();
    let shown = format!("{error:?} {error}");
    assert!(!shown.contains("hunter2"), "{shown}");
}

// -- the connection of the wire fixture -------------------------------------------------------------------------------

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/contract/iceberg-writes/fixtures/requests/iceberg-write-append.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn the_connection_of_the_wire_fixture_allows_exactly_its_hosts() {
    let stored = fixture();
    let connection = Connection::new(
        stored["connection"]["catalog"].clone(),
        stored["connection"]["storage"].clone(),
        vec!["s3://lake/warehouse/".to_owned()],
        BTreeMap::new(),
        BTreeMap::new(),
    );
    let allowed = connection.endpoint_policy().unwrap();
    for (host, port) in [
        ("catalog.example", 443),
        ("auth.example", 443),
        ("s3.eu-west-1.amazonaws.com", 443),
        ("lake.s3.eu-west-1.amazonaws.com", 443),
    ] {
        assert!(allowed.allows(host, port), "{host}:{port}");
    }
    for (host, port) in [
        ("evil.example", 443),
        ("catalog.example", 80),
        ("169.254.169.254", 80),
        ("metadata.google.internal", 80),
    ] {
        assert!(!allowed.allows(host, port), "{host}:{port}");
    }
}
