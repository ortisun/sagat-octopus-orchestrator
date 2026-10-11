//! Serving the Studio's static files (D143): a single-page app under `/studio/`, same origin as
//! the API, with the headers that keep a script from anywhere else out, and no way to read a file
//! outside the directory it was given.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use sagat_api_http::studio::{StudioConfig, StudioLimits, StudioOidc, studio_router};
use sagat_api_http::{ServerConfig, serve};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A directory shaped like a Vite build, and a secret next to it that must stay unreachable.
fn site() -> (tempdir::Dir, PathBuf) {
    let dir = tempdir::Dir::new();
    let root = dir.path().join("dist");
    std::fs::create_dir_all(root.join("assets")).expect("assets");
    std::fs::write(
        root.join("index.html"),
        "<!doctype html><title>Studio</title><div id=root></div>",
    )
    .expect("index");
    std::fs::write(root.join("assets/app.1a2b3c.js"), "console.log('studio')").expect("js");
    std::fs::write(root.join("assets/app.1a2b3c.css"), "body{margin:0}").expect("css");
    std::fs::write(root.join("assets/codicon.ttf"), [0u8, 1, 2]).expect("font");
    std::fs::write(
        root.join("favicon.svg"),
        "<svg xmlns='http://www.w3.org/2000/svg'/>",
    )
    .expect("svg");
    std::fs::write(root.join(".env"), "TOKEN=hunter2").expect("hidden");
    std::fs::write(dir.path().join("secret.txt"), "outside").expect("secret");
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.path().join("secret.txt"), root.join("assets/link.txt"))
        .expect("link");
    (dir, root)
}

mod tempdir {
    use std::path::{Path, PathBuf};

    pub struct Dir(PathBuf);
    impl Dir {
        pub fn new() -> Self {
            let path = std::env::temp_dir().join(format!("studio-{}", uuid::Uuid::now_v7()));
            std::fs::create_dir_all(&path).expect("dir");
            Self(path)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

async fn serve_studio(root: &Path, oidc: Option<StudioOidc>) -> String {
    serve_studio_with(root, oidc, StudioLimits::default()).await
}

async fn serve_studio_with(root: &Path, oidc: Option<StudioOidc>, limits: StudioLimits) -> String {
    let app = studio_router(StudioConfig {
        dir: root.to_path_buf(),
        oidc,
        limits,
    })
    .expect("a directory with an index.html");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(serve(
        listener,
        app,
        ServerConfig::default(),
        std::future::pending(),
    ));
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    sagat_api_http::auth::install_crypto_provider();
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client")
}

fn header<'a>(response: &'a reqwest::Response, name: &str) -> &'a str {
    response
        .headers()
        .get(name)
        .unwrap_or_else(|| panic!("no {name} in {:?}", response.headers()))
        .to_str()
        .expect("ascii")
}

#[tokio::test]
async fn the_app_is_served_with_the_headers_that_keep_foreign_scripts_out() {
    let (_dir, root) = site();
    let base = serve_studio(&root, None).await;
    let response = client()
        .get(format!("{base}/studio/"))
        .send()
        .await
        .expect("get");
    assert_eq!(response.status(), 200);
    assert_eq!(
        header(&response, "content-type"),
        "text/html; charset=utf-8"
    );
    assert_eq!(header(&response, "cache-control"), "no-store");
    assert_eq!(header(&response, "x-content-type-options"), "nosniff");
    assert_eq!(header(&response, "referrer-policy"), "no-referrer");
    assert_eq!(
        header(&response, "cross-origin-opener-policy"),
        "same-origin"
    );
    let csp = header(&response, "content-security-policy").to_owned();
    for directive in [
        "default-src 'none'",
        "script-src 'self'",
        "connect-src 'self'",
        "worker-src 'self';",
        "base-uri 'none'",
        "frame-ancestors 'none'",
        "form-action 'self'",
    ] {
        assert!(csp.contains(directive), "{directive} in {csp}");
    }
    assert!(
        !csp.contains("script-src 'self' 'unsafe"),
        "no inline or eval script: {csp}"
    );
    assert!(!csp.contains("unsafe-eval"), "{csp}");
    assert!(
        response
            .text()
            .await
            .expect("body")
            .contains("<div id=root>")
    );
}

#[tokio::test]
async fn built_assets_are_typed_and_cached_for_good_and_the_page_never_is() {
    let (_dir, root) = site();
    let base = serve_studio(&root, None).await;
    let js = client()
        .get(format!("{base}/studio/assets/app.1a2b3c.js"))
        .send()
        .await
        .expect("js");
    assert_eq!(js.status(), 200);
    assert_eq!(
        header(&js, "content-type"),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        header(&js, "cache-control"),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(header(&js, "x-content-type-options"), "nosniff");
    let css = client()
        .get(format!("{base}/studio/assets/app.1a2b3c.css"))
        .send()
        .await
        .expect("css");
    assert_eq!(header(&css, "content-type"), "text/css; charset=utf-8");
    let font = client()
        .get(format!("{base}/studio/assets/codicon.ttf"))
        .send()
        .await
        .expect("font");
    assert_eq!(header(&font, "content-type"), "font/ttf");
    let svg = client()
        .get(format!("{base}/studio/favicon.svg"))
        .send()
        .await
        .expect("svg");
    assert_eq!(header(&svg, "content-type"), "image/svg+xml");
    assert_eq!(header(&svg, "cache-control"), "no-cache");
    // The default CSP of an SVG opened on its own: no script, whatever is inside it.
    assert!(header(&svg, "content-security-policy").contains("default-src 'none'"));
}

#[tokio::test]
async fn a_route_of_the_app_gets_the_page_and_a_missing_file_gets_a_404_not_the_page() {
    let (_dir, root) = site();
    let base = serve_studio(&root, None).await;
    for route in [
        "workflows",
        "workflows/019b5d6e-0000-7000-8000-000000000000/edit",
        "runs/abc",
    ] {
        let response = client()
            .get(format!("{base}/studio/{route}"))
            .send()
            .await
            .expect("get");
        assert_eq!(response.status(), 200, "{route}");
        assert_eq!(
            header(&response, "content-type"),
            "text/html; charset=utf-8"
        );
        assert_eq!(header(&response, "cache-control"), "no-store");
    }
    for file in [
        "assets/missing.js",
        "nothing.png",
        "assets/app.1a2b3c.js.map",
    ] {
        let response = client()
            .get(format!("{base}/studio/{file}"))
            .send()
            .await
            .expect("get");
        assert_eq!(response.status(), 404, "{file}");
        assert!(
            !response
                .text()
                .await
                .expect("body")
                .contains("<div id=root>"),
            "{file}"
        );
    }
}

#[tokio::test]
async fn nothing_outside_the_directory_nor_any_hidden_file_can_be_read() {
    let (_dir, root) = site();
    let base = serve_studio(&root, None).await;
    for path in [
        "../secret.txt",
        "..%2fsecret.txt",
        "%2e%2e/secret.txt",
        "assets/../../secret.txt",
        "assets/%2e%2e/%2e%2e/secret.txt",
        "..\\secret.txt",
        ".env",
        "assets/.env",
        "assets/link.txt",
        "%00",
        "assets/app.1a2b3c.js%00.png",
    ] {
        let response = client()
            .get(format!("{base}/studio/{path}"))
            .send()
            .await
            .expect("get");
        let status = response.status().as_u16();
        let body = response.text().await.expect("body");
        assert!(
            [400, 404].contains(&status) || (status == 200 && body.contains("<div id=root>")),
            "{path}: {status}"
        );
        assert!(
            !body.contains("outside") && !body.contains("hunter2"),
            "{path} leaked: {body}"
        );
    }
}

#[tokio::test]
async fn only_reads_are_served_and_the_folder_form_redirects() {
    let (_dir, root) = site();
    let base = serve_studio(&root, None).await;
    for method in [
        reqwest::Method::POST,
        reqwest::Method::PUT,
        reqwest::Method::DELETE,
        reqwest::Method::PATCH,
    ] {
        let response = client()
            .request(method.clone(), format!("{base}/studio/"))
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), 405, "{method}");
    }
    let head = client()
        .head(format!("{base}/studio/"))
        .send()
        .await
        .expect("head");
    assert_eq!(head.status(), 200);
    assert_eq!(header(&head, "content-type"), "text/html; charset=utf-8");
    let redirect = client()
        .get(format!("{base}/studio"))
        .send()
        .await
        .expect("get");
    assert_eq!(redirect.status(), 308);
    assert_eq!(header(&redirect, "location"), "/studio/");
}

#[tokio::test]
async fn the_config_says_where_to_sign_in_and_allows_only_that_origin_to_be_called() {
    let (_dir, root) = site();
    let plain = serve_studio(&root, None).await;
    let config = client()
        .get(format!("{plain}/studio/config.json"))
        .send()
        .await
        .expect("get");
    assert_eq!(config.status(), 200);
    assert_eq!(header(&config, "content-type"), "application/json");
    assert_eq!(header(&config, "cache-control"), "no-store");
    let body: serde_json::Value =
        serde_json::from_slice(&config.bytes().await.expect("body")).expect("json");
    assert_eq!(body, serde_json::json!({"oidc": null}));

    let with = serve_studio(
        &root,
        Some(StudioOidc {
            issuer: "https://login.example.test/realms/acme".into(),
            client_id: "studio".into(),
            scope: "openid profile".into(),
        }),
    )
    .await;
    let config = client()
        .get(format!("{with}/studio/config.json"))
        .send()
        .await
        .expect("get");
    let body: serde_json::Value =
        serde_json::from_slice(&config.bytes().await.expect("body")).expect("json");
    assert_eq!(
        body,
        serde_json::json!({"oidc": {
            "issuer": "https://login.example.test/realms/acme",
            "clientId": "studio",
            "scope": "openid profile"
        }})
    );
    let page = client()
        .get(format!("{with}/studio/"))
        .send()
        .await
        .expect("get");
    let csp = header(&page, "content-security-policy").to_owned();
    assert!(
        csp.contains("connect-src 'self' https://login.example.test"),
        "the page may call its own origin and the issuer's, nothing else: {csp}"
    );
}

#[tokio::test]
async fn a_directory_without_an_index_is_refused_at_start_up() {
    let dir = tempdir::Dir::new();
    let refused = studio_router(StudioConfig {
        dir: dir.path().to_path_buf(),
        oidc: None,
        limits: StudioLimits::default(),
    });
    assert!(refused.is_err());
    let missing = studio_router(StudioConfig {
        dir: dir.path().join("nope"),
        oidc: None,
        limits: StudioLimits::default(),
    });
    assert!(missing.is_err());
}

// ------------------------------------------------------------- what serving files may cost

/// A request written by hand, as a hostile client writes it: nothing tidies the path.
async fn raw(addr: &str, request_target: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(
            format!("GET {request_target} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .expect("request");
    let mut answer = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut answer)).await;
    String::from_utf8_lossy(&answer).into_owned()
}

fn authority(base: &str) -> String {
    base.trim_start_matches("http://").to_owned()
}

#[tokio::test]
async fn dot_segments_a_client_library_would_tidy_away_still_get_nothing_when_sent_as_they_are() {
    let (_dir, root) = site();
    let base = serve_studio(&root, None).await;
    let addr = authority(&base);
    for target in [
        "/studio/assets/../../secret.txt",
        "/studio/../secret.txt",
        "/studio/assets/%2e%2e/%2e%2e/secret.txt",
        "/studio/assets/..%2f..%2fsecret.txt",
        "/studio/assets/%2e%2e%5c%2e%2e%5csecret.txt",
        "/studio/assets/..\\..\\secret.txt",
        "/studio/assets/link.txt",
        "/studio/.env",
        "/studio/assets/%00.js",
    ] {
        let answer = raw(&addr, target).await;
        assert!(
            !answer.contains("outside") && !answer.contains("hunter2"),
            "{target} reached a file that is not the Studio's: {answer}"
        );
        assert!(
            !answer.starts_with("HTTP/1.1 200"),
            "{target} was answered 200: {answer}"
        );
    }
}

#[tokio::test]
async fn a_file_is_sent_with_its_length_and_a_head_says_the_same_without_the_body() {
    let (_dir, root) = site();
    let base = serve_studio(&root, None).await;
    let url = format!("{base}/studio/assets/app.1a2b3c.js");
    let get = client().get(&url).send().await.expect("get");
    assert_eq!(header(&get, "content-length"), "21");
    assert_eq!(get.text().await.expect("body"), "console.log('studio')");
    let head = client().head(&url).send().await.expect("head");
    assert_eq!(head.status(), 200);
    assert_eq!(
        header(&head, "content-length"),
        "21",
        "a HEAD reports the length of the file"
    );
    assert_eq!(
        header(&head, "content-type"),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(head.bytes().await.expect("no body").len(), 0);
}

/// A file the kernel's buffers cannot swallow in one go.
fn big_file(root: &Path) {
    let file = std::fs::File::create(root.join("assets/big.1a2b3c.js")).expect("big");
    file.set_len(24 * 1024 * 1024).expect("sparse");
}

#[tokio::test]
async fn clients_that_read_nothing_cannot_hold_more_than_the_transfers_the_server_allows() {
    let (_dir, root) = site();
    big_file(&root);
    let limits = StudioLimits {
        max_transfers: 2,
        max_transfer: Duration::from_secs(60),
        wait: Duration::from_millis(300),
    };
    let base = serve_studio_with(&root, None, limits).await;
    let addr = authority(&base);

    // Two clients ask for the big file and read none of it: both transfers are held.
    let mut holders = Vec::new();
    for _ in 0..2 {
        let mut stream = TcpStream::connect(&addr).await.expect("connect");
        stream
            .write_all(b"GET /studio/assets/big.1a2b3c.js HTTP/1.1\r\nHost: test\r\n\r\n")
            .await
            .expect("request");
        holders.push(stream);
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    // A third is told to come back, with the time to wait, instead of being queued behind them.
    let refused = client()
        .get(format!("{base}/studio/assets/app.1a2b3c.js"))
        .send()
        .await
        .expect("third");
    assert_eq!(refused.status(), 503);
    assert_eq!(header(&refused, "retry-after"), "1");
    assert_eq!(header(&refused, "cache-control"), "no-store");
    // The page and the config are small and held in memory: they are not turned away.
    let page = client()
        .get(format!("{base}/studio/"))
        .send()
        .await
        .expect("page");
    assert_eq!(page.status(), 200);

    // When the holders go, the places are free again.
    drop(holders);
    let mut served = false;
    for _ in 0..50 {
        let again = client()
            .get(format!("{base}/studio/assets/app.1a2b3c.js"))
            .send()
            .await
            .expect("again");
        if again.status() == 200 {
            served = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        served,
        "the places of the clients that left were never freed"
    );
}

#[tokio::test]
async fn a_client_that_takes_a_file_a_few_bytes_at_a_time_is_cut_after_the_longest_transfer() {
    let (_dir, root) = site();
    big_file(&root);
    let limits = StudioLimits {
        max_transfers: 4,
        max_transfer: Duration::from_secs(1),
        wait: Duration::from_millis(300),
    };
    let base = serve_studio_with(&root, None, limits).await;
    let mut slow = TcpStream::connect(authority(&base)).await.expect("connect");
    slow.write_all(b"GET /studio/assets/big.1a2b3c.js HTTP/1.1\r\nHost: test\r\n\r\n")
        .await
        .expect("request");
    let mut total = 0usize;
    let mut chunk = vec![0u8; 16 * 1024];
    // For longer than the longest transfer the client takes the file in drops (it is making progress, so the
    // server's guard against a client that takes nothing does not apply)...
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_millis(2500) {
        if let Ok(Ok(n)) = tokio::time::timeout(Duration::from_secs(5), slow.read(&mut chunk)).await
        {
            total += n;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    // ...and then takes what is left as fast as it comes: the server cut the transfer, so the connection ends
    // before the file does. Without the cut the whole of it arrives.
    let mut ended = false;
    let mut fast = vec![0u8; 256 * 1024];
    let drain_until = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < drain_until {
        match tokio::time::timeout(Duration::from_secs(5), slow.read(&mut fast)).await {
            Ok(Ok(0) | Err(_)) => {
                ended = true;
                break;
            }
            Ok(Ok(n)) => total += n,
            Err(_) => break,
        }
    }
    assert!(
        ended,
        "the connection of a client that took the file in drops was never closed"
    );
    assert!(
        total < 24 * 1024 * 1024,
        "a client that took the file in drops was served all of it ({total} bytes)"
    );
}
