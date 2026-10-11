//! What the suites of this feature share.

use sagat_application::definitions::WorkflowRecord;
use sagat_application::error::{AppError, Refusal};
use sagat_domain::definition::WorkflowId;
use sagat_domain::identity::{Environment, Namespace, Principal, Role};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{World, activity, ctx, document};

/// A workflow document with one activity, `marker` making the digest differ between versions.
pub fn doc(name: &str, marker: &str) -> Value {
    let mut doc = document(name, "a", vec![activity("a")]);
    doc["metadata"]["description"] = json!(marker);
    doc
}

pub fn bytes(doc: &Value) -> Vec<u8> {
    serde_json::to_vec(doc).expect("json")
}

/// A namespace in `environment`, the way an operator creates one.
pub async fn namespace(w: &World, name: &str, environment: Environment) -> Namespace {
    w.add_namespace_in(name, environment).await
}

/// A draft of `name` in the namespace with the slug `slug`.
pub async fn draft(w: &World, slug: &str, name: &str, doc: &Value) -> WorkflowRecord {
    w.defs
        .create_workflow(
            &w.admin,
            slug,
            name,
            &bytes(doc),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("a draft")
}

/// Replaces the draft of a workflow with `doc`, as `who` would.
pub async fn edit(
    w: &World,
    who: &Principal,
    workflow: WorkflowId,
    revision: u64,
    doc: &Value,
) -> u64 {
    w.defs
        .update_draft(who, workflow, revision, &bytes(doc), &ctx())
        .await
        .expect("an edit")
        .workflow
        .draft_revision()
}

pub async fn developer(w: &World, name: &str) -> Principal {
    w.member(name, Role::Developer).await
}

pub async fn operator(w: &World, name: &str) -> Principal {
    w.member(name, Role::Operator).await
}

/// The refusal inside an error, panicking for any other error.
pub fn refusal(error: AppError) -> Refusal {
    match error {
        AppError::Versioning(refusal) => refusal,
        other => panic!("expected a refusal, got {other:?}"),
    }
}
