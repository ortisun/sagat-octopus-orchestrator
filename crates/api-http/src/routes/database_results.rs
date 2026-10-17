//! A dedicated audited view, with no arbitrary payload/blob selectors or bulk export.
use crate::{admission::admit_resolved_namespace, error::ApiError, exec::Exec, extract::Caller};
use axum::{
    Extension,
    body::Body,
    extract::{Path, Request, State},
    http::{HeaderValue, header},
    middleware::Next,
    response::Response,
};
use sagat_application::database_results::DatabaseResultRequest;
use sagat_domain::{execution::RunId, identity::NamespaceId, workflow::NodeId};
use std::time::Duration;

pub(crate) async fn no_store(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

pub(crate) async fn read<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Extension(connection): Extension<crate::server::ConnectionAbort>,
    Path((namespace, run, node)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let namespace = namespace
        .parse::<NamespaceId>()
        .map_err(|_| ApiError::not_found())?;
    let run = run.parse::<RunId>().map_err(|_| ApiError::not_found())?;
    let node = NodeId::parse(&node).map_err(|_| ApiError::not_found())?;
    let results = state.database_results().ok_or_else(ApiError::unavailable)?;
    let slot = state.blob_transfers().acquire()?;
    let expires = i64::try_from(caller.token_expires_at)
        .ok()
        .and_then(|value| time::OffsetDateTime::from_unix_timestamp(value).ok())
        .and_then(|expires| Duration::try_from(expires - time::OffsetDateTime::now_utc()).ok())
        .unwrap_or_default();
    let timeout = state
        .blob_transfers()
        .limits
        .timeout
        .min(Duration::from_secs(30))
        .min(expires);
    if timeout.is_zero() {
        return Err(ApiError::request_timeout());
    }
    let deadline = tokio::time::Instant::now() + timeout;
    let bytes = tokio::time::timeout_at(deadline, async {
        let prepared = results
            .prepare(DatabaseResultRequest {
                principal: caller.session.principal.clone(),
                namespace,
                run,
                node,
                trace_id: caller.ctx.trace_id.clone(),
            })
            .await?;
        admit_resolved_namespace(&state, &caller, prepared.namespace()).await?;
        results.read(prepared).await.map_err(ApiError::from)
    })
    .await
    .map_err(|_| ApiError::request_timeout())??;
    // Keep the bounded-buffer slot until the body is consumed or the connection is dropped.
    let mut response = Response::new(Body::from(axum::body::Bytes::from_owner(ResultBody {
        bytes,
        _slot: slot,
        _deadline: connection.deadline(deadline),
    })));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    Ok(response)
}

// Bytes retains this owner through Hyper buffers and clones, including slow response consumers.
struct ResultBody {
    bytes: sagat_application::payloads::StoredPayloadBytes,
    _slot: tokio::sync::OwnedSemaphorePermit,
    _deadline: crate::server::DeliveryDeadline,
}
impl AsRef<[u8]> for ResultBody {
    fn as_ref(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}
