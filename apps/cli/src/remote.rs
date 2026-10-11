//! What the platform knows, read through its REST API and reduced to what the commands use.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::client::{Client, with_query};

/// Page size asked for, and the most pages read: a bound on what one command pulls.
const PAGE: &str = "50";
const MAX_PAGES: usize = 100;

/// The active version of a workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Active {
    pub number: u32,
    pub digest: String,
}

/// A workflow of the namespace, as the platform keeps it.
#[derive(Debug, Clone)]
pub struct RemoteWorkflow {
    pub id: String,
    pub name: String,
    pub revision: u64,
    pub ir: Value,
    pub active: Option<Active>,
}

impl RemoteWorkflow {
    fn of(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_owned(),
            name: value.get("name")?.as_str()?.to_owned(),
            revision: value.get("draftRevision")?.as_u64()?,
            ir: value.get("ir").cloned().unwrap_or(Value::Null),
            active: value.get("activeVersion").and_then(|a| {
                Some(Active {
                    number: u32::try_from(a.get("versionNumber")?.as_u64()?).ok()?,
                    digest: a.get("digest")?.as_str()?.to_owned(),
                })
            }),
        })
    }
}

/// A namespace of the tenant.
#[derive(Debug, Clone)]
pub struct NamespaceInfo {
    pub id: String,
    pub slug: String,
    /// `dev`, `staging` or `prod`, when the platform said.
    pub environment: Option<String>,
}

/// Every item of a listing, page after page.
async fn pages(client: &Client, path: &str, params: &[(&str, &str)]) -> Result<Vec<Value>, String> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut query: Vec<(&str, &str)> = params.to_vec();
        query.push(("limit", PAGE));
        if let Some(cursor) = &cursor {
            query.push(("cursor", cursor));
        }
        let (_, body) = client
            .call("GET", &with_query(path, &query), None, None)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(page) = body.get("items").and_then(Value::as_array) {
            items.extend(page.iter().cloned());
        }
        match body.get("nextCursor").and_then(Value::as_str) {
            Some(next) => cursor = Some(next.to_owned()),
            None => return Ok(items),
        }
    }
    Err(format!(
        "{path} lists more than {} items; this tool reads at most that many",
        MAX_PAGES * 50
    ))
}

/// The workflows of the namespace the client works in, by name.
pub async fn workflows(client: &Client) -> Result<BTreeMap<String, RemoteWorkflow>, String> {
    Ok(pages(client, "/workflows", &[])
        .await?
        .iter()
        .filter_map(RemoteWorkflow::of)
        .map(|w| (w.name.clone(), w))
        .collect())
}

/// The namespaces of the tenant.
pub async fn namespaces(client: &Client) -> Result<Vec<NamespaceInfo>, String> {
    Ok(pages(client, "/namespaces", &[])
        .await?
        .iter()
        .filter_map(|n| {
            Some(NamespaceInfo {
                id: n.get("id")?.as_str()?.to_owned(),
                slug: n.get("slug")?.as_str()?.to_owned(),
                environment: n
                    .get("environment")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect())
}

/// The document a published version froze.
pub async fn version(client: &Client, id: &str, number: u32) -> Result<Value, String> {
    let (_, body) = client
        .call(
            "GET",
            &format!("/workflows/{id}/versions/{number}"),
            None,
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
    body.get("document")
        .cloned()
        .ok_or_else(|| format!("the platform's answer for version {number} has no document"))
}

/// One workflow by name.
pub async fn workflow(client: &Client, name: &str) -> Result<RemoteWorkflow, String> {
    workflows(client)
        .await?
        .remove(name)
        .ok_or_else(|| format!("the namespace has no workflow named '{name}'"))
}

/// The numbers and digests of every published version of a workflow.
pub async fn versions(client: &Client, id: &str) -> Result<Vec<(u32, String)>, String> {
    Ok(pages(client, &format!("/workflows/{id}/versions"), &[])
        .await?
        .iter()
        .filter_map(|v| {
            Some((
                u32::try_from(v.get("versionNumber")?.as_u64()?).ok()?,
                v.get("digest")?.as_str()?.to_owned(),
            ))
        })
        .collect())
}

/// A page of a listing newest first, once (for "is there an open request", not for everything).
pub async fn first_page(client: &Client, path: &str) -> Result<Vec<Value>, String> {
    let (_, body) = client
        .call("GET", &with_query(path, &[("limit", PAGE)]), None, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(body
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}
