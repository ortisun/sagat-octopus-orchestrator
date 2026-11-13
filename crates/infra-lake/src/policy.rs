//! What the sandbox may reach, derived from the pinned connection (feature 035, T035-035, D035-049, D035-114).
//!
//! The relay allows a name only if the pinned connection declares it or it is an official regional endpoint of the
//! connection's storage ([`EndpointPolicy`]). The driver builds that allowance here from the documents the platform stored
//! (`crates/infra-postgres/src/database/iceberg.rs`): the catalog's address, the token endpoint of OAuth2, an
//! S3-compatible store's address, or S3's regional endpoints for the region and the buckets of the warehouse locations.
//!
//! It is closed on purpose. The text of a URL becomes a list of hosts here, so a URL that could name another host than it
//! seems to (user information, a query, a fragment, a backslash, whitespace, a port that is not a number, an address without
//! its brackets) is refused, and so is every connection the host cannot open yet (a Glue, Hive or JDBC catalog, GCS or ADLS,
//! an assumed role, remote signing, SigV4: `runners/lake/sagat_lake/connection.py`, G035-048): an allowance for a connection
//! that cannot run is refused with a name before a container is started. The relay's network policy then decides which
//! addresses a name may resolve to; this module says nothing about that.
//!
//! `http` is for the catalog on the loopback alone, the way the host reads it. A token endpoint and an S3 endpoint are
//! `https`, **except for a development runner**, which also accepts them over `http` on the loopback: it is what the server
//! already lets a development namespace save (`requires_development`), and the host reads it the same way when the request
//! line says it is a development one (O035-010, option b). The flag is the runner's own and never a member of the connection.

use std::collections::BTreeSet;
use std::net::IpAddr;

use serde_json::Value;

use crate::relay::{Endpoint, EndpointPolicy};

const MAX_URL_BYTES: usize = 2048;
const MAX_LOCATIONS: usize = 16;
const HTTPS: u16 = 443;
const HTTP: u16 = 80;

/// Why a connection yields no allowance. A closed kind: no value of the document is repeated, because it may be a secret
/// pasted where a URL was meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("the connection is not the document the platform stores")]
    Shape,
    #[error("an address of the connection is not one the sandbox may be allowed to reach")]
    Url,
    #[error("the storage of the connection names a region the platform does not know")]
    Storage,
    #[error("a warehouse location is not an S3 location of a valid bucket")]
    Locations,
    #[error("the host cannot open this kind of connection yet")]
    Unsupported,
}

fn text<'a>(document: &'a Value, member: &str) -> Option<&'a str> {
    document.get(member).and_then(Value::as_str)
}

/// The endpoint a URL names. `loopback_http` lets the plain `http` scheme through, for a loopback host only.
fn endpoint_of(url: &str, loopback_http: bool) -> Result<Endpoint, PolicyError> {
    let (host, port) = address_of(url, loopback_http)?;
    Endpoint::new(&host, port).map_err(|_| PolicyError::Url)
}

/// Whether the last label of a host is a number (`2130706433`, `127.1`, `0x7f.1`): the URL standard reads such a host as an
/// IPv4 address in a form this parser does not, and a resolver may too. A canonical dotted quad is an address and is fine.
fn ends_in_a_number(host: &str) -> bool {
    let last = host
        .trim_end_matches('.')
        .rsplit('.')
        .next()
        .unwrap_or_default();
    let hex = last
        .strip_prefix("0x")
        .or_else(|| last.strip_prefix("0X"))
        .is_some_and(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()));
    !last.is_empty() && (last.bytes().all(|byte| byte.is_ascii_digit()) || hex)
}

/// The host and the port a URL names, checked: see the module. `loopback_http` lets `http` through for a loopback host.
fn address_of(url: &str, loopback_http: bool) -> Result<(String, u16), PolicyError> {
    if url.len() > MAX_URL_BYTES
        || !url.is_ascii()
        || url.bytes().any(|byte| {
            byte <= b' ' || byte == 0x7f || byte == b'\\' || byte == b'?' || byte == b'#'
        })
    {
        return Err(PolicyError::Url);
    }
    let (scheme, rest) = url.split_once("://").ok_or(PolicyError::Url)?;
    let default = match scheme.to_ascii_lowercase().as_str() {
        "https" => HTTPS,
        "http" if loopback_http => HTTP,
        _ => return Err(PolicyError::Url),
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(PolicyError::Url);
    }
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (inner, after) = bracketed.split_once(']').ok_or(PolicyError::Url)?;
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':').ok_or(PolicyError::Url)?),
        };
        (format!("[{inner}]"), port)
    } else {
        match authority.rsplit_once(':') {
            // a colon in what is left is an IPv6 address without its brackets, which a client reads differently
            Some((host, _)) if host.contains(':') => return Err(PolicyError::Url),
            Some((host, port)) => (host.to_owned(), Some(port)),
            None => (authority.to_owned(), None),
        }
    };
    let port = match port {
        None => default,
        Some(digits) => {
            if digits.is_empty()
                || digits.len() > 5
                || !digits.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(PolicyError::Url);
            }
            digits.parse::<u16>().map_err(|_| PolicyError::Url)?
        }
    };
    Endpoint::new(&host, port).map_err(|_| PolicyError::Url)?;
    if scheme.eq_ignore_ascii_case("http") && !is_loopback(&host) {
        return Err(PolicyError::Url);
    }
    if !host.starts_with('[') && host.parse::<IpAddr>().is_err() && ends_in_a_number(&host) {
        return Err(PolicyError::Url);
    }
    Ok((host, port))
}

fn is_loopback(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// The endpoints the catalog declares: its address and, for OAuth2, its token endpoint (over `http` on the loopback only for a
/// development runner).
fn catalog_endpoints(catalog: &Value, development: bool) -> Result<Vec<Endpoint>, PolicyError> {
    match text(catalog, "type") {
        Some("rest") => {}
        Some("glue" | "hive" | "jdbc") => return Err(PolicyError::Unsupported),
        _ => return Err(PolicyError::Shape),
    }
    let rest = catalog
        .get("rest")
        .filter(|rest| rest.is_object())
        .ok_or(PolicyError::Shape)?;
    let uri = text(rest, "uri").ok_or(PolicyError::Shape)?;
    if rest.get("remoteSigning").and_then(Value::as_bool) == Some(true) {
        return Err(PolicyError::Unsupported);
    }
    let mut declared = vec![endpoint_of(uri, true)?];
    if let Some(auth) = rest.get("auth") {
        match text(auth, "type") {
            Some("bearer") => {}
            Some("oauth2") => {
                let token = text(auth, "tokenUri").ok_or(PolicyError::Shape)?;
                declared.push(endpoint_of(token, development)?);
            }
            Some("sigv4") => return Err(PolicyError::Unsupported),
            _ => return Err(PolicyError::Shape),
        }
    }
    Ok(declared)
}

/// The buckets the warehouse locations name, each once.
fn buckets(locations: &[String]) -> Result<BTreeSet<String>, PolicyError> {
    if locations.is_empty() || locations.len() > MAX_LOCATIONS {
        return Err(PolicyError::Locations);
    }
    let mut named = BTreeSet::new();
    for location in locations {
        let rest = location
            .strip_prefix("s3://")
            .ok_or(PolicyError::Locations)?;
        named.insert(rest.split('/').next().unwrap_or_default().to_owned());
    }
    Ok(named)
}

/// The allowance of the sandbox for a connection: `catalog` and `storage` are the documents the platform stored, and
/// `locations` the warehouse locations. `development` is the runner's: only a development runner lets a token endpoint and an
/// S3 endpoint be `http` (on the loopback, as the catalog is everywhere).
///
/// # Errors
/// [`PolicyError`] for a document that is not the stored shape, an address the sandbox must not be allowed to reach, a
/// region or a bucket that is not one, and a kind of connection the host cannot open yet.
pub fn endpoint_policy(
    catalog: &Value,
    storage: &Value,
    locations: &[String],
    development: bool,
) -> Result<EndpointPolicy, PolicyError> {
    let mut declared = catalog_endpoints(catalog, development)?;

    match text(storage, "type") {
        Some("s3") => {}
        Some("gcs" | "adls") => return Err(PolicyError::Unsupported),
        _ => return Err(PolicyError::Shape),
    }
    let s3 = storage
        .get("s3")
        .filter(|s3| s3.is_object())
        .ok_or(PolicyError::Shape)?;
    if s3.get("roleArn").is_some() || s3.get("externalId").is_some() {
        return Err(PolicyError::Unsupported);
    }
    let region = text(s3, "region").ok_or(PolicyError::Shape)?;
    let buckets = buckets(locations)?;
    let names: Vec<&str> = buckets.iter().map(String::as_str).collect();

    // The region and the buckets are checked whether or not S3's own hosts are used: the host sets the region on its client
    // and reads the locations either way. The relay's constructor is the one place that knows what a region and a bucket are.
    EndpointPolicy::default()
        .with_s3_regional(region, &[])
        .map_err(|_| PolicyError::Storage)?;
    EndpointPolicy::default()
        .with_s3_regional("us-east-1", &names)
        .map_err(|_| PolicyError::Locations)?;

    if let Some(endpoint) = s3.get("endpoint") {
        // An S3-compatible store: its address is the allowance, and the regional hosts of AWS are not added beside it.
        let (host, port) = address_of(endpoint.as_str().ok_or(PolicyError::Shape)?, development)?;
        declared.push(Endpoint::new(&host, port).map_err(|_| PolicyError::Url)?);
        // With `pathStyle: false` the host asks its client for virtual addressing, and the client then connects to
        // `<bucket>.<endpoint host>` (measured with PyArrow: `Host: mybucket.localhost:PORT`). Each bucket of the warehouse
        // is therefore allowed under the store's own name, and no other. An address has no such name, and a bucket with a
        // dot is reached by path, as it is on S3.
        let path_style = s3
            .get("pathStyle")
            .and_then(Value::as_bool)
            .ok_or(PolicyError::Shape)?;
        if !path_style && !host.starts_with('[') && host.parse::<IpAddr>().is_err() {
            let store = host.trim_end_matches('.').to_ascii_lowercase();
            for bucket in names.iter().filter(|bucket| !bucket.contains('.')) {
                declared.push(
                    Endpoint::new(&format!("{bucket}.{store}"), port)
                        .map_err(|_| PolicyError::Url)?,
                );
            }
        }
        return Ok(EndpointPolicy::new(declared));
    }
    EndpointPolicy::new(declared)
        .with_s3_regional(region, &names)
        .map_err(|_| PolicyError::Storage)
}
