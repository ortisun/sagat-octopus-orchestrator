//! What publication of a gated definition reads from the namespace (0.3.0 slice): the namespace warnings
//! `CONCURRENCY_LIMIT_MISMATCH` and `RATE_LIMIT_MISMATCH` (T022-069, T022-025, G1187, D022-041) and the ceiling of
//! node-level declarations (T022-013, D022-040); and the quota resources of the gate, set at and over their bound, with
//! the usage the quota view reads (T022-013, SC-022-011).

use sagat_application::clock::SystemClock;
use sagat_application::definitions::{DefinitionService, IrError, Published};
use sagat_application::error::AppError;
use sagat_application::flow_control::FlowControlSwitches;
use sagat_application::instances::{Activation, Capability, InstanceKind, InstanceStore};
use sagat_domain::identity::{Principal, Subject};
use sagat_domain::quota::QuotaResource;
use sagat_domain::workflow::{DefinitionWarning, GateKind, WorkflowError};
use sagat_infra_ir::JsonIrCodec;
use sagat_infra_postgres::{PgInstanceStore, PgRunStore};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::engine::{World, ctx, world};
use crate::support::instances::{nothing, registration};

const BOTH_ON: FlowControlSwitches = FlowControlSwitches {
    concurrency_keys: true,
    rate_limits: true,
};
const KINDS: [InstanceKind; 2] = [InstanceKind::ControlPlane, InstanceKind::ExecutionEngine];

fn definitions(w: &World) -> DefinitionService<PgRunStore, SystemClock, JsonIrCodec> {
    DefinitionService::new(w.store.clone(), SystemClock, JsonIrCodec).with_flow_control(BOTH_ON)
}

/// Every live control plane and engine runs both gates, and the operator activated them (018's registry).
async fn activate_both(w: &World) {
    let registry = PgInstanceStore::new(w.db.pool.clone());
    let capabilities = [
        Capability::FlowControlConcurrencyV1,
        Capability::FlowControlRateV1,
    ];
    for kind in KINDS {
        registry
            .register(&registration(Uuid::new_v4(), kind, &capabilities))
            .await
            .expect("processes that run the gates");
    }
    let actor = Subject::parse("operator:flow-control").expect("subject");
    for capability in capabilities {
        registry
            .activate(
                Activation {
                    capability,
                    kinds: &KINDS,
                    actor: &actor,
                },
                nothing,
            )
            .await
            .expect("activated");
    }
}

/// A definition whose run-level `concurrency` names `group` with `limit`, and whose activity is rate-limited in
/// `rate_group` at `rate` per `per`.
fn grouped(name: &str, group: &str, limit: u32, rate_group: &str, rate: u32, per: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "fetch",
        "concurrency": {"group": group, "key": "input.accountId", "limit": limit},
        "nodes": [{
            "id": "fetch", "type": "activity", "activity": "provider.fetch",
            "rateLimit": {"group": rate_group, "key": "input.providerId", "limit": rate, "per": per}
        }]
    })
}

async fn publish_as(
    w: &World,
    principal: &Principal,
    namespace: &str,
    doc: &Value,
) -> Result<Published, AppError> {
    let service = definitions(w);
    let name = doc["metadata"]["name"].as_str().expect("name");
    let workflow = service
        .create_workflow(
            principal,
            namespace,
            name,
            &serde_json::to_vec(doc).expect("json"),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("created")
        .workflow
        .id();
    service
        .publish(
            principal,
            workflow,
            1,
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
}

async fn publish(w: &World, doc: &Value) -> Result<Published, AppError> {
    publish_as(w, &w.admin, "payments", doc).await
}

fn mismatches(published: &Published) -> Vec<(&'static str, Option<String>, String, String)> {
    published
        .warnings
        .iter()
        .filter_map(|warning| match warning {
            DefinitionWarning::ConcurrencyLimitMismatch { node, group, other } => Some((
                "CONCURRENCY_LIMIT_MISMATCH",
                node.as_ref().map(ToString::to_string),
                group.as_str().to_owned(),
                other.to_string(),
            )),
            DefinitionWarning::RateLimitMismatch { node, group, other } => Some((
                "RATE_LIMIT_MISMATCH",
                node.as_ref().map(ToString::to_string),
                group.as_str().to_owned(),
                other.to_string(),
            )),
            _ => None,
        })
        .collect()
}

/// G1187, T022-069, T022-025: a group another active definition of the namespace declares at the same level with
/// another limit (or, for a rate limit, another window) is warned, once per declaration, naming that workflow; the same
/// values, a derived group, another level, and another tenant's namespace are not. The warning never refuses.
#[tokio::test]
async fn a_group_declared_elsewhere_with_another_limit_is_warned_and_nothing_else_is() {
    let w = world().await;
    activate_both(&w).await;
    let first = publish(
        &w,
        &grouped("first", "accounts", 5, "provider-api", 10, "second"),
    )
    .await
    .expect("the first declaration of a group has nothing to differ from");
    assert!(mismatches(&first).is_empty(), "{:?}", first.warnings);

    let same = publish(
        &w,
        &grouped("same", "accounts", 5, "provider-api", 10, "second"),
    )
    .await
    .expect("published");
    assert!(
        mismatches(&same).is_empty(),
        "the same values: {:?}",
        same.warnings
    );

    let other_limit = publish(
        &w,
        &grouped("other-limit", "accounts", 3, "provider-api", 10, "minute"),
    )
    .await
    .expect("a warning never refuses");
    assert_eq!(
        mismatches(&other_limit),
        [
            (
                "CONCURRENCY_LIMIT_MISMATCH",
                None,
                "accounts".to_owned(),
                "first".to_owned()
            ),
            (
                "RATE_LIMIT_MISMATCH",
                Some("fetch".to_owned()),
                "provider-api".to_owned(),
                "first".to_owned()
            ),
        ],
        "a different limit, and a different window with the same limit"
    );

    // the run-level group `provider-api` is not the step-level one: levels never share a key row
    let other_level = publish(
        &w,
        &json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": "other-level"},
            "entrypoint": "fetch",
            "rateLimit": {"group": "provider-api", "key": "input.providerId", "limit": 1, "per": "hour"},
            "nodes": [{"id": "fetch", "type": "activity", "activity": "provider.fetch",
                       "concurrency": {"key": "input.accountId", "limit": 9}}]
        }),
    )
    .await
    .expect("published");
    assert!(
        mismatches(&other_level).is_empty(),
        "another level, and a derived group: {:?}",
        other_level.warnings
    );

    // another tenant's namespace that declares the same group differently is not this namespace
    let elsewhere = publish_as(
        &w,
        &w.other_admin,
        "payments",
        &grouped("elsewhere", "accounts", 1, "provider-api", 1, "hour"),
    )
    .await
    .expect("published in the other tenant");
    assert!(
        mismatches(&elsewhere).is_empty(),
        "{:?}",
        elsewhere.warnings
    );

    // validation reports what publication would
    let service = definitions(&w);
    let draft = service
        .create_workflow(
            &w.admin,
            "payments",
            "draft-only",
            &serde_json::to_vec(&grouped(
                "draft-only",
                "accounts",
                2,
                "provider-api",
                10,
                "second",
            ))
            .expect("json"),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("created");
    let validation = service
        .validate(&w.admin, draft.workflow.id(), None, &ctx())
        .await
        .expect("a validation");
    let codes: Vec<_> = validation
        .warnings
        .iter()
        .map(|warning| (warning.code, warning.path.clone()))
        .collect();
    assert!(
        codes.contains(&(
            "CONCURRENCY_LIMIT_MISMATCH",
            Some("/concurrency".to_owned())
        )),
        "{codes:?}"
    );
    assert!(validation.problems.is_empty(), "{:?}", validation.problems);

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM flow_control_group_declarations")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(
        rows, 9,
        "two explicit groups for each of first, same, other-limit and elsewhere, one for other-level; none for a draft"
    );
}

/// T022-013 (`flow_control_node_declarations`): 16 node-level declarations of one kind by default; a 17th is refused at
/// publication under the code of its kind, before a version exists; a namespace that allows more publishes it.
#[tokio::test]
async fn the_namespace_ceiling_of_node_declarations_is_checked_at_publication() {
    let w = world().await;
    activate_both(&w).await;
    let chain = |name: &str, count: usize| {
        let nodes: Vec<Value> = (0..count)
            .map(|i| {
                let mut node = json!({
                    "id": format!("n{i}"), "type": "activity", "activity": "provider.fetch",
                    "concurrency": {"key": "input.accountId", "limit": 1}
                });
                if i + 1 < count {
                    node["next"] = json!(format!("n{}", i + 1));
                }
                node
            })
            .collect();
        json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": name},
            "entrypoint": "n0",
            "nodes": nodes
        })
    };
    publish(&w, &chain("at-bound", 16))
        .await
        .expect("16 is the default");
    match publish(&w, &chain("over-bound", 17)).await {
        Err(AppError::InvalidDefinition(error)) => assert!(
            matches!(
                *error,
                IrError::Invalid(WorkflowError::InvalidFlowControl {
                    kind: GateKind::Concurrency,
                    node: None,
                    ..
                })
            ),
            "{error:?}"
        ),
        other => panic!("expected CONCURRENCY_POLICY_INVALID, got {other:?}"),
    }
    let versions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM workflow_versions v JOIN workflows w ON w.id = v.workflow_id \
         WHERE w.name = 'over-bound'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(versions, 0, "refused before a version exists");

    // an operator grants the hard ceiling, and the namespace takes 17
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
         VALUES ($1, 'flow_control_node_declarations', 64, 'operator')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("ceiling granted");
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::FlowControlNodeDeclarations,
            17,
            &ctx(),
        )
        .await
        .expect("set");
    publish(&w, &chain("over-bound-allowed", 17))
        .await
        .expect("the namespace allows 17");
}

/// T022-013, SC-022-011: each resource of the gate is set at its hard ceiling and refused one over it (once an
/// operator granted it), a count may be zero and a policy may not, and the quota view reads the usage of each count
/// from the waiter and key rows.
#[tokio::test]
async fn each_quota_resource_of_the_gate_is_set_at_its_bound_refused_over_it_and_counted() {
    let w = world().await;
    // it reads the live-key and waiter usages, shown only with the reveal grant (decision D022-133)
    crate::support::flow_control::reveal_to_admin(&w).await;
    let resources = [
        QuotaResource::QueuedRuns,
        QuotaResource::ConcurrencyLiveKeys,
        QuotaResource::FlowControlKeyWaiters,
        QuotaResource::ConcurrencyLimit,
        QuotaResource::FlowControlQueueTimeoutMs,
        QuotaResource::FlowControlNodeDeclarations,
    ];
    for resource in resources {
        let max = resource.maximum();
        sqlx::query(
            "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
             VALUES ($1, $2, $3, 'operator')",
        )
        .bind(w.admin.tenant_id().as_uuid())
        .bind(resource.as_str())
        .bind(i64::try_from(max).expect("bounded"))
        .execute(&w.db.pool)
        .await
        .expect("ceiling granted");
        let view = w
            .quotas
            .set(&w.admin, Some(w.namespace.id()), resource, max, &ctx())
            .await
            .unwrap_or_else(|error| panic!("{resource} at its bound: {error:?}"));
        assert_eq!(view.limit, max, "{resource}");
        assert_eq!(
            view.used,
            resource.has_usage().then_some(0),
            "{resource}: nothing waits, no key lives"
        );
        assert!(
            matches!(
                w.quotas
                    .set(&w.admin, Some(w.namespace.id()), resource, max + 1, &ctx())
                    .await,
                Err(AppError::NotAllowed(_))
            ),
            "{resource} over its bound"
        );
        let zero = w
            .quotas
            .set(&w.admin, Some(w.namespace.id()), resource, 0, &ctx())
            .await;
        if resource.has_usage() {
            assert_eq!(zero.expect("a count may be zero").limit, 0, "{resource}");
        } else {
            assert!(
                matches!(zero, Err(AppError::NotAllowed(_))),
                "{resource}: a policy of zero"
            );
        }
    }
}

/// Security review S4, correctness review F7d (2026-11-16): the warning reads this namespace only (another namespace of
/// the same tenant that declares the group differently is not warned about), and the active version of each other
/// workflow only (a workflow whose new active version dropped the group no longer counts).
#[tokio::test]
async fn the_warning_reads_this_namespace_and_the_active_versions_only() {
    let w = world().await;
    activate_both(&w).await;
    let ledger = w.add_namespace("ledger").await;
    publish_as(
        &w,
        &w.admin,
        ledger.slug().as_str(),
        &grouped("other-namespace", "accounts", 1, "provider-api", 1, "hour"),
    )
    .await
    .expect("published in another namespace of the tenant");

    let service = definitions(&w);
    let dropped = service
        .create_workflow(
            &w.admin,
            "payments",
            "dropped",
            &serde_json::to_vec(&grouped(
                "dropped",
                "accounts",
                2,
                "provider-api",
                2,
                "hour",
            ))
            .expect("json"),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("created")
        .workflow
        .id();
    service
        .publish(
            &w.admin,
            dropped,
            1,
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("version 1 declares the groups");
    let ungrouped = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "dropped"},
        "entrypoint": "fetch",
        "nodes": [{"id": "fetch", "type": "activity", "activity": "provider.fetch"}]
    });
    service
        .update_draft(
            &w.admin,
            dropped,
            1,
            &serde_json::to_vec(&ungrouped).expect("json"),
            &ctx(),
        )
        .await
        .expect("draft");
    service
        .publish(
            &w.admin,
            dropped,
            2,
            None,
            &format!("publish-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("version 2, active, declares none");

    let mine = publish(
        &w,
        &grouped("mine", "accounts", 5, "provider-api", 10, "second"),
    )
    .await
    .expect("published");
    assert!(
        mismatches(&mine).is_empty(),
        "neither another namespace nor an inactive version counts: {:?}",
        mine.warnings
    );
}

/// Correctness review F7d and F7e (2026-11-16): the node-declarations ceiling holds for rate limits too, and a value
/// above the default ceiling is refused without an operator's grant (`AboveCeiling`), not only above the hard bound.
#[tokio::test]
async fn the_rate_limit_declarations_and_the_default_ceilings_are_bounded_too() {
    let w = world().await;
    activate_both(&w).await;
    let nodes: Vec<Value> = (0..17)
        .map(|i| {
            let mut node = json!({
                "id": format!("n{i}"), "type": "activity", "activity": "provider.fetch",
                "rateLimit": {"key": "input.providerId", "limit": 1, "per": "second"}
            });
            if i < 16 {
                node["next"] = json!(format!("n{}", i + 1));
            }
            node
        })
        .collect();
    let doc = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "many-rates"},
        "entrypoint": "n0",
        "nodes": nodes
    });
    match publish(&w, &doc).await {
        Err(AppError::InvalidDefinition(error)) => assert!(
            matches!(
                *error,
                IrError::Invalid(WorkflowError::InvalidFlowControl {
                    kind: GateKind::RateLimit,
                    node: None,
                    ..
                })
            ),
            "{error:?}"
        ),
        other => panic!("expected RATE_LIMIT_POLICY_INVALID, got {other:?}"),
    }
    for resource in [
        QuotaResource::QueuedRuns,
        QuotaResource::ConcurrencyLiveKeys,
        QuotaResource::FlowControlKeyWaiters,
        QuotaResource::FlowControlNodeDeclarations,
    ] {
        let default = resource.default_limit();
        w.quotas
            .set(&w.admin, Some(w.namespace.id()), resource, default, &ctx())
            .await
            .unwrap_or_else(|error| panic!("{resource} at its default ceiling: {error:?}"));
        assert!(
            matches!(
                w.quotas
                    .set(
                        &w.admin,
                        Some(w.namespace.id()),
                        resource,
                        default + 1,
                        &ctx()
                    )
                    .await,
                Err(AppError::NotAllowed(_))
            ),
            "{resource} above its default ceiling without a grant"
        );
    }
}

/// Correctness review F6 (2026-11-16): a publication asked again with its own key after the namespace lowered its
/// ceiling of node-level declarations is answered by the replay (as D042-105), not refused; a new key is refused.
#[tokio::test]
async fn a_publication_retried_after_the_ceiling_was_lowered_is_replayed() {
    let w = world().await;
    activate_both(&w).await;
    sqlx::query(
        "INSERT INTO quota_ceilings (tenant_id, resource, max_value, granted_by) \
         VALUES ($1, 'flow_control_node_declarations', 64, 'operator')",
    )
    .bind(w.admin.tenant_id().as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("ceiling granted");
    let set = |value| {
        let w = &w;
        async move {
            w.quotas
                .set(
                    &w.admin,
                    Some(w.namespace.id()),
                    QuotaResource::FlowControlNodeDeclarations,
                    value,
                    &ctx(),
                )
                .await
                .expect("set");
        }
    };
    set(17).await;
    let nodes: Vec<Value> = (0..17)
        .map(|i| {
            let mut node = json!({
                "id": format!("n{i}"), "type": "activity", "activity": "provider.fetch",
                "concurrency": {"key": "input.accountId", "limit": 1}
            });
            if i < 16 {
                node["next"] = json!(format!("n{}", i + 1));
            }
            node
        })
        .collect();
    let doc = json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "retried"},
        "entrypoint": "n0",
        "nodes": nodes
    });
    let service = definitions(&w);
    let workflow = service
        .create_workflow(
            &w.admin,
            "payments",
            "retried",
            &serde_json::to_vec(&doc).expect("json"),
            &format!("create-{}", Uuid::now_v7()),
            &ctx(),
        )
        .await
        .expect("created")
        .workflow
        .id();
    let first = service
        .publish(&w.admin, workflow, 1, None, "publish-retried-1", &ctx())
        .await
        .expect("published under 17");
    set(16).await;
    let again = service
        .publish(&w.admin, workflow, 1, None, "publish-retried-1", &ctx())
        .await
        .expect("the same request is replayed");
    assert_eq!(again.version.id(), first.version.id());
    assert!(!again.created);
    assert!(
        matches!(
            service
                .publish(&w.admin, workflow, 1, None, "publish-retried-2", &ctx())
                .await,
            Err(AppError::InvalidDefinition(_))
        ),
        "a new request is judged by the ceiling in force"
    );
}
