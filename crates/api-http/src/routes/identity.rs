//! Auth context, workspaces and namespaces (US01, US02, US04).
//!
//! Every handler takes [`Caller`] first, so authentication (401) is decided before any path, query
//! or body is looked at. The tenant is never read from the request: it comes from the session.

use std::str::FromStr;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use sagat_application::clock::Clock;
use sagat_application::identity::{IdentityStore, Page, PageRequest};
use sagat_domain::identity::WorkspaceId;

use crate::AppState;
use crate::dto::{
    AuthContextBody, CreateNamespaceBody, CreateWorkspaceBody, NamespaceBody, PageBody, PageQuery,
    WorkspaceBody,
};
use crate::error::ApiError;
use crate::extract::{ApiJson, Caller};
use crate::routes::execution::optional_idempotency_key;

fn page_request(query: PageQuery) -> PageRequest {
    PageRequest {
        limit: query.limit,
        cursor: query.cursor,
    }
}

fn page_body<T, B: for<'a> From<&'a T>>(page: &Page<T>) -> PageBody<B> {
    PageBody {
        items: page.items.iter().map(B::from).collect(),
        next_cursor: page.next_cursor.clone(),
    }
}

/// An identifier that does not parse cannot exist: report it like any other unknown resource.
fn parse_id<T: FromStr>(raw: &str) -> Result<T, ApiError> {
    raw.parse().map_err(|_| ApiError::not_found())
}

pub async fn auth_context(caller: Caller) -> Json<AuthContextBody> {
    let principal = &caller.session.principal;
    Json(AuthContextBody {
        tenant: caller.session.tenant.slug().to_string(),
        subject: principal.subject().to_string(),
        role: principal.role().to_string(),
        permissions: principal
            .role()
            .permissions()
            .iter()
            .map(|p| p.as_str())
            .collect(),
        trace_id: caller.trace.trace_id().to_string(),
    })
}

/// The version of the platform, for any caller with a token: the build, the API and the worker
/// protocols it speaks (a client compares them with what it was written for; `sagat doctor` does).
pub async fn platform_version(_caller: Caller) -> Json<crate::dto::VersionBody> {
    Json(crate::dto::VersionBody {
        service: "sagat-octopus-orchestrator",
        version: env!("CARGO_PKG_VERSION"),
        api_version: "v1",
        worker_protocol: crate::dto::ProtocolRange {
            minimum: "1.0".to_owned(),
            maximum: format!("1.{}", sagat_application::worker::PROTOCOL_MINOR),
        },
    })
}

pub async fn create_workspace<S, C>(
    caller: Caller,
    State(state): State<AppState<S, C>>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<CreateWorkspaceBody>,
) -> Result<(StatusCode, Json<WorkspaceBody>), ApiError>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    let principal = &caller.session.principal;
    let workspace = match optional_idempotency_key(&headers)? {
        Some(key) => {
            state
                .identity
                .create_workspace_idempotent(principal, &body.slug, key, &caller.ctx)
                .await?
        }
        None => {
            state
                .identity
                .create_workspace(principal, &body.slug, &caller.ctx)
                .await?
        }
    };
    Ok((StatusCode::CREATED, Json(WorkspaceBody::from(&workspace))))
}

pub async fn list_workspaces<S, C>(
    caller: Caller,
    State(state): State<AppState<S, C>>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PageBody<WorkspaceBody>>, ApiError>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    let page = state
        .identity
        .list_workspaces(&caller.session.principal, &page_request(query))
        .await?;
    Ok(Json(page_body(&page)))
}

pub async fn get_workspace<S, C>(
    caller: Caller,
    State(state): State<AppState<S, C>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceBody>, ApiError>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    let id = parse_id::<WorkspaceId>(&id)?;
    let workspace = state
        .identity
        .get_workspace(&caller.session.principal, id, &caller.ctx)
        .await?;
    Ok(Json(WorkspaceBody::from(&workspace)))
}

pub async fn create_namespace<S, C>(
    caller: Caller,
    State(state): State<AppState<S, C>>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<CreateNamespaceBody>,
) -> Result<(StatusCode, Json<NamespaceBody>), ApiError>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    let workspace_id = parse_id::<WorkspaceId>(&workspace_id)?;
    let principal = &caller.session.principal;
    let namespace = match optional_idempotency_key(&headers)? {
        Some(key) => {
            state
                .identity
                .create_namespace_idempotent(
                    principal,
                    workspace_id,
                    &body.slug,
                    &body.environment,
                    key,
                    &caller.ctx,
                )
                .await?
        }
        None => {
            state
                .identity
                .create_namespace(
                    principal,
                    workspace_id,
                    &body.slug,
                    &body.environment,
                    &caller.ctx,
                )
                .await?
        }
    };
    Ok((StatusCode::CREATED, Json(NamespaceBody::from(&namespace))))
}

pub async fn list_namespaces<S, C>(
    caller: Caller,
    State(state): State<AppState<S, C>>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PageBody<NamespaceBody>>, ApiError>
where
    S: IdentityStore + 'static,
    C: Clock + 'static,
{
    let page = state
        .identity
        .list_namespaces(&caller.session.principal, &page_request(query))
        .await?;
    Ok(Json(page_body(&page)))
}
