use crate::{
    admission::admit,
    dto_database::*,
    error::ApiError,
    extract::{ApiJson, Caller},
    governance::Governance,
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use sagat_application::{
    api_admission::{ApiOperation, ApiTarget},
    database::{ConnectionService, ConnectionWrite},
    error::AppError,
};
use sagat_domain::{
    database::ConnectionStatus,
    identity::{NamespaceId, Slug},
};

fn service<B: Governance>(state: &B) -> Result<&ConnectionService<B::Store, B::Clock>, ApiError> {
    state.connections().ok_or_else(ApiError::not_found)
}
async fn namespace<B: Governance>(
    state: &B,
    caller: &Caller,
    id: &str,
    operation: ApiOperation,
) -> Result<NamespaceId, ApiError> {
    let ns = id.parse().map_err(|_| ApiError::not_found())?;
    admit(state, caller, operation, ApiTarget::Namespace(ns)).await?;
    Ok(ns)
}
fn name(value: &str) -> Result<Slug, ApiError> {
    Slug::parse(value).map_err(|_| ApiError::not_found())
}
fn request_id(value: &str) -> Result<uuid::Uuid, ApiError> {
    value
        .parse()
        .map_err(|_| AppError::NotAllowed("requestId must be a UUID").into())
}
pub async fn list<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(ns): Path<String>,
) -> Result<Json<ConnectionListBody>, ApiError> {
    let ns = namespace(&state, &caller, &ns, ApiOperation::NamespaceRead).await?;
    let rows = service(&state)?
        .list(&caller.session.principal, ns, &caller.ctx)
        .await?;
    Ok(Json(ConnectionListBody {
        items: rows.iter().map(ConnectionBody::from).collect(),
    }))
}
pub async fn get<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((ns, raw_name)): Path<(String, String)>,
    query: Result<Query<RevisionQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Json<ConnectionBody>, ApiError> {
    let ns = namespace(&state, &caller, &ns, ApiOperation::NamespaceRead).await?;
    let Query(query) =
        query.map_err(|_| AppError::NotAllowed("invalid connection revision query"))?;
    let row = service(&state)?
        .get(
            &caller.session.principal,
            ns,
            &name(&raw_name)?,
            query.revision,
            &caller.ctx,
        )
        .await?;
    Ok(Json(ConnectionBody::from(&row)))
}
pub async fn put<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((ns, raw_name)): Path<(String, String)>,
    ApiJson(body): ApiJson<PutConnectionBody>,
) -> Result<Json<ConnectionBody>, ApiError> {
    let ns = namespace(&state, &caller, &ns, ApiOperation::DatabaseManage).await?;
    let write = ConnectionWrite {
        name: name(&raw_name)?,
        expected_revision: body.expected_revision,
        request_id: request_id(&body.request_id)?,
        config: body.config.into_domain()?,
        status: body.status.into(),
    };
    let row = service(&state)?
        .put(&caller.session.principal, ns, write, &caller.ctx)
        .await?;
    Ok(Json(ConnectionBody::from(&row)))
}
pub async fn delete<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path((ns, raw_name)): Path<(String, String)>,
    ApiJson(body): ApiJson<DeleteConnectionBody>,
) -> Result<Json<ConnectionBody>, ApiError> {
    let ns = namespace(&state, &caller, &ns, ApiOperation::DatabaseManage).await?;
    let service = service(&state)?;
    let name = name(&raw_name)?;
    let current = service
        .get(&caller.session.principal, ns, &name, None, &caller.ctx)
        .await?;
    let row = service
        .put(
            &caller.session.principal,
            ns,
            ConnectionWrite {
                name,
                expected_revision: body.expected_revision,
                request_id: request_id(&body.request_id)?,
                config: current.config,
                status: ConnectionStatus::Deleted,
            },
            &caller.ctx,
        )
        .await?;
    Ok(Json(ConnectionBody::from(&row)))
}

pub async fn drivers<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(ns): Path<String>,
) -> Result<Json<DatabaseDriverListBody>, ApiError> {
    namespace(&state, &caller, &ns, ApiOperation::NamespaceRead).await?;
    Ok(Json(DatabaseDriverListBody {
        items: [DatabaseDriverBody {
            id: "postgresql",
            operations: ["query", "execute"],
            parameter_types: [
                "null",
                "boolean",
                "integer",
                "decimal",
                "float",
                "text",
                "timestamp",
                "date",
                "binary",
                "json",
            ],
            default_limits: sagat_domain::database::DatabaseLimits::default().into(),
            maximum_limits: sagat_domain::database::DatabaseLimits::MAX.into(),
        }],
    }))
}
