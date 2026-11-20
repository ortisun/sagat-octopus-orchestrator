use super::Plan;
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::{
    Request, Response,
    body::{Frame, Incoming},
    service::service_fn,
};
use std::io;
type Body = UnsyncBoxBody<Bytes, io::Error>;
fn full(bytes: Vec<u8>) -> Body {
    Full::new(Bytes::from(bytes))
        .map_err(|never| match never {})
        .boxed_unsync()
}
async fn answer(
    request: Request<Incoming>,
    origin: reqwest::Url,
    plan: Plan,
    client: reqwest::Client,
) -> Result<Response<Body>, io::Error> {
    let relative = request
        .uri()
        .path()
        .strip_prefix(origin.path().trim_end_matches('/'))
        .unwrap_or(request.uri().path());
    let fault = plan.next("http", &format!("{} {}", request.method(), relative));
    if let Some(fault) = &fault {
        if fault.action == "failBefore" {
            return Err(io::Error::other("injected connection refusal"));
        }
        if fault.action == "status" {
            let mut response = Response::builder()
                .status(fault.status.unwrap_or(503))
                .header("content-type", "application/json");
            if let Some(ms) = fault.retry_after_ms {
                response = response.header("retry-after", ms.div_ceil(1000).to_string());
            }
            return response
                .body(full(
                    serde_json::to_vec(fault.body.as_ref().unwrap_or(&serde_json::json!({})))
                        .map_err(io::Error::other)?,
                ))
                .map_err(io::Error::other);
        }
    }
    let (parts, body) = request.into_parts();
    let mut headers = parts.headers;
    for key in [
        "host",
        "connection",
        "transfer-encoding",
        "upgrade",
        "keep-alive",
        "proxy-connection",
    ] {
        headers.remove(key);
    }
    let upstream = format!("{}{}", origin.origin().ascii_serialization(), parts.uri);
    let response = client
        .request(parts.method, &upstream)
        .headers(headers)
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await
        .map_err(io::Error::other)?;
    if fault.as_ref().is_some_and(|f| f.action == "dropResponse") {
        let _ = response.bytes().await.map_err(io::Error::other)?;
        return Err(io::Error::other("injected lost response"));
    }
    let mut builder = Response::builder().status(response.status());
    if let Some(headers) = builder.headers_mut() {
        *headers = response.headers().clone();
    }
    builder
        .body(
            StreamBody::new(
                response
                    .bytes_stream()
                    .map(|chunk| chunk.map(Frame::data).map_err(io::Error::other)),
            )
            .boxed_unsync(),
        )
        .map_err(io::Error::other)
}
pub async fn start(
    url: &str,
    plan: Plan,
    client: reqwest::Client,
) -> Result<(String, tokio::task::JoinHandle<()>), Box<dyn std::error::Error>> {
    let origin = reqwest::Url::parse(url)?;
    if origin.scheme() != "http" {
        return Err("fault fixture only supports local HTTP".into());
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let mut local = origin.clone();
    local.set_host(Some("127.0.0.1"))?;
    local
        .set_port(Some(address.port()))
        .map_err(|_| "invalid fixture port")?;
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let origin = origin.clone();
            let plan = plan.clone();
            let client = client.clone();
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        hyper_util::rt::TokioIo::new(stream),
                        service_fn(move |request| {
                            answer(request, origin.clone(), plan.clone(), client.clone())
                        }),
                    )
                    .await;
            });
        }
    });
    Ok((local.to_string(), task))
}
