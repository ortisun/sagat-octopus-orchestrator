//! Serving the Studio (D143): the files of a single-page app under `/studio/`, same origin as the
//! API, so the API needs no CORS.
//!
//! The directory is an operator's choice (`SAGAT_STUDIO_DIR`); what this module guarantees is what
//! a request can reach inside it and what the browser is told about it:
//!
//! * a path is made of plain segments: `..`, a backslash, a NUL byte or a hidden file name is not
//!   served, and a file is served only if its real path (symbolic links resolved) is inside the
//!   real directory;
//! * a path with no file extension that matches no file is a route of the app and gets the page;
//!   a missing *file* is a `404`, never the page (a script that fails to load must not be answered
//!   with HTML);
//! * a file is streamed, not read whole into memory; at most [`StudioLimits::max_transfers`] files
//!   are in flight at once (another request is told to come back: `503` with `Retry-After`), and a
//!   transfer that takes longer than [`StudioLimits::max_transfer`] is cut, so a client that opens
//!   many connections and reads nothing cannot hold the memory or the connections of the platform;
//! * every answer carries a content security policy that allows scripts, styles, images and
//!   connections from the page's own origin only (and the issuer's, when one is configured, for
//!   the sign-in), `nosniff`, no referrer and a same-origin opener policy; built assets are
//!   `immutable`, the page is `no-store`.

use std::io::{self, Read as _};
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Sleep;
use tokio_stream::Stream;
use tokio_util::io::ReaderStream;

/// The largest file served. The biggest thing in a build is an editor worker of a few megabytes.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// The page is read for every navigation so a rebuilt Studio does not retain stale asset names.
const MAX_INDEX_BYTES: u64 = 64 * 1024;

/// Where the Studio is and how it signs in.
#[derive(Debug, Clone)]
pub struct StudioConfig {
    /// The directory of the build (it holds `index.html`).
    pub dir: PathBuf,
    /// The issuer the browser signs in with, when the deployment has one.
    pub oidc: Option<StudioOidc>,
    /// How much of the platform the files may hold.
    pub limits: StudioLimits,
}

/// What serving files may cost. The files are public (they are what a browser needs before anyone
/// has signed in), so what keeps them from being a way to exhaust the platform is a bound, not a
/// credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StudioLimits {
    /// Files being sent at once. A page load asks for about twenty and the assets are cached for a
    /// year; a request over the limit is answered `503` with `Retry-After: 1`.
    pub max_transfers: usize,
    /// The longest one file may take to be read by the client.
    pub max_transfer: Duration,
    /// How long a request waits for a place before it is told to come back.
    pub wait: Duration,
}

impl Default for StudioLimits {
    fn default() -> Self {
        Self {
            max_transfers: 64,
            max_transfer: Duration::from_secs(60),
            wait: Duration::from_secs(2),
        }
    }
}

/// The public settings of the sign-in (nothing secret: a client id of a public client).
#[derive(Debug, Clone)]
pub struct StudioOidc {
    pub issuer: String,
    pub client_id: String,
    pub scope: String,
}

/// Why the Studio cannot be served.
#[derive(Debug, thiserror::Error)]
pub enum StudioError {
    #[error("the Studio directory {0} does not hold a readable index.html")]
    NoIndex(String),
    #[error("the OIDC issuer {0} is not an https URL (or an http URL on a loopback address)")]
    BadIssuer(String),
}

#[derive(Clone)]
struct Site {
    root: PathBuf,
    config: Vec<u8>,
    csp: HeaderValue,
    transfers: Arc<Semaphore>,
    max_transfer: Duration,
    wait: Duration,
}

/// The routes `/studio`, `/studio/` and `/studio/{*path}`.
///
/// # Errors
/// [`StudioError`] when the directory has no `index.html` or the issuer is not a URL the browser
/// may be told to call.
pub fn studio_router(config: StudioConfig) -> Result<Router, StudioError> {
    let root = std::fs::canonicalize(&config.dir)
        .map_err(|_| StudioError::NoIndex(config.dir.display().to_string()))?;
    validate_index(&root).map_err(|_| StudioError::NoIndex(config.dir.display().to_string()))?;
    let connect = match &config.oidc {
        Some(oidc) => Some(origin_of(&oidc.issuer)?),
        None => None,
    };
    let site = Site {
        root,
        config: serde_json::to_vec(&json!({
            "oidc": config.oidc.as_ref().map(|oidc| json!({
                "issuer": oidc.issuer,
                "clientId": oidc.client_id,
                "scope": oidc.scope,
            })),
        }))
        .unwrap_or_default(),
        csp: HeaderValue::from_str(&policy(connect.as_deref()))
            .map_err(|_| StudioError::BadIssuer(String::new()))?,
        transfers: Arc::new(Semaphore::new(config.limits.max_transfers.max(1))),
        max_transfer: config.limits.max_transfer,
        wait: config.limits.wait,
    };
    Ok(Router::new()
        .route("/studio", get(|| async { Redirect::permanent("/studio/") }))
        .route("/studio/", get(page).head(page))
        .route("/studio/config.json", get(settings).head(settings))
        .route("/studio/{*path}", get(file).head(file))
        .with_state(site))
}

/// The origin (`scheme://host[:port]`) of the issuer, which the page is allowed to call.
fn origin_of(issuer: &str) -> Result<String, StudioError> {
    let bad = || StudioError::BadIssuer(issuer.chars().take(64).collect());
    let (scheme, rest) = issuer.split_once("://").ok_or_else(bad)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty()
        || authority.contains('@')
        || authority
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b == b';' || b == b'\'')
    {
        return Err(bad());
    }
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, _)| host);
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    match scheme {
        "https" => {}
        "http" if loopback => {}
        _ => return Err(bad()),
    }
    Ok(format!("{scheme}://{authority}"))
}

/// The content security policy of every answer.
fn policy(issuer_origin: Option<&str>) -> String {
    let connect = issuer_origin.map_or_else(|| "'self'".to_owned(), |o| format!("'self' {o}"));
    format!(
        "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; font-src 'self'; connect-src {connect}; worker-src 'self'; \
         base-uri 'none'; form-action 'self'; frame-ancestors 'none'"
    )
}

fn hardened(site: &Site, mut response: Response, cache: &'static str) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, site.csp.clone());
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

fn typed(content_type: &'static str, body: Vec<u8>) -> Response {
    let mut response = body.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

async fn page(State(site): State<Site>, method: Method) -> Response {
    html(&site, &method).await
}

async fn html(site: &Site, method: &Method) -> Response {
    let Some(index) = read_index(&site.root).await else {
        let mut unavailable = StatusCode::SERVICE_UNAVAILABLE.into_response();
        unavailable
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        return hardened(site, unavailable, "no-store");
    };
    let length = index.len() as u64;
    let body = if method == Method::HEAD {
        Vec::new()
    } else {
        index
    };
    let mut response = typed("text/html; charset=utf-8", body);
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    hardened(site, response, "no-store")
}

async fn settings(State(site): State<Site>, method: Method) -> Response {
    let body = if method == Method::HEAD {
        Vec::new()
    } else {
        site.config.clone()
    };
    hardened(&site, typed("application/json", body), "no-store")
}

async fn file(
    State(site): State<Site>,
    method: Method,
    UrlPath(path): UrlPath<String>,
) -> Response {
    let Some(relative) = plain_path(&path) else {
        return hardened(&site, StatusCode::NOT_FOUND.into_response(), "no-store");
    };
    if relative == Path::new("index.html") {
        return html(&site, &method).await;
    }
    if let Some((real, length)) = locate(&site.root, &relative).await {
        // A place among the transfers, waited for a moment (a page asks for twenty files at once and a
        // browser does not ask again for a script that failed), then a refusal that says when to come back.
        let permit = match tokio::time::timeout(
            site.wait,
            Arc::clone(&site.transfers).acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            _ => {
                let mut busy = StatusCode::SERVICE_UNAVAILABLE.into_response();
                busy.headers_mut()
                    .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
                return hardened(&site, busy, "no-store");
            }
        };
        let Ok(file) = tokio::fs::File::open(&real).await else {
            return hardened(&site, StatusCode::NOT_FOUND.into_response(), "no-store");
        };
        let kind = content_type(&relative);
        let cache = if relative.starts_with("assets") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        // A `HEAD` is answered with the headers of the `GET`, length included, and no body.
        let body = if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from_stream(Transfer {
                inner: ReaderStream::with_capacity(file, 64 * 1024),
                deadline: Box::pin(tokio::time::sleep(site.max_transfer)),
                _permit: permit,
            })
        };
        let mut response = Response::new(body);
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
        return hardened(&site, response, cache);
    }
    // A route of the app has no extension; a missing file does, and is not the page.
    if relative.extension().is_none() {
        return html(&site, &method).await;
    }
    hardened(&site, StatusCode::NOT_FOUND.into_response(), "no-store")
}

/// Verify at startup that the index is a readable, regular file inside the configured directory.
/// Its bytes are deliberately discarded: requests read the current build, not this snapshot.
fn validate_index(root: &Path) -> io::Result<()> {
    let path = root.join("index.html");
    let metadata = std::fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_INDEX_BYTES
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unsafe index"));
    }
    let real = std::fs::canonicalize(path)?;
    if !real.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "index outside root",
        ));
    }
    let mut contents = Vec::with_capacity(metadata.len() as usize);
    std::fs::File::open(real)?
        .take(MAX_INDEX_BYTES + 1)
        .read_to_end(&mut contents)?;
    if contents.len() as u64 > MAX_INDEX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "oversized index",
        ));
    }
    Ok(())
}

/// Read the current page with a hard cap. A failed, missing, replaced or unsafe index never falls
/// back to the startup copy because that copy may reference assets that no longer exist.
async fn read_index(root: &Path) -> Option<Vec<u8>> {
    let path = root.join("index.html");
    let metadata = tokio::fs::symlink_metadata(&path).await.ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_INDEX_BYTES
    {
        return None;
    }
    let real = tokio::fs::canonicalize(path).await.ok()?;
    if !real.starts_with(root) {
        return None;
    }
    let file = tokio::fs::File::open(real).await.ok()?;
    let mut contents = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_INDEX_BYTES + 1)
        .read_to_end(&mut contents)
        .await
        .ok()?;
    (contents.len() as u64 <= MAX_INDEX_BYTES).then_some(contents)
}

/// The body of a file: it holds its place among the transfers in flight until it is dropped, and it
/// ends in an error (so the connection is closed) when the client has not taken the file in time.
struct Transfer<S> {
    inner: S,
    deadline: Pin<Box<Sleep>>,
    _permit: OwnedSemaphorePermit,
}

impl<S: Stream<Item = io::Result<Bytes>> + Unpin> Stream for Transfer<S> {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.deadline.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Some(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the client did not take the file in time",
            ))));
        }
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

/// A relative path of plain, visible segments, or `None`.
fn plain_path(url_path: &str) -> Option<PathBuf> {
    if url_path.is_empty() || url_path.contains(['\\', '\0']) {
        return None;
    }
    let mut out = PathBuf::new();
    for component in Path::new(url_path).components() {
        match component {
            Component::Normal(segment) => {
                let name = segment.to_str()?;
                if name.starts_with('.') {
                    return None;
                }
                out.push(name);
            }
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// The real path of the file `relative` under `root` and its length, if it is a regular file (not a
/// pipe, which opening could block on) whose real path is inside `root` (a link out of the directory
/// is not followed).
async fn locate(root: &Path, relative: &Path) -> Option<(PathBuf, u64)> {
    let real = tokio::fs::canonicalize(root.join(relative)).await.ok()?;
    if !real.starts_with(root) {
        return None;
    }
    let metadata = tokio::fs::metadata(&real).await.ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    Some((real, metadata.len()))
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("ico") => "image/x-icon",
        Some("webp") => "image/webp",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}
