//! The fleet activation of the two flow-control capabilities through the generic operator command
//! `control-plane capability-activate <token>` (task T022-116; decisions D022-136 and D022-137; ADR-0089 decision 6,
//! ADR-0098 decision 4), against the real registry. Every check the removed `flow-control-activate` made is proved here
//! through the generic command: the operator, a live process of each kind, no live process lacking the capability, the
//! refusal of the runtime role, the audit row; with what the generic command adds (`--expect-live`, `--dry-run`) and the
//! prerequisite the old command did not check (an active digest key version), before the census and under the epoch lock.

use std::time::Duration;

use sagat_application::instances::{Capability, InstanceKind};
use sagat_control_plane::RunError;
use sagat_control_plane::capability_activation::{
    ActivationArgs, ActivationOutcome, capability_activate,
};
use sagat_control_plane::config::DatabaseConfig;

use crate::support::instances::{Registry, registry};
use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD};

/// The two capabilities, each with its registry token.
const GATES: [(Capability, &str); 2] = [
    (
        Capability::FlowControlConcurrencyV1,
        "flow-control-concurrency-v1",
    ),
    (Capability::FlowControlRateV1, "flow-control-rate-v1"),
];

fn database(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url,
        max_connections: 2,
        statement_timeout: Duration::from_secs(5),
    }
}

fn args(token: &str, expect_live: Option<u32>, dry_run: bool) -> ActivationArgs {
    ActivationArgs {
        capability: token.to_owned(),
        operator: Some("ops:ana".to_owned()),
        expect_live,
        dry_run,
    }
}

async fn activate(r: &Registry, a: &ActivationArgs) -> Result<ActivationOutcome, RunError> {
    capability_activate(&database(r.db.url.clone()), a).await
}

fn declined(result: Result<ActivationOutcome, RunError>) -> String {
    match result {
        Err(RunError::Declined(message)) => message,
        other => panic!("expected a refusal in words, got {other:?}"),
    }
}

async fn recorded(r: &Registry, token: &str) -> Option<Vec<String>> {
    sqlx::query_scalar("SELECT kinds FROM platform_requirements WHERE capability = $1")
        .bind(token)
        .fetch_optional(&r.db.pool)
        .await
        .expect("requirements")
}

/// Activates digest key version 1, as `flow_control_key_activate` does for the operator (runbook section 1.2, step 2).
async fn key_version_active(r: &Registry) {
    let (outcome,): (String,) = sqlx::query_as(
        "SELECT out_outcome FROM flow_control_key_activate(1::smallint, 'operator')",
    )
    .fetch_one(&r.db.pool)
    .await
    .expect("the digest key version is activated");
    assert_eq!(outcome, "ACTIVATED");
}

/// Decision D022-137 items 1 to 5, for each capability: an empty fleet, a fleet with one kind only, a live process that
/// lacks the capability and a count that is not the one expected are each refused with nothing recorded; a dry run
/// changes nothing; the activation records both kinds once, with the operator in the history, and a second run says it
/// is recorded already.
#[tokio::test]
async fn each_flow_control_capability_needs_a_capable_live_process_of_each_kind_and_is_recorded_once()
 {
    for (capability, token) in GATES {
        let r = registry().await;
        key_version_active(&r).await;
        // an empty fleet: nothing would serve after it, even with `--expect-live 0`
        let message = declined(activate(&r, &args(token, Some(0), false)).await);
        assert!(
            message.contains("no process of the kinds") && message.contains(token),
            "{token}: {message}"
        );
        // an engine alone: the control plane kind has no live process
        r.serving(InstanceKind::ExecutionEngine, &[capability])
            .await;
        let message = declined(activate(&r, &args(token, Some(1), false)).await);
        assert!(
            message.contains("none of CONTROL_PLANE is live"),
            "{token}: {message}"
        );
        // a control plane of an older build (it does not list the capability) is named by its instance id
        let older = r
            .serving(
                InstanceKind::ControlPlane,
                &[Capability::RunInputValidationV1],
            )
            .await;
        for dry_run in [false, true] {
            let message = declined(activate(&r, &args(token, Some(2), dry_run)).await);
            assert!(
                message.contains(&format!("1 live process(es) do not list {token}"))
                    && message.contains(&older.to_string()),
                "{token} dry_run={dry_run}: {message}"
            );
        }
        r.age(older, 40).await;
        r.serving(InstanceKind::ControlPlane, &[capability]).await;
        // the count the operator expects
        for expected in [1, 3] {
            let message = declined(activate(&r, &args(token, Some(expected), false)).await);
            assert!(
                message.contains("2 control plane(s) and execution engine(s) are live"),
                "{token}: {message}"
            );
        }
        assert_eq!(recorded(&r, token).await, None, "{token}");
        assert!(
            matches!(
                activate(&r, &args(token, Some(2), true)).await,
                Ok(ActivationOutcome::WouldActivate { live: 2, .. })
            ),
            "{token}"
        );
        assert_eq!(recorded(&r, token).await, None, "{token}: a dry run");
        match activate(&r, &args(token, Some(2), false))
            .await
            .expect("activated")
        {
            ActivationOutcome::Activated { entry, live, .. } => {
                assert_eq!(entry.capability, capability);
                assert_eq!(live, 2);
            }
            other => panic!("{token}: {other:?}"),
        }
        let mut kinds = recorded(&r, token).await.expect("recorded");
        kinds.sort();
        assert_eq!(kinds, ["CONTROL_PLANE", "EXECUTION_ENGINE"], "{token}");
        let (actor, role): (String, String) = sqlx::query_as(
            "SELECT actor, database_user FROM platform_requirement_history WHERE capability = $1",
        )
        .bind(token)
        .fetch_one(&r.db.pool)
        .await
        .expect("one history row");
        assert_eq!(actor, "ops:ana", "{token}");
        assert!(!role.is_empty(), "{token}");
        assert!(
            matches!(
                activate(&r, &args(token, Some(2), false)).await,
                Ok(ActivationOutcome::AlreadyActive { .. })
            ),
            "{token}"
        );
        assert_eq!(r.history().await, 1, "{token}: one activation");
    }
}

/// Decision D022-137 item 6: with no digest key version `ACTIVE`, every gated decision would be refused as unavailable,
/// so the activation and its dry run are refused, naming the missing key version, with nothing recorded; once a version
/// is active the same command passes.
#[tokio::test]
async fn a_flow_control_capability_is_refused_while_no_digest_key_version_is_active() {
    for (capability, token) in GATES {
        let r = registry().await;
        for kind in InstanceKind::ALL {
            r.serving(kind, &[capability]).await;
        }
        for dry_run in [true, false] {
            let message = declined(activate(&r, &args(token, Some(2), dry_run)).await);
            assert!(
                message.contains(&format!(
                    "{token} needs a digest key version in force, and none is active"
                )) && message.contains("Nothing was changed"),
                "{token} dry_run={dry_run}: {message}"
            );
        }
        assert_eq!(recorded(&r, token).await, None, "{token}");
        key_version_active(&r).await;
        assert!(
            matches!(
                activate(&r, &args(token, Some(2), false)).await,
                Ok(ActivationOutcome::Activated { .. })
            ),
            "{token}"
        );
    }
}

/// Decision D022-137 item 6, under the lock: the prerequisite is read again under the epoch lock. A transaction that
/// holds the epoch and leaves no version `ACTIVE` (a fixture: no command of this build does that) commits while the
/// activation waits for the lock, after its first read found the version active; the activation is refused.
#[tokio::test]
async fn the_digest_key_version_is_read_again_under_the_epoch_lock() {
    let r = registry().await;
    key_version_active(&r).await;
    for kind in InstanceKind::ALL {
        r.serving(kind, &[Capability::FlowControlRateV1]).await;
    }
    let mut holder = r.db.pool.begin().await.expect("a transaction");
    sqlx::query("SELECT platform_requirement_lock()")
        .execute(&mut *holder)
        .await
        .expect("the epoch is held");
    sqlx::query("UPDATE flow_control_key_versions SET state = 'RETIRING' WHERE state = 'ACTIVE'")
        .execute(&mut *holder)
        .await
        .expect("no version active, not committed");
    let url = r.db.url.clone();
    let activation = tokio::spawn(async move {
        capability_activate(
            &database(url),
            &args("flow-control-rate-v1", Some(2), false),
        )
        .await
    });
    r.wait_for_lock_waiters(1, "the activation waits for the epoch lock")
        .await;
    holder.commit().await.expect("committed");
    let message = declined(activation.await.expect("the activation ended"));
    assert!(
        message.contains("needs a digest key version in force"),
        "{message}"
    );
    assert_eq!(recorded(&r, "flow-control-rate-v1").await, None);
}

/// The old command's refusal of the runtime role, through the generic command: the activation and its dry run say to
/// use the schema-owning role, and nothing is recorded.
#[tokio::test]
async fn the_runtime_role_is_told_to_use_the_owner_role() {
    let r = registry().await;
    key_version_active(&r).await;
    for kind in InstanceKind::ALL {
        r.serving(kind, &[Capability::FlowControlConcurrencyV1])
            .await;
    }
    let runtime = database(r.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD));
    for dry_run in [false, true] {
        let message = declined(
            capability_activate(
                &runtime,
                &args("flow-control-concurrency-v1", Some(2), dry_run),
            )
            .await,
        );
        assert!(
            message.contains("schema-owning database role"),
            "dry_run={dry_run}: {message}"
        );
    }
    assert_eq!(recorded(&r, "flow-control-concurrency-v1").await, None);
}

/// The operator is required (the activation is recorded with who made it) and must be a subject.
#[tokio::test]
async fn the_operator_is_required_and_must_be_a_subject() {
    let r = registry().await;
    key_version_active(&r).await;
    let without = ActivationArgs {
        operator: None,
        ..args("flow-control-rate-v1", Some(2), false)
    };
    let message = declined(activate(&r, &without).await);
    assert!(message.contains("--operator NAME is required"), "{message}");
    let unprintable = ActivationArgs {
        operator: Some("ops:\u{7}ana".to_owned()),
        ..args("flow-control-rate-v1", Some(2), false)
    };
    assert!(matches!(
        activate(&r, &unprintable).await,
        Err(RunError::Config(_))
    ));
    assert_eq!(recorded(&r, "flow-control-rate-v1").await, None);
}
