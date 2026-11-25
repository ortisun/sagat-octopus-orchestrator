//! The live concurrency keys of a namespace (feature 022, FR-022-022; T022-026, T022-111), and the opening both key
//! listings share (this one and `routes/rate_limits.rs`, decision D022-068 item 6).
//!
//! A namespace that belongs to another tenant is a `404` like one that does not exist. Keys appear only as labels, and
//! only to a caller who may reveal personal data in the namespace (D022-060); every other caller reads one row per group
//! (`view: GROUPS`, decisions D022-061, D022-086). The cursor is sealed, of its listing's own kind and view (D022-059).

use axum::Json;
use axum::extract::{Path, Query, State};
use sagat_application::api_admission::{ApiOperation, ApiTarget};
use sagat_application::flow_control_keys::{KeyListing, ListingPosition};
use sagat_domain::identity::NamespaceId;
use sagat_domain::workflow::ResolvedGroup;
use serde::Deserialize;

use crate::admission::admit;
use crate::dto_flow_control::ConcurrencyKeyPageBody;
use crate::error::ApiError;
use crate::extract::Caller;
use crate::governance::Governance;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConcurrencyKeyQuery {
    /// An explicit group or a derived one (`=<workflow>` or `=<workflow>/<node>`).
    pub group: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

/// A key listing's request once opened: its namespace, group, place and page size, the caller admitted.
pub(crate) struct KeyListingQuery {
    pub namespace: NamespaceId,
    pub group: Option<ResolvedGroup>,
    pub after: Option<ListingPosition>,
    pub limit: u32,
}

/// Opens a request of the key listing of `listing`'s gate, the same way for both gates (decision D022-068 item 6): the
/// namespace and the group are read; a cursor is the place sealed for this namespace of this tenant, of this gate's
/// listing in either view (D022-059, D022-086), anything else, altered, or sealed for another namespace or the other
/// gate is a `400` decided before admission, the same whether the namespace exists or not; a group's listing resumes
/// only after a place of that group; the caller is admitted for `ops:read`; and a cursor sealed under a digest key
/// version out of force is a `400` too, asked after admission (it reads the database, D022-063). Whether the cursor's
/// view is the caller's is the listing's own question (it reads the reveal right).
///
/// # Errors
/// The `404`, `400`, `422` and admission refusals above.
pub(crate) async fn open_key_listing<B: Governance>(
    caller: &Caller,
    state: &B,
    namespace: &str,
    query: &ConcurrencyKeyQuery,
    listing: KeyListing,
) -> Result<KeyListingQuery, ApiError> {
    let service = state.operations().ok_or_else(ApiError::not_found)?;
    let namespace: NamespaceId = namespace.parse().map_err(|_| ApiError::not_found())?;
    let group = query
        .group
        .as_deref()
        .map(|g| {
            ResolvedGroup::parse(g)
                .map_err(|_| ApiError::validation("the group is not a valid group name"))
        })
        .transpose()?;
    let after = query
        .cursor
        .as_deref()
        .map(|c| {
            service
                .key_position(&caller.session.principal, namespace, listing, c)
                .ok_or_else(|| ApiError::bad_request("the cursor is not valid"))
        })
        .transpose()?;
    if let (Some(group), Some(after)) = (&group, &after)
        && after.position.group != *group
    {
        return Err(ApiError::bad_request("the cursor is not valid"));
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    admit(
        state,
        caller,
        ApiOperation::OpsRead,
        ApiTarget::Namespace(namespace),
    )
    .await?;
    if let Some(cursor) = query.cursor.as_deref()
        && !service
            .key_cursor_in_force(cursor)
            .await
            .map_err(ApiError::from)?
    {
        return Err(ApiError::bad_request("the cursor is not valid"));
    }
    Ok(KeyListingQuery {
        namespace,
        group,
        after,
        limit,
    })
}

/// The refusal of a cursor written for the other view of a key listing: the caller's reveal right changed between two
/// pages, so the listing must be read again from its start (batch 4 correctness nit 1).
pub(crate) const OTHER_VIEW: &str =
    "the cursor was written for the other view of this listing: read it again without a cursor";

/// `GET /namespaces/{namespaceId}/concurrency-keys`
pub async fn list_concurrency_keys<B: Governance>(
    caller: Caller,
    State(state): State<B>,
    Path(namespace): Path<String>,
    Query(query): Query<ConcurrencyKeyQuery>,
) -> Result<Json<ConcurrencyKeyPageBody>, ApiError> {
    let service = state.operations().ok_or_else(ApiError::not_found)?;
    let opened =
        open_key_listing(&caller, &state, &namespace, &query, KeyListing::Concurrency).await?;
    let page = service
        .concurrency_keys(
            &caller.session.principal,
            opened.namespace,
            opened.group.as_ref(),
            opened.after.as_ref(),
            Some(opened.limit + 1),
            &caller.ctx,
        )
        .await?
        // a cursor of the other view: what the caller may see changed between two pages (batch 4 nit 1)
        .ok_or_else(|| ApiError::bad_request(OTHER_VIEW))?;
    let body = ConcurrencyKeyPageBody::of(&page, opened.limit as usize, |last| {
        service.concurrency_key_cursor(&caller.session.principal, opened.namespace, &page, last)
    })?;
    Ok(Json(body))
}
