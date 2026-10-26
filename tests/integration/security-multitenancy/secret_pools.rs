//! Worker pools of a secret (FR-009-002, gap G48): a secret may list the pools allowed to receive
//! it. A step that declares it is not leased to a worker outside them (it stays available for one
//! inside), the value is refused again at `ResolveSecrets`, and the refusal is audited once per
//! worker identity, run, node and attempt. No list is today's behaviour.

use std::sync::Arc;
use std::time::Duration;

use sagat_application::error::AppError;
use sagat_application::worker::Capabilities;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{NamespaceId, Principal, Role};
use serde_json::{Value, json};

use crate::support::engine::{
    Session, World, activity, ctx, document, flaky, subject_of, task_ref, with, world,
};

const VALUE: &str = "sk-live-4eC39HqLyjWDarjtT1zdp7dc";

fn ns(w: &World) -> NamespaceId {
    w.namespace.id()
}

async fn worker(w: &World, name: &str) -> Principal {
    w.member(name, Role::Worker).await
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| (*n).to_owned()).collect()
}

/// Grants `who` the pools `pools` in the first tenant's namespace, bound to every activity.
async fn grant(w: &World, who: &str, pools: &[&str]) {
    w.bindings
        .bind_with_pools(
            &w.admin,
            ns(w),
            who,
            &names(&["*"]),
            Some(&names(pools)),
            &ctx(),
        )
        .await
        .expect("bind with pools");
}

async fn restrict(w: &World, secret: &str, pools: &[&str]) {
    w.secrets
        .set_pools(&w.admin, ns(w), secret, &names(pools), &ctx())
        .await
        .expect("set the pools of the secret");
}

fn flow(secrets: &[&str]) -> Value {
    document(
        "pay",
        "a",
        vec![with(activity("a"), "secrets", json!(secrets))],
    )
}

/// A stored secret `stripe-key` and a started run whose only step declares it.
async fn run_declaring(w: &World) -> RunId {
    w.put_secret("stripe-key", VALUE).await;
    let (wf, _) = w.publish(flow(&["stripe-key"])).await;
    w.start(wf, &json!({})).await
}

async fn poll_len(w: &World, session: &Session) -> usize {
    w.poll(session, 8).await.len()
}

async fn refusals(w: &World) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT actor, reason FROM audit_records WHERE action = 'secret.delivery_refused' \
         ORDER BY actor, created_at",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("audit")
}

async fn step_state(w: &World, run: RunId) -> (String, i32, bool) {
    sqlx::query_as(
        "SELECT status, attempt, lease_token_hash IS NOT NULL FROM steps WHERE run_id = $1",
    )
    .bind(run.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("step")
}

#[tokio::test]
async fn a_secret_without_a_pool_list_is_delivered_to_any_worker_bound_to_the_activity() {
    let w = world().await;
    run_declaring(&w).await;
    let anyone = worker(&w, "anyone").await;
    grant(&w, "anyone", &[]).await;
    let session = w.session_as(&anyone, &["act.a"], 10).await;
    let task = w.one_task(&session).await;
    let got = w
        .workers
        .resolve_secrets(&anyone, &task_ref(&task), None)
        .await
        .expect("resolve");
    assert_eq!(got[0].value.expose(), VALUE.as_bytes());
    assert_eq!(refusals(&w).await, Vec::new());
}

#[tokio::test]
async fn a_worker_of_an_allowed_pool_gets_the_task_and_the_value() {
    let w = world().await;
    run_declaring(&w).await;
    restrict(&w, "stripe-key", &["trusted", "other"]).await;
    let insider = worker(&w, "insider").await;
    grant(&w, "insider", &["trusted"]).await;
    let session = w.session_as(&insider, &["act.a"], 10).await;
    let task = w.one_task(&session).await;
    let got = w
        .workers
        .resolve_secrets(&insider, &task_ref(&task), None)
        .await
        .expect("resolve");
    assert_eq!(got[0].value.expose(), VALUE.as_bytes());
    assert_eq!(refusals(&w).await, Vec::new());
}

#[tokio::test]
async fn a_worker_outside_the_pools_is_not_handed_the_lease_and_an_allowed_one_is() {
    let w = world().await;
    let run = run_declaring(&w).await;
    restrict(&w, "stripe-key", &["trusted"]).await;
    let outsider = worker(&w, "outsider").await;
    grant(&w, "outsider", &["scratch"]).await;
    let bare = worker(&w, "bare").await;
    grant(&w, "bare", &[]).await;
    let insider = worker(&w, "insider").await;
    grant(&w, "insider", &["trusted"]).await;

    // What a worker *says* about itself is not read: advertising `trusted` changes nothing.
    w.bindings
        .bind(&w.admin, ns(&w), "liar", &names(&["*"]), &ctx())
        .await
        .expect("bind");
    let liar = worker(&w, "liar").await;
    let registered = w
        .workers
        .register(
            &liar,
            "payments",
            "liar-1",
            &Capabilities {
                activities: names(&["act.a"]),
                languages: vec![],
                pools: names(&["trusted"]),
                max_inflight: 10,
                protocol_version: "1".into(),
            },
        )
        .await
        .expect("register");
    let lying = Session {
        principal: liar,
        worker_id: registered.worker_id,
        token: registered.session_token.expose().to_owned(),
    };

    let outside = w.session_as(&outsider, &["act.a"], 10).await;
    let no_pool = w.session_as(&bare, &["act.a"], 10).await;
    for _ in 0..3 {
        assert_eq!(poll_len(&w, &outside).await, 0);
        assert_eq!(poll_len(&w, &no_pool).await, 0);
        assert_eq!(poll_len(&w, &lying).await, 0);
    }
    // Nothing was leased and no attempt was spent: the step waits for a worker that may have it.
    assert_eq!(step_state(&w, run).await, ("READY".to_owned(), 0, false));

    let inside = w.session_as(&insider, &["act.a"], 10).await;
    let task = w.one_task(&inside).await;
    assert_eq!(step_state(&w, run).await.0, "LEASED");
    let got = w
        .workers
        .resolve_secrets(&insider, &task_ref(&task), None)
        .await
        .expect("resolve");
    assert_eq!(got[0].value.expose(), VALUE.as_bytes());
}

#[tokio::test]
async fn a_refusal_is_audited_once_per_worker_identity_and_attempt_however_often_it_polls() {
    let w = world().await;
    let run = run_declaring(&w).await;
    restrict(&w, "stripe-key", &["trusted"]).await;
    let one = worker(&w, "outsider-one").await;
    let two = worker(&w, "outsider-two").await;
    grant(&w, "outsider-one", &["scratch"]).await;
    grant(&w, "outsider-two", &[]).await;
    let first = w.session_as(&one, &["act.a"], 10).await;
    // A second session of the same identity is the same worker as far as the trail goes.
    let first_again = w.session_as(&one, &["act.a"], 10).await;
    let second = w.session_as(&two, &["act.a"], 10).await;
    for _ in 0..20 {
        poll_len(&w, &first).await;
        poll_len(&w, &first_again).await;
    }
    for _ in 0..20 {
        poll_len(&w, &second).await;
    }
    let rows = refusals(&w).await;
    let actors: Vec<_> = rows.iter().map(|(a, _)| a.as_str()).collect();
    assert_eq!(actors, ["outsider-one", "outsider-two"], "{rows:?}");
    for (_, reason) in &rows {
        assert!(
            reason.contains("stripe-key")
                && reason.contains("attempt 1")
                && reason.contains("node a"),
            "{reason}"
        );
        assert!(!reason.contains(VALUE));
    }
    let resource: String = sqlx::query_scalar(
        "SELECT DISTINCT resource_id FROM audit_records WHERE action = 'secret.delivery_refused'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("resource");
    assert_eq!(resource, format!("{}/run {run}", ns(&w)));
    assert_eq!(step_state(&w, run).await.0, "READY");
}

#[tokio::test]
async fn a_retry_is_a_new_attempt_and_the_next_refusal_is_a_new_event_but_the_trail_stays_bounded()
{
    let w = world().await;
    w.put_secret("stripe-key", VALUE).await;
    let (wf, _) = w
        .publish(document(
            "pay",
            "a",
            vec![with(flaky("a", 3), "secrets", json!(["stripe-key"]))],
        ))
        .await;
    let run = w.start(wf, &json!({})).await;
    restrict(&w, "stripe-key", &["trusted"]).await;
    let outsider = worker(&w, "outsider").await;
    grant(&w, "outsider", &[]).await;
    let insider = worker(&w, "insider").await;
    grant(&w, "insider", &["trusted"]).await;
    let outside = w.session_as(&outsider, &["act.a"], 10).await;
    let inside = w.session_as(&insider, &["act.a"], 10).await;

    for _ in 0..5 {
        poll_len(&w, &outside).await;
    }
    let task = w.one_task(&inside).await;
    w.fail(&inside, &task, true).await.expect("fail");
    w.age_retry(run, "a").await;
    w.scheduler.tick().await;
    for _ in 0..5 {
        poll_len(&w, &outside).await;
    }
    let reasons: Vec<String> = refusals(&w).await.into_iter().map(|(_, r)| r).collect();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(
        reasons[0].contains("attempt 1") && reasons[1].contains("attempt 2"),
        "{reasons:?}"
    );
}

#[tokio::test]
async fn a_lease_held_by_a_worker_that_left_the_pool_cannot_resolve_the_value() {
    let w = world().await;
    run_declaring(&w).await;
    restrict(&w, "stripe-key", &["trusted"]).await;
    let insider = worker(&w, "insider").await;
    grant(&w, "insider", &["trusted"]).await;
    let session = w.session_as(&insider, &["act.a"], 10).await;
    let task = w.one_task(&session).await;

    // The administrator takes the grant away while the lease is current.
    grant(&w, "insider", &["scratch"]).await;
    let refused = w
        .workers
        .resolve_secrets(&insider, &task_ref(&task), None)
        .await;
    assert!(
        matches!(&refused, Err(AppError::SecretUnavailable { name }) if name == "stripe-key"),
        "{refused:?}"
    );
    let rows = refusals(&w).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].1.contains("resolve"), "{rows:?}");

    // The check is live: the grant back, the same lease resolves.
    grant(&w, "insider", &["trusted"]).await;
    let got = w
        .workers
        .resolve_secrets(&insider, &task_ref(&task), None)
        .await
        .expect("resolve");
    assert_eq!(got[0].value.expose(), VALUE.as_bytes());
}

#[tokio::test]
async fn restricting_a_secret_after_the_lease_was_handed_out_stops_the_next_resolution() {
    let w = world().await;
    run_declaring(&w).await;
    let anyone = worker(&w, "anyone").await;
    grant(&w, "anyone", &[]).await;
    let session = w.session_as(&anyone, &["act.a"], 10).await;
    let task = w.one_task(&session).await;
    restrict(&w, "stripe-key", &["trusted"]).await;
    let refused = w
        .workers
        .resolve_secrets(&anyone, &task_ref(&task), None)
        .await;
    assert!(
        matches!(refused, Err(AppError::SecretUnavailable { .. })),
        "{refused:?}"
    );
    // Lifting the restriction restores today's behaviour.
    restrict(&w, "stripe-key", &[]).await;
    w.workers
        .resolve_secrets(&anyone, &task_ref(&task), None)
        .await
        .expect("resolve");
}

#[tokio::test]
async fn pool_names_belong_to_the_tenant_and_the_namespace_that_granted_them() {
    let w = world().await;
    // Tenant A grants the identity `shared-name` the pool `trusted`.
    grant(&w, "shared-name", &["trusted"]).await;
    // Tenant B has a secret that lists `trusted`, a step that declares it, and a worker with the
    // same identity name that tenant B never put in any pool.
    w.secrets
        .put(
            &w.other_admin,
            w.other_namespace.id(),
            "stripe-key",
            VALUE.as_bytes().to_vec(),
            &ctx(),
        )
        .await
        .expect("secret of tenant B");
    w.secrets
        .set_pools(
            &w.other_admin,
            w.other_namespace.id(),
            "stripe-key",
            &names(&["trusted"]),
            &ctx(),
        )
        .await
        .expect("pools of tenant B");
    let wf = w.publish_as_other_tenant(flow(&["stripe-key"])).await;
    w.start_as_other_tenant(wf, &json!({}))
        .await
        .expect("start");
    let foreign = Principal::new(
        w.other_admin.tenant_id(),
        subject_of("shared-name"),
        Role::Worker,
    );
    sagat_infra_postgres::PgIdentityStore::new(w.db.pool.clone())
        .insert_membership(&sagat_domain::identity::Membership::restore(
            foreign.tenant_id(),
            foreign.subject().clone(),
            Role::Worker,
            crate::support::now(),
        ))
        .await
        .expect("foreign secret-pool worker membership");
    let session = w.session_as(&foreign, &["act.a"], 10).await;
    assert_eq!(
        poll_len(&w, &session).await,
        0,
        "the grant of tenant A means nothing to B"
    );

    // Tenant B's administrator can neither see nor change the pools of tenant A's secrets.
    w.put_secret("a-only", VALUE).await;
    restrict(&w, "a-only", &["trusted"]).await;
    let seen = w
        .secrets
        .pools(&w.other_admin, w.other_namespace.id(), "a-only", &ctx())
        .await;
    assert_eq!(seen, Err(AppError::NotFound));
    let changed = w
        .secrets
        .set_pools(
            &w.other_admin,
            w.other_namespace.id(),
            "a-only",
            &[],
            &ctx(),
        )
        .await;
    assert_eq!(changed, Err(AppError::NotFound));
    assert_eq!(
        w.secrets
            .pools(&w.admin, ns(&w), "a-only", &ctx())
            .await
            .expect("own pools"),
        names(&["trusted"])
    );
}

#[tokio::test]
async fn a_grant_in_one_namespace_does_not_reach_a_secret_of_another() {
    let w = world().await;
    let other = w.add_namespace("billing").await;
    w.put_secret_in(other.id(), "stripe-key", VALUE).await;
    w.secrets
        .set_pools(
            &w.admin,
            other.id(),
            "stripe-key",
            &names(&["trusted"]),
            &ctx(),
        )
        .await
        .expect("pools");
    // `trusted` is granted in `payments` only.
    grant(&w, "insider", &["trusted"]).await;
    let granted_elsewhere: i64 =
        sqlx::query_scalar("SELECT count(*) FROM worker_binding_pools WHERE namespace_id = $1")
            .bind(other.id().as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(granted_elsewhere, 0);
}

#[tokio::test]
async fn pools_are_managed_by_whoever_manages_secrets_and_bindings_and_validated() {
    let w = world().await;
    w.put_secret("stripe-key", VALUE).await;
    let dora = w.member("dora", Role::Developer).await;
    let denied = w
        .secrets
        .set_pools(&dora, ns(&w), "stripe-key", &names(&["x"]), &ctx())
        .await;
    assert!(
        matches!(denied, Err(AppError::Forbidden { .. })),
        "{denied:?}"
    );
    let missing = w
        .secrets
        .set_pools(&w.admin, ns(&w), "nope", &names(&["x"]), &ctx())
        .await;
    assert_eq!(missing, Err(AppError::NotFound));
    for bad in [
        names(&["has space"]),
        names(&[""]),
        vec!["x".repeat(65)],
        (0..17).map(|i| format!("p{i}")).collect(),
    ] {
        let result = w
            .secrets
            .set_pools(&w.admin, ns(&w), "stripe-key", &bad, &ctx())
            .await;
        assert!(
            matches!(result, Err(AppError::NotAllowed(_))),
            "{bad:?}: {result:?}"
        );
        let result = w
            .bindings
            .bind_with_pools(&w.admin, ns(&w), "x", &names(&["*"]), Some(&bad), &ctx())
            .await;
        assert!(
            matches!(result, Err(AppError::NotAllowed(_))),
            "{bad:?}: {result:?}"
        );
    }

    restrict(&w, "stripe-key", &["b", "a", "b"]).await;
    let listed = w
        .secrets
        .list(&w.admin, ns(&w), &ctx())
        .await
        .expect("list");
    assert_eq!(listed[0].pools, Some(names(&["a", "b"])));
    let audited: Vec<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE action = 'secret.pools_changed'",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(audited, ["pools: a, b"]);

    // Rotating the value keeps the restriction; deleting the secret removes it.
    w.put_secret("stripe-key", "a-rotated-value-1").await;
    assert_eq!(
        w.secrets
            .pools(&w.admin, ns(&w), "stripe-key", &ctx())
            .await
            .expect("pools"),
        names(&["a", "b"])
    );
    w.secrets
        .delete(&w.admin, ns(&w), "stripe-key", &ctx())
        .await
        .expect("delete");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM secret_pools")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(left, 0);
}

#[tokio::test]
async fn pool_grants_follow_the_binding_replacing_it_keeps_them_and_removing_it_removes_them() {
    let w = world().await;
    grant(&w, "insider", &["trusted", "gpu"]).await;
    // A replacement of the activities that names no pools leaves the grants in place.
    let view = w
        .bindings
        .bind(&w.admin, ns(&w), "insider", &names(&["act.*"]), &ctx())
        .await
        .expect("rebind");
    assert_eq!(view.pools, names(&["gpu", "trusted"]));
    let listed = w
        .bindings
        .list(&w.admin, ns(&w), &ctx())
        .await
        .expect("list");
    assert_eq!(listed[0].pools, names(&["gpu", "trusted"]));
    // Naming an empty list clears them.
    grant(&w, "insider", &[]).await;
    let listed = w
        .bindings
        .list(&w.admin, ns(&w), &ctx())
        .await
        .expect("list");
    assert_eq!(listed[0].pools, Vec::<String>::new());
    // Removing the binding removes whatever grants it had.
    grant(&w, "insider", &["trusted"]).await;
    w.bindings
        .unbind(&w.admin, ns(&w), "insider", &ctx())
        .await
        .expect("unbind");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM worker_binding_pools")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(left, 0);
}

// ------------------------------------------------------------- review follow-ups

/// How many runs to start ahead of the unrestricted one: more than the candidate window of a poll
/// for eight tasks (`min(8 * 4, 64)`).
const AHEAD_OF_THE_WINDOW: usize = 40;

#[tokio::test]
async fn refused_steps_ahead_of_the_window_do_not_starve_a_general_worker() {
    let w = world().await;
    w.put_secret("stripe-key", VALUE).await;
    restrict(&w, "stripe-key", &["trusted"]).await;
    let (restricted, _) = w.publish(flow(&["stripe-key"])).await;
    // The older steps are all restricted; the one behind them declares nothing.
    for _ in 0..AHEAD_OF_THE_WINDOW {
        w.start(restricted, &json!({})).await;
    }
    let (free, _) = w.publish(document("free", "a", vec![activity("a")])).await;
    let free_run = w.start(free, &json!({})).await;

    let general = worker(&w, "general").await;
    grant(&w, "general", &[]).await;
    let session = w.session_as(&general, &["act.a"], 100).await;
    let tasks = w.poll(&session, 8).await;
    assert_eq!(
        tasks.iter().map(|t| t.run_id).collect::<Vec<_>>(),
        vec![free_run],
        "the unrestricted step is not hidden behind refused ones"
    );

    // Refused steps are not written down one row per step per poll: a few are sampled.
    for _ in 0..5 {
        poll_len(&w, &session).await;
    }
    let rows = refusals(&w).await;
    assert!(
        !rows.is_empty() && rows.len() <= 4,
        "the audit samples the oldest refused steps: {}",
        rows.len()
    );

    // The entitled worker still gets the restricted work, oldest first.
    let insider = worker(&w, "insider").await;
    grant(&w, "insider", &["trusted"]).await;
    let inside = w.session_as(&insider, &["act.a"], 100).await;
    assert_eq!(poll_len(&w, &inside).await, 8);
}

#[tokio::test]
async fn a_step_created_before_its_secrets_were_recorded_is_still_refused_at_the_lease() {
    let w = world().await;
    let run = run_declaring(&w).await;
    restrict(&w, "stripe-key", &["trusted"]).await;
    // What a step that predates the `secrets` column looks like.
    sqlx::query("UPDATE steps SET secrets = NULL WHERE run_id = $1")
        .bind(run.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("legacy step");
    let outsider = worker(&w, "outsider").await;
    grant(&w, "outsider", &[]).await;
    let session = w.session_as(&outsider, &["act.a"], 10).await;
    assert_eq!(poll_len(&w, &session).await, 0);
    assert_eq!(step_state(&w, run).await, ("READY".to_owned(), 0, false));
    assert_eq!(refusals(&w).await.len(), 1);
}

#[tokio::test]
async fn a_secret_created_with_pools_is_never_unrestricted_and_a_rotation_keeps_them() {
    let w = world().await;
    let made = w
        .secrets
        .put_with_pools(
            &w.admin,
            ns(&w),
            "stripe-key",
            VALUE.as_bytes().to_vec(),
            Some(&names(&["trusted"])),
            &ctx(),
        )
        .await
        .expect("create restricted");
    assert_eq!(made.pools, Some(names(&["trusted"])));
    // The row of the version and the restriction exist together.
    let (secrets, pools): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM secrets), (SELECT count(*) FROM secret_pools)",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!((secrets, pools), (1, 1));

    // A rotation that names no pools keeps them; naming `[]` lifts them.
    let rotated = w
        .secrets
        .put(
            &w.admin,
            ns(&w),
            "stripe-key",
            b"a-rotated-value-2".to_vec(),
            &ctx(),
        )
        .await
        .expect("rotate");
    assert_eq!(rotated.pools, Some(names(&["trusted"])));
    let lifted = w
        .secrets
        .put_with_pools(
            &w.admin,
            ns(&w),
            "stripe-key",
            b"a-rotated-value-3".to_vec(),
            Some(&[]),
            &ctx(),
        )
        .await
        .expect("lift");
    assert_eq!(lifted.pools, Some(Vec::new()));

    // Deleting a secret deletes its restriction (documented): a recreation names the pools again.
    restrict(&w, "stripe-key", &["trusted"]).await;
    w.secrets
        .delete(&w.admin, ns(&w), "stripe-key", &ctx())
        .await
        .expect("delete");
    let again = w
        .secrets
        .put(
            &w.admin,
            ns(&w),
            "stripe-key",
            b"a-recreated-value".to_vec(),
            &ctx(),
        )
        .await
        .expect("recreate");
    assert_eq!(again.pools, Some(Vec::new()));

    // Nothing is stored when a pool name is refused, and the creation is audited with its pools.
    let refused = w
        .secrets
        .put_with_pools(
            &w.admin,
            ns(&w),
            "second-key",
            b"a-second-value-1".to_vec(),
            Some(&names(&["has space"])),
            &ctx(),
        )
        .await;
    assert!(
        matches!(refused, Err(AppError::NotAllowed(_))),
        "{refused:?}"
    );
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM secrets WHERE name = 'second-key')")
            .fetch_one(&w.db.pool)
            .await
            .expect("exists");
    assert!(!exists);
    let reasons: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE action = 'secret.stored' ORDER BY created_at",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(reasons[0].as_deref(), Some("pools: trusted"));
    assert_eq!(
        reasons[1], None,
        "a rotation that names no pools says nothing about them"
    );
}

#[tokio::test]
async fn only_those_who_manage_secrets_are_told_which_pools_may_receive_one() {
    let w = world().await;
    w.put_secret("stripe-key", VALUE).await;
    restrict(&w, "stripe-key", &["trusted"]).await;
    let dora = w.member("dora", Role::Developer).await;
    w.secrets
        .grant(&w.admin, ns(&w), "stripe-key", "dora", &ctx())
        .await
        .expect("grant");
    let seen = w.secrets.list(&dora, ns(&w), &ctx()).await.expect("list");
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].pools, None,
        "not disclosed, which is not the same as empty"
    );
    let admin = w
        .secrets
        .list(&w.admin, ns(&w), &ctx())
        .await
        .expect("list");
    assert_eq!(admin[0].pools, Some(names(&["trusted"])));
}

/// A resolution that starts while a change to who may receive the secret is in flight waits for it
/// and judges the result; it cannot pass the old check and read the value after the change committed.
async fn resolution_waits_for_the_change(change: &str) {
    let w = Arc::new(world().await);
    run_declaring(&w).await;
    restrict(&w, "stripe-key", &["trusted"]).await;
    let insider = worker(&w, "insider").await;
    grant(&w, "insider", &["trusted"]).await;
    let session = w.session_as(&insider, &["act.a"], 10).await;
    let task = w.one_task(&session).await;

    // The change is made in a transaction that has not committed yet.
    let mut tx = w.db.pool.begin().await.expect("begin");
    sqlx::query("SELECT id FROM secrets WHERE name = 'stripe-key' FOR UPDATE")
        .execute(&mut *tx)
        .await
        .expect("lock the secret like set_pools does");
    sqlx::raw_sql(sqlx::AssertSqlSafe(change.to_owned()))
        .execute(&mut *tx)
        .await
        .expect("change");

    let resolver = {
        let w = Arc::clone(&w);
        tokio::spawn(async move {
            w.workers
                .resolve_secrets(&insider, &task_ref(&task), None)
                .await
                .map(|secrets| secrets.len())
        })
    };
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !resolver.is_finished(),
        "the resolution went ahead of a change that was about to commit"
    );
    tx.commit().await.expect("commit");
    let result = resolver.await.expect("task");
    assert!(
        matches!(result, Err(AppError::SecretUnavailable { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_resolution_cannot_read_the_value_past_a_restriction_that_is_committing() {
    resolution_waits_for_the_change(
        "DELETE FROM secret_pools WHERE pool = 'trusted'; \
         INSERT INTO secret_pools (tenant_id, secret_id, pool, granted_by) \
         SELECT tenant_id, id, 'elsewhere', 'test' FROM secrets WHERE name = 'stripe-key'",
    )
    .await;
}

#[tokio::test]
async fn a_resolution_cannot_read_the_value_past_a_grant_that_is_being_taken_away() {
    resolution_waits_for_the_change(
        "DELETE FROM worker_binding_pools WHERE subject = 'insider' AND pool = 'trusted'",
    )
    .await;
}
