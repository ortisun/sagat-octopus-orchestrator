use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use sagat_application::chain_progress::{ChainCommandStore, ChainRunTx};
use sagat_application::error::AppError;
use sagat_application::runs::RunStore;
use sagat_domain::execution::RunId;
use sagat_domain::identity::NamespaceId;
use serde_json::json;
use tower_http::limit::RequestBodyLimitLayer;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;

use crate::dto_chains::{
    ChainCancelBody, ChainInspectionBody, ChainRunPageBody, ChainSignalBody, ChainSignalResultBody,
};
use crate::error::ApiError;
use crate::exec::Exec;
use crate::extract::{ApiJson, Caller};

pub fn routes<B, T>(state: B, max_body_bytes: usize) -> Router<T>
where
    B: Exec,
    B::Store: ChainCommandStore,
    <B::Store as RunStore>::Tx: ChainRunTx,
    T: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/namespaces/{namespace_id}/chains/{chain_id}",
            get(inspect_chain::<B>),
        )
        .route(
            "/namespaces/{namespace_id}/chains/{chain_id}/runs",
            get(list_chain_runs::<B>),
        )
        .route(
            "/namespaces/{namespace_id}/chains/{chain_id}/signals/{signal_name}",
            post(signal_chain::<B>),
        )
        .route(
            "/namespaces/{namespace_id}/chains/{chain_id}/cancel",
            post(cancel_chain::<B>),
        )
        .layer(RequestBodyLimitLayer::new(max_body_bytes))
        .with_state(state)
}

pub async fn inspect_chain<B>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace_id, chain_id)): Path<(String, String)>,
) -> Result<(HeaderMap, Json<ChainInspectionBody>), ApiError>
where
    B: Exec,
    B::Store: ChainCommandStore,
    <B::Store as RunStore>::Tx: ChainRunTx,
{
    let namespace_id = parse_namespace(&namespace_id)?;
    let chain_id = parse_run(&chain_id)?;
    let inspection = state
        .runs()
        .chain_commands()
        .inspect(
            &caller.session.principal,
            namespace_id,
            chain_id,
            &caller.ctx,
        )
        .await?;
    Ok((no_store(), Json(inspection.into())))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChainRunsQuery {
    pub cursor: Option<String>,
    pub limit: Option<String>,
}

/// The cursor is the index of the last segment returned, in a fixed-width opaque form. A segment is
/// only ever appended, so a page never repeats or skips one when the chain grows between reads.
fn decode_segment_cursor(cursor: &str) -> Result<u16, ApiError> {
    let invalid = || ApiError::bad_request("the cursor is not valid");
    let bytes = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| invalid())?;
    let bytes: [u8; 2] = bytes.try_into().map_err(|_| invalid())?;
    let index = u16::from_be_bytes(bytes);
    if index >= 10_000 {
        return Err(invalid());
    }
    Ok(index)
}

fn encode_segment_cursor(index: u16) -> String {
    URL_SAFE_NO_PAD.encode(index.to_be_bytes())
}

pub async fn list_chain_runs<B>(
    caller: Caller,
    State(state): State<B>,
    Path((namespace_id, chain_id)): Path<(String, String)>,
    Query(query): Query<ChainRunsQuery>,
) -> Result<(HeaderMap, Json<ChainRunPageBody>), ApiError>
where
    B: Exec,
    B::Store: ChainCommandStore,
    <B::Store as RunStore>::Tx: ChainRunTx,
{
    let namespace_id = parse_namespace(&namespace_id)?;
    let chain_id = parse_run(&chain_id)?;
    let after = query
        .cursor
        .as_deref()
        .map(decode_segment_cursor)
        .transpose()?;
    // Out-of-range sizes are clamped, as every list of the API does; text is not a size.
    let limit = match query.limit.as_deref() {
        None => 50,
        Some(text) => text
            .parse::<i64>()
            .map_err(|_| ApiError::bad_request("the limit is not a number"))?
            .clamp(1, 200) as u32,
    };
    let page = state
        .runs()
        .chain_commands()
        .list_runs(
            &caller.session.principal,
            namespace_id,
            chain_id,
            after,
            limit,
            &caller.ctx,
        )
        .await?;
    Ok((
        no_store(),
        Json(ChainRunPageBody::from_page(page, encode_segment_cursor)),
    ))
}

pub async fn signal_chain<B>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Path((namespace_id, chain_id, signal_name)): Path<(String, String, String)>,
    ApiJson(body): ApiJson<ChainSignalBody>,
) -> Result<(StatusCode, HeaderMap, Json<ChainSignalResultBody>), ApiError>
where
    B: Exec,
    B::Store: ChainCommandStore,
    <B::Store as RunStore>::Tx: ChainRunTx,
{
    let namespace_id = parse_namespace(&namespace_id)?;
    let chain_id = parse_run(&chain_id)?;
    let expected = parse_expected_run(&body.expected_run_id)?;
    let key = idempotency_key(&headers)?.to_owned();
    let payload =
        serde_json::to_vec(&body.payload).map_err(|_| ApiError::from(AppError::Internal))?;
    let commands = state.runs().chain_commands();
    let delivered = commands
        .signal(
            &caller.session.principal,
            namespace_id,
            chain_id,
            Some(expected),
            &signal_name,
            &payload,
            &key,
            &caller.ctx,
        )
        .await
        .map_err(chain_error)?;
    Ok((
        StatusCode::ACCEPTED,
        no_store(),
        Json(ChainSignalResultBody {
            delivered_to: delivered.into_iter().map(|node| node.to_string()).collect(),
        }),
    ))
}

pub async fn cancel_chain<B>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Path((namespace_id, chain_id)): Path<(String, String)>,
    ApiJson(body): ApiJson<ChainCancelBody>,
) -> Result<(HeaderMap, StatusCode), ApiError>
where
    B: Exec,
    B::Store: ChainCommandStore,
    <B::Store as RunStore>::Tx: ChainRunTx,
{
    let namespace_id = parse_namespace(&namespace_id)?;
    let chain_id = parse_run(&chain_id)?;
    let expected = parse_expected_run(&body.expected_run_id)?;
    if !(1..=512).contains(&body.reason.chars().count()) {
        return Err(ApiError::validation(
            "reason must be between 1 and 512 characters",
        ));
    }
    let key = idempotency_key(&headers)?.to_owned();
    let commands = state.runs().chain_commands();
    commands
        .cancel(
            &caller.session.principal,
            namespace_id,
            chain_id,
            Some(expected),
            &body.reason,
            &key,
            &caller.ctx,
        )
        .await
        .map_err(chain_error)?;
    Ok((no_store(), StatusCode::ACCEPTED))
}

fn parse_namespace(value: &str) -> Result<NamespaceId, ApiError> {
    value
        .parse::<uuid::Uuid>()
        .map(NamespaceId::new)
        .map_err(|_| ApiError::not_found())
}

fn parse_run(value: &str) -> Result<RunId, ApiError> {
    value.parse().map_err(|_| ApiError::not_found())
}

fn parse_expected_run(value: &str) -> Result<RunId, ApiError> {
    value.parse().map_err(|_| {
        ApiError::validation("expectedRunId must be a valid run identifier")
            .with_reason("INVALID_EXPECTED_RUN_ID")
    })
}

fn idempotency_key(headers: &HeaderMap) -> Result<&str, ApiError> {
    let key = headers
        .get("Idempotency-Key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("the Idempotency-Key header is required"))?;
    if !(8..=160).contains(&key.len()) || !key.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        return Err(ApiError::bad_request(
            "the Idempotency-Key must be 8 to 160 printable ASCII characters",
        ));
    }
    Ok(key)
}

fn chain_error(error: AppError) -> ApiError {
    if error == AppError::StateConflict("the chain head changed") {
        ApiError::from(error).with_details(json!({"reason": "CHAIN_HEAD_CHANGED"}))
    } else {
        error.into()
    }
}

fn no_store() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers
}
