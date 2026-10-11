//! Workflow definitions and runs.
//!
//! Every handler takes [`Caller`] first, so authentication (401) is decided before any path,
//! query, header or body is looked at. The namespace is a header (`X-Octopus-Namespace`, a slug);
//! the tenant is never read from the request.

use std::str::FromStr;

use std::convert::Infallible;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use sagat_application::definitions::DiffRef;
use sagat_application::error::AppError;
use sagat_application::identity::{Page, PageRequest};
use sagat_application::runs::{GraphChanges, ReviewAction, RunFilter, RunOperation, StartRun};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::{FailureCode, ReviewOutcome, RunId, RunStatus};
use sagat_domain::workflow::NodeId;
use serde::Deserialize;
use serde_json::Value;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::dto::PageBody;
use crate::dto_exec::{
    CreateWorkflowBody, JournalEventBody, OperationBody, OperationResultBody, PublishBody,
    PublishedBody, RunBody, SignalResultBody, StartRunBody, UpdateDraftBody, ValidateBody,
    VersionBody, VersionSummary, WorkflowBody, compensation_plans_body, diff_body,
    graph_changes_body, runtime_graph, validation_body,
};
use crate::error::ApiError;
use crate::exec::Exec;
use crate::extract::{ApiJson, Caller};
use crate::streams::{StreamLimits, StreamRefused};

const NAMESPACE_HEADER: &str = "x-octopus-namespace";
const IDEMPOTENCY_HEADER: &str = "idempotency-key";

fn namespace(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get(NAMESPACE_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 63)
        .ok_or_else(|| ApiError::bad_request("the X-Octopus-Namespace header is required"))
}

/// `Idempotency-Key` is mandatory on every operation that creates or changes something, so a client
/// that retries after an ambiguous failure can never apply it twice.
fn idempotency_key(headers: &HeaderMap) -> Result<&str, ApiError> {
    let key = headers
        .get(IDEMPOTENCY_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("the Idempotency-Key header is required"))?;
    if !(8..=160).contains(&key.len()) || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(ApiError::bad_request(
            "the Idempotency-Key must be 8 to 160 printable ASCII characters",
        ));
    }
    Ok(key)
}

/// The id an operation is answered with. It is derived from the run and the `Idempotency-Key`, so a
/// request sent again because its answer was lost is answered with the same id: it is the same
/// operation. It names the request, it is not a stored record (the journal event the operation
/// caused is the record: G153).
fn operation_id(run: &str, key: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("operation/{run}/{key}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn parse_id<T: FromStr>(raw: &str) -> Result<T, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    #[serde(rename = "workflowId")]
    pub workflow_id: Option<String>,
    pub status: Option<String>,
}

fn page_request(query: &ListQuery) -> PageRequest {
    PageRequest {
        limit: query.limit,
        cursor: query.cursor.clone(),
    }
}

fn page<T, B: for<'a> From<&'a T>>(page: &Page<T>) -> Json<PageBody<B>> {
    Json(PageBody {
        items: page.items.iter().map(B::from).collect(),
        next_cursor: page.next_cursor.clone(),
    })
}

fn to_bytes(value: &Value) -> Result<Vec<u8>, ApiError> {
    serde_json::to_vec(value).map_err(|_| ApiError::bad_request("the document cannot be encoded"))
}

// ------------------------------------------------------------------------------ definitions

pub async fn create_workflow<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<CreateWorkflowBody>,
) -> Result<(StatusCode, Json<WorkflowBody>), ApiError> {
    let record = state
        .definitions()
        .create_workflow(
            &caller.session.principal,
            namespace(&headers)?,
            &body.name,
            &to_bytes(&body.ir)?,
            idempotency_key(&headers)?,
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(WorkflowBody::from(&record))))
}

pub async fn list_workflows<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<PageBody<WorkflowBody>>, ApiError> {
    let result = state
        .definitions()
        .list_workflows(
            &caller.session.principal,
            namespace(&headers)?,
            &page_request(&query),
        )
        .await?;
    Ok(page(&result))
}

pub async fn get_workflow<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
) -> Result<Json<WorkflowBody>, ApiError> {
    let record = state
        .definitions()
        .get_workflow(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(WorkflowBody::from(&record)))
}

pub async fn update_draft<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateDraftBody>,
) -> Result<Json<WorkflowBody>, ApiError> {
    let record = state
        .definitions()
        .update_draft(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            body.draft_revision,
            &to_bytes(&body.ir)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(WorkflowBody::from(&record)))
}

pub async fn validate_workflow<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ValidateBody>,
) -> Result<Json<Value>, ApiError> {
    let document = body.ir.as_ref().map(to_bytes).transpose()?;
    let validation = state
        .definitions()
        .validate(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            document.as_deref(),
            &caller.ctx,
        )
        .await?;
    Ok(Json(validation_body(&validation)))
}

#[derive(Debug, Default, Deserialize)]
pub struct DiffQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

/// `draft` or a version number: nothing else names a document of a workflow.
fn diff_ref(raw: Option<&str>) -> Result<Option<DiffRef>, ApiError> {
    match raw {
        None => Ok(None),
        Some("draft") => Ok(Some(DiffRef::Draft)),
        Some(number) => number
            .parse::<u32>()
            .ok()
            .filter(|n| *n >= 1 && number.bytes().all(|b| b.is_ascii_digit()))
            .map(|n| Some(DiffRef::Version(n)))
            .ok_or_else(|| {
                ApiError::bad_request("`from` and `to` are `draft` or a version number")
            }),
    }
}

pub async fn diff_workflow<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    Query(query): Query<DiffQuery>,
) -> Result<Json<Value>, ApiError> {
    let diff = state
        .definitions()
        .diff(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            diff_ref(query.from.as_deref())?,
            diff_ref(query.to.as_deref())?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(diff_body(&diff)))
}

pub async fn publish_workflow<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<PublishBody>,
) -> Result<(StatusCode, Json<PublishedBody>), ApiError> {
    let published = state
        .definitions()
        .publish(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            body.draft_revision,
            body.release_notes.as_deref(),
            idempotency_key(&headers)?,
            &caller.ctx,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(PublishedBody {
            version: VersionSummary::from(&published.version),
            created: published.created,
            warnings: sagat_application::definitions::warnings_text(&published.warnings),
        }),
    ))
}

pub async fn list_versions<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<PageBody<VersionBody>>, ApiError> {
    let result = state
        .definitions()
        .list_versions(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            &page_request(&query),
            &caller.ctx,
        )
        .await?;
    Ok(page(&result))
}

pub async fn get_version<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path((id, number)): Path<(String, String)>,
) -> Result<Json<VersionBody>, ApiError> {
    let number = number.parse::<u32>().map_err(|_| ApiError::not_found())?;
    let record = state
        .definitions()
        .get_version(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            number,
            &caller.ctx,
        )
        .await?;
    let mut body = VersionBody::from(&record);
    if let Some(publishing) = state.publishing() {
        body.scripts = Some(
            publishing
                .dependencies(
                    &caller.session.principal,
                    parse_id::<WorkflowId>(&id)?,
                    number,
                    &caller.ctx,
                )
                .await?,
        );
    }
    Ok(Json(body))
}

// ------------------------------------------------------------------------------ runs

pub async fn start_run<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<StartRunBody>,
) -> Result<(StatusCode, Json<RunBody>), ApiError> {
    let input = to_bytes(&body.input)?;
    let version = body
        .version_id
        .as_deref()
        .map(parse_id::<WorkflowVersionId>)
        .transpose()?;
    let record = state
        .runs()
        .start(
            &caller.session.principal,
            &StartRun {
                namespace: namespace(&headers)?,
                workflow: parse_id::<WorkflowId>(&body.workflow_id)?,
                version,
                input: &input,
                correlation_id: body.correlation_id.as_deref(),
                deadline: body.deadline,
                key: idempotency_key(&headers)?,
            },
            &caller.ctx,
        )
        .await?;
    Ok((StatusCode::ACCEPTED, Json(RunBody::from(&record))))
}

pub async fn list_runs<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<PageBody<RunBody>>, ApiError> {
    let filter = RunFilter {
        workflow: query
            .workflow_id
            .as_deref()
            .map(parse_id::<WorkflowId>)
            .transpose()?,
        status: query
            .status
            .as_deref()
            .map(|s| {
                s.parse::<RunStatus>()
                    .map_err(|_| ApiError::validation("unknown run status"))
            })
            .transpose()?,
    };
    let result = state
        .runs()
        .list(
            &caller.session.principal,
            namespace(&headers)?,
            &filter,
            &page_request(&query),
        )
        .await?;
    Ok(page(&result))
}

pub async fn get_run<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
) -> Result<Json<RunBody>, ApiError> {
    let run = state
        .runs()
        .get(
            &caller.session.principal,
            parse_id::<RunId>(&id)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(RunBody::from(&run)))
}

pub async fn run_journal<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<PageBody<JournalEventBody>>, ApiError> {
    let result = state
        .runs()
        .journal(
            &caller.session.principal,
            parse_id::<RunId>(&id)?,
            &page_request(&query),
            &caller.ctx,
        )
        .await?;
    Ok(page(&result))
}

/// `GET /runs/{runId}/telemetry`
pub async fn run_telemetry<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
) -> Result<Json<crate::dto_exec::RunTelemetryBody>, ApiError> {
    let run = parse_id::<RunId>(&id)?;
    let view = state
        .runs()
        .telemetry(&caller.session.principal, run, &caller.ctx)
        .await?;
    let record = state
        .runs()
        .get(&caller.session.principal, run, &caller.ctx)
        .await?;
    let millis = |seconds: f64| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ms = (seconds * 1000.0).round() as u64;
        ms
    };
    Ok(Json(crate::dto_exec::RunTelemetryBody {
        run_id: run.to_string(),
        trace_id: record.info.trace_id.clone(),
        state: view.state.as_str(),
        head_seq: view.telemetry.head_seq,
        applied_seq: view.telemetry.applied_seq,
        lag_ms: millis(view.telemetry.behind_seconds),
        budget_ms: u64::try_from(view.budget.as_millis()).unwrap_or(u64::MAX),
    }))
}

/// `GET /runs/{runId}/attempts`
pub async fn run_attempts<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
) -> Result<Json<crate::dto_exec::AttemptListBody>, ApiError> {
    let run = parse_id::<RunId>(&id)?;
    let attempts = state
        .runs()
        .attempts(&caller.session.principal, run, &caller.ctx)
        .await?;
    let record = state
        .runs()
        .get(&caller.session.principal, run, &caller.ctx)
        .await?;
    Ok(Json(crate::dto_exec::AttemptListBody {
        run_id: run.to_string(),
        trace_id: record.info.trace_id.clone(),
        items: attempts
            .iter()
            .map(crate::dto_exec::AttemptBody::from)
            .collect(),
    }))
}

pub async fn run_graph<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let graph = state
        .runs()
        .graph(
            &caller.session.principal,
            parse_id::<RunId>(&id)?,
            &caller.ctx,
        )
        .await?;
    Ok(Json(runtime_graph(&graph)))
}

/// `since` is read by hand, not by the extractor, so that a run the caller may not see is `404` (or
/// `403`) whatever else is wrong with the request: a refusal about the query must not tell a foreign
/// run from one that does not exist, and must not come before the refusal about the run.
#[derive(Debug, Default, Deserialize)]
pub struct ChangesQuery {
    pub since: Option<String>,
}

pub async fn run_graph_changes<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path(id): Path<String>,
    Query(query): Query<ChangesQuery>,
) -> Result<Json<Value>, ApiError> {
    let run = parse_id::<RunId>(&id)?;
    let Some(since) = query
        .since
        .as_deref()
        .and_then(|text| text.parse::<u64>().ok())
    else {
        // Who may not read the run is told that first; only then what is wrong with the query.
        state
            .runs()
            .get(&caller.session.principal, run, &caller.ctx)
            .await?;
        return Err(ApiError::bad_request(
            "since is the projection sequence the client last saw: a non-negative integer",
        ));
    };
    let changes = state
        .runs()
        .graph_changes(&caller.session.principal, run, since, &caller.ctx)
        .await?;
    Ok(Json(graph_changes_body(&changes)))
}

#[derive(Debug, Default, Deserialize)]
pub struct StreamQuery {
    pub after: Option<u64>,
}

/// The sequence a reconnecting client has: `Last-Event-ID` wins over `after`.
fn resume_point(headers: &HeaderMap, query: &StreamQuery) -> Result<Option<u64>, ApiError> {
    match headers.get("last-event-id") {
        None => Ok(query.after),
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|text| text.parse::<u64>().ok())
            .map(Some)
            .ok_or_else(|| ApiError::bad_request("Last-Event-ID is a projection sequence")),
    }
}

/// The run as a stream of graph changes (`text/event-stream`, D141, D142).
///
/// The first read happens before the stream opens, so an unknown run, another tenant's run or a
/// caller who may not read runs gets the ordinary `404` or `403`. After that the server looks for
/// a change every half second while the run moves (every two seconds once it has been still for
/// half a minute), sends what changed, and closes when the run is finished, when the token
/// expires, or after fifteen minutes; a client that is not told `end` reconnects with the last
/// sequence it saw.
pub async fn run_stream<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<StreamQuery>,
) -> Result<Response, ApiError> {
    let run = parse_id::<RunId>(&id)?;
    let after = resume_point(&headers, &query)?;
    let principal = caller.session.principal.clone();
    let ctx = caller.ctx.clone();
    // Reading first means a refusal is a refusal, not an empty stream.
    let first = state
        .runs()
        .graph_changes(&principal, run, after.unwrap_or(u64::MAX), &ctx)
        .await?;
    let limits = *state.streams().limits();
    let slot = state
        .streams()
        .try_open(principal.tenant_id(), principal.subject().as_str())
        .map_err(|refused| {
            metrics::counter!("run_streams_refused_total").increment(1);
            ApiError::rate_limited(std::time::Duration::from_secs(5)).with_reason(match refused {
                StreamRefused::GlobalAtCapacity => "STREAMS_GLOBAL",
                StreamRefused::SubjectAtCapacity => "STREAMS_PER_SUBJECT",
                StreamRefused::TenantAtCapacity => "STREAMS_PER_TENANT",
            })
        })?;
    let deadline = stream_deadline(caller.token_expires_at, limits.max_lifetime);
    let who = Who {
        tenant: caller.session.tenant.slug().as_str().to_owned(),
        subject: principal.subject().as_str().to_owned(),
    };
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<SseEvent, Infallible>>(8);
    metrics::gauge!("run_streams_open").increment(1.0);
    tokio::spawn(async move {
        let _slot = slot;
        follow(state, principal, who, ctx, run, first, limits, deadline, tx).await;
        metrics::gauge!("run_streams_open").decrement(1.0);
    });
    let mut response = Sse::new(ReceiverStream::new(rx))
        .keep_alive(KeepAlive::new().interval(limits.ping).text("ping"))
        .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    // Proxies that buffer a response would hold every event back.
    response.headers_mut().insert(
        "x-accel-buffering",
        axum::http::HeaderValue::from_static("no"),
    );
    Ok(response)
}

/// The instant a stream must end: the token's expiry or the longest lifetime, whichever is first.
fn stream_deadline(
    token_expires_at: u64,
    max_lifetime: std::time::Duration,
) -> tokio::time::Instant {
    let now = tokio::time::Instant::now();
    let unix_now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let until_token = std::time::Duration::from_secs(token_expires_at.saturating_sub(unix_now));
    now + until_token.min(max_lifetime)
}

fn graph_event(changes: &GraphChanges) -> SseEvent {
    SseEvent::default()
        .event("graph")
        .id(changes.projection_seq().to_string())
        .data(graph_changes_body(changes).to_string())
}

/// Who opened a stream, as the identity provider named them: what is asked again, every so often,
/// to know whether they may still read.
struct Who {
    tenant: String,
    subject: String,
}

/// `base` and up to a quarter more, so that the streams a database blip or a deploy cut at the same
/// moment do not all come back at the same moment.
fn jittered(base: std::time::Duration) -> std::time::Duration {
    let unit = (uuid::Uuid::new_v4().as_u128() >> 64) as f64 / u64::MAX as f64;
    base + base.mul_f64(unit * 0.25)
}

/// Sends the first changes, then follows the run until it ends, the deadline passes or the
/// client leaves (the channel closes).
///
/// Each tick looks at the run's own row, which is one read of one row; only when its version moved
/// is the graph read (a snapshot of every step of the run). A stream of a quiet run therefore costs
/// the database what a `GET /runs/{id}` costs, a tick at a time.
#[allow(clippy::too_many_arguments)]
async fn follow<B: Exec>(
    state: B,
    mut principal: sagat_domain::identity::Principal,
    who: Who,
    ctx: sagat_application::identity::RequestContext,
    run: RunId,
    first: GraphChanges,
    limits: StreamLimits,
    deadline: tokio::time::Instant,
    tx: tokio::sync::mpsc::Sender<Result<SseEvent, Infallible>>,
) {
    let send = |event: SseEvent| {
        let tx = tx.clone();
        async move { tx.send(Ok(event)).await.is_ok() }
    };
    let mut cursor = first.projection_seq();
    let mut finished = first.graph.run.status.is_terminal();
    if !send(graph_event(&first)).await {
        return;
    }
    let mut last_change = tokio::time::Instant::now();
    let mut next_recheck = last_change + limits.recheck;
    let mut pause = limits.poll_min;
    while !finished {
        tokio::select! {
            () = tokio::time::sleep(jittered(pause)) => {}
            () = tx.closed() => return,
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            let _ = send(SseEvent::default().event("reconnect").data("{}")).await;
            return;
        }
        // The role the stream opened with is not the role for fifteen minutes: whoever was removed
        // or lowered since is asked again, and the stream is cut for them.
        if now >= next_recheck {
            next_recheck = now + limits.recheck;
            match state.authenticate(&who.tenant, &who.subject).await {
                Ok(session) => principal = session.principal,
                Err(
                    AppError::Unauthenticated | AppError::NotFound | AppError::Forbidden { .. },
                ) => {
                    // "Come back", not "the run is over": the client opens the stream again and is told the
                    // truth by the refusal (401, 403 or 404), which it shows as what it is.
                    let _ = send(SseEvent::default().event("reconnect").data("{}")).await;
                    return;
                }
                Err(_) => {}
            }
        }
        match state.runs().get(&principal, run, &ctx).await {
            Ok(record) if record.version == cursor => {
                if last_change.elapsed() >= limits.idle_after {
                    pause = limits.poll_max;
                }
                continue;
            }
            Ok(_) => {}
            Err(AppError::NotFound | AppError::Forbidden { .. }) => {
                let _ = send(SseEvent::default().event("reconnect").data("{}")).await;
                return;
            }
            Err(_) => {
                metrics::counter!("run_streams_tick_errors_total").increment(1);
                continue;
            }
        }
        match state
            .runs()
            .graph_changes(&principal, run, cursor, &ctx)
            .await
        {
            Ok(changes) if changes.projection_seq() != cursor || changes.reset => {
                cursor = changes.projection_seq();
                finished = changes.graph.run.status.is_terminal();
                last_change = tokio::time::Instant::now();
                pause = limits.poll_min;
                if !send(graph_event(&changes)).await {
                    return;
                }
            }
            Ok(_) => {
                if last_change.elapsed() >= limits.idle_after {
                    pause = limits.poll_max;
                }
            }
            // The run is gone, or the caller may no longer read it: come back, and be told which.
            Err(AppError::NotFound | AppError::Forbidden { .. }) => {
                let _ = send(SseEvent::default().event("reconnect").data("{}")).await;
                return;
            }
            // The database is unavailable for a moment: the next tick tries again, and the
            // client does not see a gap because it resumes from the sequence it has. It is
            // counted, because a stream that cannot read says nothing about why.
            Err(_) => {
                metrics::counter!("run_streams_tick_errors_total").increment(1);
            }
        }
    }
    let _ = send(SseEvent::default().event("end").data("{}")).await;
}

pub async fn operate_run<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<OperationBody>,
) -> Result<(StatusCode, Json<OperationResultBody>), ApiError> {
    if !(8..=2000).contains(&body.reason.trim().chars().count()) {
        return Err(ApiError::validation(
            "the reason must be 8 to 2000 characters",
        ));
    }
    let review = match body.action.as_str() {
        "RETRY_STEP" => Some(ReviewAction::Retry),
        "MARK_MANUAL_REVIEW" => Some(ReviewAction::Hold),
        "RETRY_COMPENSATION" => Some(ReviewAction::Retry),
        "SKIP_COMPENSATION" => Some(ReviewAction::Skip),
        "RESOLVE_STEP" => Some(ReviewAction::Resolve(resolution(&body)?)),
        _ => None,
    };
    if let Some(action) = review {
        return review_step(&caller, &state, &headers, &id, &body, &action).await;
    }
    let operation = match body.action.as_str() {
        "CANCEL" => RunOperation::Cancel,
        "PAUSE" => RunOperation::Pause,
        "RESUME" => RunOperation::Resume,
        "RERUN_AS_NEW" => return rerun_run(&caller, &state, &headers, &id, &body).await,
        _ => return Err(ApiError::validation("unknown operation")),
    };
    if body.step_id.is_some() {
        return Err(ApiError::validation(
            "CANCEL, PAUSE and RESUME apply to the whole run",
        ));
    }
    let run_id = parse_id::<RunId>(&id)?;
    let run = state
        .runs()
        .operate(
            &caller.session.principal,
            run_id,
            operation,
            &body.reason,
            body.expected_run_version,
            idempotency_key(&headers)?,
            &caller.ctx,
        )
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(OperationResultBody {
            operation_id: operation_id(&run_id.to_string(), idempotency_key(&headers)?),
            accepted: true,
            run_status: run.status.as_str().to_owned(),
            run_version: run.version,
            new_run_id: None,
        }),
    ))
}

/// `RERUN_AS_NEW`: a run of the same version with the same input, from a run that ended.
async fn rerun_run<B: Exec>(
    caller: &Caller,
    state: &B,
    headers: &HeaderMap,
    id: &str,
    body: &OperationBody,
) -> Result<(StatusCode, Json<OperationResultBody>), ApiError> {
    if body.step_id.is_some() || body.outcome.is_some() || body.error_code.is_some() {
        return Err(ApiError::validation(
            "RERUN_AS_NEW applies to the whole run and takes only a reason",
        ));
    }
    let run_id = parse_id::<RunId>(id)?;
    let original = state
        .runs()
        .get(&caller.session.principal, run_id, &caller.ctx)
        .await?;
    if body
        .expected_run_version
        .is_some_and(|expected| expected != original.version)
    {
        return Err(AppError::RevisionConflict {
            current: original.version,
        }
        .into());
    }
    let started = state
        .runs()
        .rerun(
            &caller.session.principal,
            run_id,
            &body.reason,
            idempotency_key(headers)?,
            &caller.ctx,
        )
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(OperationResultBody {
            operation_id: operation_id(&run_id.to_string(), idempotency_key(headers)?),
            accepted: true,
            run_status: original.status.as_str().to_owned(),
            run_version: original.version,
            new_run_id: Some(started.info.run_id.to_string()),
        }),
    ))
}

/// What `RESOLVE_STEP` declares: the step's effect happened, or it did not and why.
fn resolution(body: &OperationBody) -> Result<ReviewOutcome, ApiError> {
    match body.outcome.as_deref() {
        Some("SUCCEEDED") if body.error_code.is_none() => Ok(ReviewOutcome::Succeeded),
        Some("FAILED") => {
            let code = body
                .error_code
                .as_deref()
                .unwrap_or("DECLARED_FAILED_BY_OPERATOR");
            FailureCode::parse(code)
                .map(ReviewOutcome::Failed)
                .map_err(|_| ApiError::validation("errorCode is not a valid failure code"))
        }
        Some("SUCCEEDED") => Err(ApiError::validation(
            "errorCode only goes with outcome FAILED",
        )),
        _ => Err(ApiError::validation(
            "RESOLVE_STEP needs outcome SUCCEEDED or FAILED",
        )),
    }
}

/// An operator decides on a step that waits for a decision (a compensation that ran out of
/// attempts, or an effect whose outcome nobody could establish).
async fn review_step<B: Exec>(
    caller: &Caller,
    state: &B,
    headers: &HeaderMap,
    id: &str,
    body: &OperationBody,
    action: &ReviewAction,
) -> Result<(StatusCode, Json<OperationResultBody>), ApiError> {
    let Some(step) = body.step_id.as_deref() else {
        return Err(ApiError::validation(
            "stepId is required for this operation",
        ));
    };
    let node = NodeId::parse_stored(step)
        .map_err(|_| ApiError::validation("stepId is not a step of a run"))?;
    if body.action == "RETRY_COMPENSATION" && !node.is_compensation() {
        return Err(ApiError::validation(
            "RETRY_COMPENSATION needs the id of a compensation (ending in ~c)",
        ));
    }
    if body.action != "RESOLVE_STEP" && (body.outcome.is_some() || body.error_code.is_some()) {
        return Err(ApiError::validation(
            "outcome and errorCode only go with RESOLVE_STEP",
        ));
    }
    let run_id = parse_id::<RunId>(id)?;
    let run = state
        .runs()
        .review(
            &caller.session.principal,
            run_id,
            &node,
            action,
            &body.reason,
            body.expected_run_version,
            idempotency_key(headers)?,
            &caller.ctx,
        )
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(OperationResultBody {
            operation_id: operation_id(&run_id.to_string(), idempotency_key(headers)?),
            accepted: true,
            run_status: run.status.as_str().to_owned(),
            run_version: run.version,
            new_run_id: None,
        }),
    ))
}

/// The compensation plans a version was published with (`FR-003-002`).
pub async fn get_compensation_plans<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    Path((id, number)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let number = number.parse::<u32>().map_err(|_| ApiError::not_found())?;
    let plans = state
        .definitions()
        .compensation_plans(
            &caller.session.principal,
            parse_id::<WorkflowId>(&id)?,
            number,
            &caller.ctx,
        )
        .await?;
    Ok(Json(compensation_plans_body(&plans)))
}

pub async fn signal_run<B: Exec>(
    caller: Caller,
    State(state): State<B>,
    headers: HeaderMap,
    Path((id, name)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<SignalResultBody>), ApiError> {
    let delivered = state
        .runs()
        .signal(
            &caller.session.principal,
            parse_id::<RunId>(&id)?,
            &name,
            &body,
            idempotency_key(&headers)?,
            &caller.ctx,
        )
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(SignalResultBody {
            delivered_to: delivered.iter().map(ToString::to_string).collect(),
        }),
    ))
}
