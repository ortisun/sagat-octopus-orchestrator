//! The secret store (FR-009-002, FR-009-003, FR-009-004, US02): a value goes in once, is stored
//! sealed, and comes out only to a worker that holds the lease of a step whose definition declares
//! it, published by someone who may use it, with every step audited and no value anywhere else.

use std::sync::Arc;

use sagat_application::error::AppError;
use sagat_application::quotas::QuotaStore;
use sagat_application::secrets::SecretService;
use sagat_domain::execution::{RunStatus, StepStatus};
use sagat_domain::identity::{NamespaceId, Principal, Role};
use sagat_domain::quota::QuotaResource;
use serde_json::{Value, json};
use sqlx::Row;

use crate::support::engine::{World, activity, ctx, document, task_ref, with, world};

/// A value no ordinary text contains, built from characters `strpos` treats literally.
const CANARY: &str = "sk-live-4eC39HqLyjWDarjtT1zdp7dc";

fn flow(name: &str, secrets: &[&str]) -> Value {
    document(
        name,
        "a",
        vec![with(activity("a"), "secrets", json!(secrets))],
    )
}

fn ns(w: &World) -> NamespaceId {
    w.namespace.id()
}

/// Every table whose rows mention `needle` anywhere in their text form, with how many rows do.
async fn tables_mentioning(w: &World, needle: &str) -> Vec<(String, i64)> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename FROM pg_tables WHERE schemaname = 'public' ORDER BY 1",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("tables");
    let mut found = Vec::new();
    for table in tables {
        // Table names come from the catalogue, not from a caller.
        let sql = format!("SELECT count(*) FROM \"{table}\" x WHERE strpos(x::text, $1) > 0");
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(needle)
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
        if count > 0 {
            found.push((table, count));
        }
    }
    found
}

/// A developer who may use `stripe-key`, a published workflow that declares it, a run and a leased
/// step: everything a worker needs to ask for the secret.
struct Leased {
    developer: Principal,
    session: crate::support::engine::Session,
    task: sagat_application::worker::ClaimedTask,
    run: sagat_domain::execution::RunId,
}

async fn leased_step(w: &World, secrets: &[&str]) -> Leased {
    let developer = w.member("dora", Role::Developer).await;
    for name in secrets {
        w.secrets
            .grant(&w.admin, ns(w), name, "dora", &ctx())
            .await
            .expect("grant");
    }
    let (wf, _) = w
        .try_publish_as(&developer, flow("pay", secrets))
        .await
        .expect("the developer may use the secrets");
    let run = w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    Leased {
        developer,
        session,
        task,
        run,
    }
}

/// How many secrets the leased step is handed right now.
async fn resolved_count(w: &World, l: &Leased) -> Result<usize, AppError> {
    w.workers
        .resolve_secrets(&l.session.principal, &task_ref(&l.task), None)
        .await
        .map(|secrets| secrets.len())
}

// ------------------------------------------------------------- storing

#[tokio::test]
async fn a_secret_is_stored_sealed_and_shown_nowhere() {
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;

    let listed = w
        .secrets
        .list(&w.admin, ns(&w), &ctx())
        .await
        .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (
            listed[0].name.as_str(),
            listed[0].version,
            listed[0].key_id.as_str()
        ),
        ("stripe-key", 1, "k1")
    );
    assert!(!format!("{listed:?}").contains(CANARY));

    // No row of any table, the ciphertext included, contains the value or any readable form of it.
    assert_eq!(tables_mentioning(&w, CANARY).await, Vec::new());
    let (ciphertext_len, nonce_len): (i32, i32) =
        sqlx::query_as("SELECT octet_length(ciphertext), octet_length(nonce) FROM secret_versions")
            .fetch_one(&w.db.pool)
            .await
            .expect("row");
    assert_eq!(
        (ciphertext_len, nonce_len),
        (i32::try_from(CANARY.len()).expect("small") + 16, 12),
        "the value plus the authentication tag, under a fresh nonce"
    );

    // The audit trail names the secret and says who, never what.
    let (action, resource, actor): (String, String, String) = sqlx::query_as(
        "SELECT action, resource_id, actor FROM audit_records WHERE action = 'secret.stored'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        (action.as_str(), resource.as_str(), actor.as_str()),
        (
            "secret.stored",
            format!("{}/stripe-key", ns(&w)).as_str(),
            "alice"
        )
    );
}

#[tokio::test]
async fn rotating_a_secret_adds_a_version_keeps_a_few_and_the_next_resolution_sees_the_newest() {
    let w = world().await;
    let l = {
        w.put_secret("stripe-key", "first-value-0001").await;
        leased_step(&w, &["stripe-key"]).await
    };
    for n in 2..=7 {
        w.put_secret("stripe-key", &format!("rotated-value-{n:04}"))
            .await;
    }
    let (current, kept): (i32, i64) = sqlx::query_as(
        "SELECT s.current_version, count(v.id) FROM secrets s \
         JOIN secret_versions v ON v.secret_id = s.id GROUP BY s.current_version",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("versions");
    assert_eq!((current, kept), (7, 5), "the newest five are kept");

    let resolved = w
        .workers
        .resolve_secrets(&l.session.principal, &task_ref(&l.task), None)
        .await
        .expect("resolve");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].version, 7);
    assert_eq!(resolved[0].value.expose(), b"rotated-value-0007");
}

#[tokio::test]
async fn a_value_and_a_name_must_be_usable() {
    let w = world().await;
    for (what, name, value) in [
        ("an empty value", "ok-name", Vec::new()),
        ("a huge value", "ok-name", vec![b'x'; 16 * 1024 + 1]),
        ("an upper-case name", "Stripe", b"value".to_vec()),
        ("a name starting with a digit", "1key", b"value".to_vec()),
        ("a name with a space", "a key", b"value".to_vec()),
        ("an empty name", "", b"value".to_vec()),
    ] {
        let result = w.secrets.put(&w.admin, ns(&w), name, value, &ctx()).await;
        assert!(
            matches!(result, Err(AppError::NotAllowed(_))),
            "{what}: {result:?}"
        );
    }
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM secrets")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn a_namespace_holds_a_bounded_number_of_secrets_but_rotation_and_deletion_are_free() {
    let w = world().await;
    w.quotas
        .set(&w.admin, Some(ns(&w)), QuotaResource::Secrets, 2, &ctx())
        .await
        .expect("quota");
    w.put_secret("one-key", "value-one-0001").await;
    w.put_secret("two-key", "value-two-0001").await;
    let third = w
        .secrets
        .put(
            &w.admin,
            ns(&w),
            "three-key",
            b"value-three".to_vec(),
            &ctx(),
        )
        .await;
    assert_eq!(
        third.map(|_| ()),
        Err(AppError::QuotaExceeded("secrets in the namespace"))
    );
    // A new version of an existing secret is not another secret.
    w.put_secret("one-key", "value-one-0002").await;
    // Deleting frees the place.
    w.secrets
        .delete(&w.admin, ns(&w), "two-key", &ctx())
        .await
        .expect("delete");
    w.put_secret("three-key", "value-three-01").await;
    let used = w
        .store
        .usage(w.admin.tenant_id(), Some(ns(&w)), QuotaResource::Secrets)
        .await
        .expect("usage");
    assert_eq!(used, 2);
}

#[tokio::test]
async fn racing_creators_of_one_name_make_one_secret_with_every_version_kept_in_order() {
    let w = Arc::new(world().await);
    let mut attempts = Vec::new();
    for i in 0..8 {
        let w = Arc::clone(&w);
        attempts.push(tokio::spawn(async move {
            w.secrets
                .put(
                    &w.admin,
                    ns(&w),
                    "shared-key",
                    format!("value-from-{i:02}-xxxx").into_bytes(),
                    &ctx(),
                )
                .await
        }));
    }
    let mut versions = Vec::new();
    for attempt in attempts {
        versions.push(
            attempt
                .await
                .expect("task")
                .expect("every put succeeds")
                .version,
        );
    }
    versions.sort_unstable();
    assert_eq!(
        versions,
        (1..=8).collect::<Vec<_>>(),
        "one secret, eight versions"
    );
    let secrets: i64 = sqlx::query_scalar("SELECT count(*) FROM secrets")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(secrets, 1);
}

// ------------------------------------------------------------- who may use a secret

#[tokio::test]
async fn publishing_needs_the_secret_to_exist_and_its_access_list_to_name_the_publisher() {
    let w = world().await;
    let dora = w.member("dora", Role::Developer).await;

    // Nothing exists yet: an administrator is refused as well.
    let missing = w
        .try_publish_as(&w.admin, flow("a-flow", &["stripe-key"]))
        .await;
    assert_eq!(
        missing.map(|_| ()),
        Err(AppError::SecretUnavailable {
            name: "stripe-key".into()
        })
    );

    // It exists; the developer is not on its list (somebody else is). The answer is the same as for
    // a missing one.
    w.put_secret("stripe-key", CANARY).await;
    w.secrets
        .grant(&w.admin, ns(&w), "stripe-key", "erin", &ctx())
        .await
        .expect("someone else is on the list");
    let denied = w
        .try_publish_as(&dora, flow("b-flow", &["stripe-key"]))
        .await;
    assert_eq!(
        denied.map(|_| ()),
        Err(AppError::SecretUnavailable {
            name: "stripe-key".into()
        })
    );
    // The administrator is not subject to the list; once on the list, the developer may.
    w.try_publish_as(&w.admin, flow("c-flow", &["stripe-key"]))
        .await
        .expect("administrator");
    w.secrets
        .grant(&w.admin, ns(&w), "stripe-key", "dora", &ctx())
        .await
        .expect("grant");
    w.try_publish_as(&dora, flow("d-flow", &["stripe-key"]))
        .await
        .expect("developer on the list");

    // The refusals are in the audit trail, once per publisher and secret (a repeat collapses).
    crate::support::eventually("the refusal is audited", || async {
        let rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records WHERE action = 'secret.denied' AND actor = 'dora'",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
        (rows == 1).then_some(())
    })
    .await;
}

#[tokio::test]
async fn a_worker_receives_only_the_declared_secrets_for_the_length_of_its_lease() {
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;
    w.put_secret("other-key", "not-for-this-step-0001").await;
    let l = leased_step(&w, &["stripe-key"]).await;

    let resolved = w
        .workers
        .resolve_secrets(&l.session.principal, &task_ref(&l.task), None)
        .await
        .expect("resolve");
    let names: Vec<_> = resolved.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["stripe-key"], "only what the step declares");
    assert_eq!(resolved[0].value.expose(), CANARY.as_bytes());
    assert!(
        !format!("{resolved:?}").contains(CANARY),
        "a resolved secret never prints"
    );

    // The lease is the capability: a different token is a stale attempt, and a finished step has none.
    let mut forged = task_ref(&l.task);
    forged.lease_token = "not-the-lease-token";
    assert_eq!(
        w.workers
            .resolve_secrets(&l.session.principal, &forged, None)
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt)
    );
    w.complete(&l.session, &l.task, &json!({"ok": true}))
        .await
        .expect("complete");
    assert_eq!(
        w.workers
            .resolve_secrets(&l.session.principal, &task_ref(&l.task), None)
            .await
            .map(|_| ()),
        Err(AppError::StaleAttempt),
        "once the attempt is over the capability is gone"
    );
    assert_eq!(w.step_status(l.run, "a").await, Some(StepStatus::Succeeded));
}

#[tokio::test]
async fn a_step_that_declares_nothing_resolves_nothing() {
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;
    let (wf, _) = w.publish(document("plain", "a", vec![activity("a")])).await;
    w.start(wf, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    let resolved = w
        .workers
        .resolve_secrets(&session.principal, &task_ref(&task), None)
        .await
        .expect("resolve");
    assert!(resolved.is_empty());
    let audited: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = 'secret.resolved'")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(audited, 0, "nothing was handed out, nothing to record");
}

#[tokio::test]
async fn every_resolution_is_audited_with_names_and_versions_and_never_values() {
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;
    let l = leased_step(&w, &["stripe-key"]).await;
    w.workers
        .resolve_secrets(
            &l.session.principal,
            &task_ref(&l.task),
            Some("0af7651916cd43dd8448eb211c80319c"),
        )
        .await
        .expect("resolve");

    let (actor, reason, trace): (String, String, Option<String>) = sqlx::query_as(
        "SELECT actor, reason, trace_id FROM audit_records WHERE action = 'secret.resolved'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(actor, "worker-1");
    assert_eq!(reason, "node a attempt 1: stripe-key v1");
    assert_eq!(trace.as_deref(), Some("0af7651916cd43dd8448eb211c80319c"));
    assert_eq!(tables_mentioning(&w, CANARY).await, Vec::new());
}

#[tokio::test]
async fn losing_access_after_publishing_stops_the_next_resolution_and_regaining_it_restores_it() {
    // "ACL race when renewing a token": the check is made when the value is asked for, not only
    // when the workflow was published.
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;
    let l = leased_step(&w, &["stripe-key"]).await;
    assert_eq!(resolved_count(&w, &l).await, Ok(1));

    // Somebody else stays on the list: what matters is the publisher's own entry.
    w.secrets
        .grant(&w.admin, ns(&w), "stripe-key", "erin", &ctx())
        .await
        .expect("grant a bystander");
    w.secrets
        .revoke(&w.admin, ns(&w), "stripe-key", "dora", &ctx())
        .await
        .expect("revoke");
    assert_eq!(
        resolved_count(&w, &l).await,
        Err(AppError::SecretUnavailable {
            name: "stripe-key".into()
        })
    );
    let denied: String = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE action = 'secret.denied' AND actor = 'worker-1'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("the refusal is recorded");
    assert!(denied.contains("stripe-key") && denied.contains("not usable by its publisher"));

    w.secrets
        .grant(&w.admin, ns(&w), "stripe-key", "dora", &ctx())
        .await
        .expect("grant again");
    assert_eq!(resolved_count(&w, &l).await, Ok(1));

    // The publisher leaving the tenant, or being demoted to a role that cannot use secrets, stops it too.
    sqlx::query("UPDATE memberships SET role = 'viewer' WHERE subject = 'dora'")
        .execute(&w.db.pool)
        .await
        .expect("demote");
    assert!(matches!(
        resolved_count(&w, &l).await,
        Err(AppError::SecretUnavailable { .. })
    ));
    sqlx::query("DELETE FROM memberships WHERE subject = 'dora'")
        .execute(&w.db.pool)
        .await
        .expect("remove");
    assert!(matches!(
        resolved_count(&w, &l).await,
        Err(AppError::SecretUnavailable { .. })
    ));
    let _ = l.developer;
}

#[tokio::test]
async fn deleting_a_secret_stops_resolution_and_removes_its_versions_and_list() {
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;
    let l = leased_step(&w, &["stripe-key"]).await;
    w.secrets
        .delete(&w.admin, ns(&w), "stripe-key", &ctx())
        .await
        .expect("delete");
    assert!(matches!(
        w.workers
            .resolve_secrets(&l.session.principal, &task_ref(&l.task), None)
            .await,
        Err(AppError::SecretUnavailable { .. })
    ));
    for table in ["secrets", "secret_versions", "secret_access"] {
        let sql = format!("SELECT count(*) FROM {table}");
        let rows: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
        assert_eq!(rows, 0, "{table}");
    }
    let again = w
        .secrets
        .delete(&w.admin, ns(&w), "stripe-key", &ctx())
        .await;
    assert_eq!(again, Err(AppError::NotFound));
}

#[tokio::test]
async fn an_unbound_worker_is_refused_before_any_secret_is_read() {
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;
    let l = leased_step(&w, &["stripe-key"]).await;
    sqlx::query("DELETE FROM worker_bindings WHERE subject = 'worker-1'")
        .execute(&w.db.pool)
        .await
        .expect("unbind");
    let refused = w
        .workers
        .resolve_secrets(&l.session.principal, &task_ref(&l.task), None)
        .await;
    assert_eq!(refused.map(|_| ()), Err(AppError::NotBound));
    let resolved: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = 'secret.resolved'")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(resolved, 0);
}

// ------------------------------------------------------------- what the step may send back

#[tokio::test]
async fn a_secret_in_a_steps_output_fails_the_step_and_is_stored_nowhere() {
    for (what, output) in [
        ("the raw value", json!({"echo": CANARY})),
        (
            "inside other text",
            json!({"message": format!("calling with {CANARY} now")}),
        ),
        (
            "in a nested value",
            json!({"a": {"b": [1, {"token": CANARY}]}}),
        ),
    ] {
        let w = world().await;
        w.put_secret("stripe-key", CANARY).await;
        let l = leased_step(&w, &["stripe-key"]).await;
        let ack = w
            .complete(&l.session, &l.task, &output)
            .await
            .expect("the report is accepted as a failure");
        assert_eq!(ack.step_status, StepStatus::Failed, "{what}");
        let code: Option<String> =
            sqlx::query_scalar("SELECT failure_code FROM steps WHERE run_id = $1 AND node = 'a'")
                .bind(l.run.as_uuid())
                .fetch_one(&w.db.pool)
                .await
                .expect("code");
        assert_eq!(code.as_deref(), Some("SECRET_IN_OUTPUT"), "{what}");
        assert_eq!(w.run_status(l.run).await, RunStatus::Failed, "{what}");
        assert_eq!(
            tables_mentioning(&w, CANARY).await,
            Vec::new(),
            "{what}: the value reached no table"
        );
        let denied: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records WHERE action = 'secret.denied' AND actor = 'worker-1'",
        )
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
        assert_eq!(denied, 1, "{what}");
    }
}

#[tokio::test]
async fn the_escaped_forms_of_a_secret_are_recognised_too() {
    let w = world().await;
    let value = "pa\"ss\\word\nwith-\u{e9}-and-more";
    w.put_secret("db-password", value).await;
    // Raw JSON text, so the worker's serializer decides how the value is escaped. The ASCII-only
    // form is built with a backslash from a variable: a literal escape in this source would be
    // turned into the character itself before the compiler sees it.
    let backslash = '\\';
    for (what, raw) in [
        (
            "minimal escaping",
            r#"{"x":"pa\"ss\\word\nwith-é-and-more"}"#.to_owned(),
        ),
        (
            "ASCII-only escaping",
            format!(r#"{{"x":"pa\"ss\\word\nwith-{backslash}u00e9-and-more"}}"#),
        ),
    ] {
        let l = {
            let (wf, _) = w
                .publish(flow(&format!("f-{}", what.len()), &["db-password"]))
                .await;
            w.start(wf, &json!({})).await;
            let session = w.session(&["act.a"]).await;
            let task = w.one_task(&session).await;
            (session, task)
        };
        let ack = w
            .workers
            .complete(
                &l.0.principal,
                &task_ref(&l.1),
                raw.as_bytes(),
                &format!("done-{}", l.1.task_id()),
            )
            .await
            .expect("accepted");
        assert_eq!(ack.step_status, StepStatus::Failed, "{what}");
    }
}

#[tokio::test]
async fn an_output_that_does_not_contain_the_secret_is_stored_as_usual() {
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;
    let l = leased_step(&w, &["stripe-key"]).await;
    let ack = w
        .complete(
            &l.session,
            &l.task,
            &json!({"charged": true, "id": "ch_123"}),
        )
        .await
        .expect("complete");
    assert_eq!(ack.step_status, StepStatus::Succeeded);
}

#[tokio::test]
async fn very_short_secrets_are_not_searched_for_because_they_would_match_ordinary_text() {
    // A documented limit of the output scan, pinned so it cannot change unnoticed.
    let w = world().await;
    w.put_secret("pin", "1234").await;
    let l = leased_step(&w, &["pin"]).await;
    let ack = w
        .complete(&l.session, &l.task, &json!({"order": "id 1234"}))
        .await
        .expect("complete");
    assert_eq!(ack.step_status, StepStatus::Succeeded);
}

// ------------------------------------------------------------- binding the ciphertext to its place

#[tokio::test]
async fn a_ciphertext_moved_to_another_secret_does_not_open() {
    let w = world().await;
    w.put_secret("alpha-key", "alpha-value-0001").await;
    w.put_secret("beta-key", "beta-value-00002").await;
    let l = leased_step(&w, &["beta-key"]).await;

    // Someone with write access to the table copies alpha's sealed value over beta's.
    let alpha = sqlx::query(
        "SELECT v.key_id, v.nonce, v.ciphertext FROM secret_versions v \
         JOIN secrets s ON s.id = v.secret_id WHERE s.name = 'alpha-key'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("alpha");
    sqlx::query(
        "UPDATE secret_versions v SET key_id = $1, nonce = $2, ciphertext = $3 \
         FROM secrets s WHERE s.id = v.secret_id AND s.name = 'beta-key'",
    )
    .bind(alpha.get::<String, _>("key_id"))
    .bind(alpha.get::<Vec<u8>, _>("nonce"))
    .bind(alpha.get::<Vec<u8>, _>("ciphertext"))
    .execute(&w.db.pool)
    .await
    .expect("swap");

    let result = w
        .workers
        .resolve_secrets(&l.session.principal, &task_ref(&l.task), None)
        .await;
    assert_eq!(
        result.map(|_| ()),
        Err(AppError::Internal),
        "bound to its own tenant, namespace, name and row, a moved ciphertext is just noise"
    );
}

// ------------------------------------------------------------- keys

#[tokio::test]
async fn re_sealing_moves_every_version_to_the_active_key_so_the_old_one_can_be_retired() {
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;
    w.put_secret("stripe-key", "second-version-0001").await;
    w.put_secret("db-password", "database-password-1").await;
    let l = leased_step(&w, &["stripe-key", "db-password"]).await;

    // A new key is added in front; the old one stays so everything can still be opened.
    let k2 = sagat_infra_secrets::generate_key().expect("key");
    let rotated = sagat_infra_secrets::SecretsConfig::parse(&format!("k2:{k2},k1:{}", w.key_k1))
        .expect("ring");
    let admin = SecretService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        rotated.cipher(),
    );
    let moved = admin.reseal(100).await.expect("reseal");
    assert_eq!(moved, 3, "every version, not only the current ones");
    let still: i64 =
        sqlx::query_scalar("SELECT count(*) FROM secret_versions WHERE key_id <> 'k2'")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(still, 0);
    assert_eq!(admin.reseal(100).await.expect("again"), 0, "idempotent");

    // The old key can now be retired: a ring with k2 alone resolves everything.
    let retired = sagat_infra_secrets::SecretsConfig::parse(&format!("k2:{k2}")).expect("ring");
    let workers = sagat_application::worker::WorkerService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        sagat_application::worker::WorkerSettings::default(),
    )
    .with_vault(retired.cipher());
    let resolved = workers
        .resolve_secrets(&l.session.principal, &task_ref(&l.task), None)
        .await
        .expect("resolve under the new ring alone");
    let values: Vec<_> = resolved.iter().map(|s| s.value.expose().to_vec()).collect();
    assert_eq!(
        values,
        vec![
            b"second-version-0001".to_vec(),
            b"database-password-1".to_vec()
        ]
    );

    // And a ring without the key a version is sealed under cannot open it: a loud failure.
    let wrong = sagat_infra_secrets::SecretsConfig::parse(&format!(
        "k9:{}",
        sagat_infra_secrets::generate_key().expect("key")
    ))
    .expect("ring");
    let workers = sagat_application::worker::WorkerService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        sagat_application::worker::WorkerSettings::default(),
    )
    .with_vault(wrong.cipher());
    assert_eq!(
        workers
            .resolve_secrets(&l.session.principal, &task_ref(&l.task), None)
            .await
            .map(|_| ()),
        Err(AppError::Internal)
    );
}

#[tokio::test]
async fn without_a_key_ring_no_secret_is_ever_resolved() {
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;
    let l = leased_step(&w, &["stripe-key"]).await;
    let replica_without_vault = sagat_application::worker::WorkerService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::execution::Engine::new(sagat_infra_ir::JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        sagat_application::worker::WorkerSettings::default(),
    );
    let refused = replica_without_vault
        .resolve_secrets(&l.session.principal, &task_ref(&l.task), None)
        .await;
    assert!(
        matches!(refused, Err(AppError::NotAllowed(_))),
        "{refused:?}"
    );
}

// ------------------------------------------------------------- the tenant boundary and roles

#[tokio::test]
async fn the_secrets_of_one_tenant_are_invisible_and_unchangeable_by_another() {
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;
    let theirs = w.other_namespace.id();

    // The other tenant reaches for this tenant's namespace: every operation is a plain not-found.
    let foreign = &w.other_admin;
    let here = ns(&w);
    assert_eq!(
        w.secrets.list(foreign, here, &ctx()).await.map(|_| ()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.secrets
            .put(
                foreign,
                here,
                "stripe-key",
                b"overwritten-value".to_vec(),
                &ctx()
            )
            .await
            .map(|_| ()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.secrets.delete(foreign, here, "stripe-key", &ctx()).await,
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.secrets
            .grant(foreign, here, "stripe-key", "bob", &ctx())
            .await,
        Err(AppError::NotFound)
    );

    // The same name in its own namespace is another secret entirely.
    w.secrets
        .put(
            foreign,
            theirs,
            "stripe-key",
            b"their-own-secret".to_vec(),
            &ctx(),
        )
        .await
        .expect("their own");
    let mine: Vec<u8> = {
        let l = leased_step(&w, &["stripe-key"]).await;
        w.workers
            .resolve_secrets(&l.session.principal, &task_ref(&l.task), None)
            .await
            .expect("resolve")[0]
            .value
            .expose()
            .to_vec()
    };
    assert_eq!(
        mine,
        CANARY.as_bytes(),
        "untouched by the other tenant's secret of the same name"
    );
}

#[tokio::test]
async fn managing_secrets_is_for_administrators_and_reading_their_names_for_developers() {
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;
    let tenant = w.admin.tenant_id();
    for role in [Role::Developer, Role::Operator, Role::Viewer, Role::Worker] {
        let who = w.member(&format!("as-{role}"), role).await;
        let _ = tenant;
        let put = w
            .secrets
            .put(&who, ns(&w), "x-key", b"value-value".to_vec(), &ctx())
            .await;
        let delete = w.secrets.delete(&who, ns(&w), "stripe-key", &ctx()).await;
        let grant = w
            .secrets
            .grant(&who, ns(&w), "stripe-key", "someone", &ctx())
            .await;
        let access = w.secrets.access(&who, ns(&w), "stripe-key", &ctx()).await;
        assert!(matches!(put, Err(AppError::Forbidden { .. })), "{role}");
        assert!(matches!(delete, Err(AppError::Forbidden { .. })), "{role}");
        assert!(matches!(grant, Err(AppError::Forbidden { .. })), "{role}");
        assert!(matches!(access, Err(AppError::Forbidden { .. })), "{role}");
        let list = w.secrets.list(&who, ns(&w), &ctx()).await;
        if role == Role::Developer {
            assert_eq!(list.expect("a developer sees the names").len(), 1);
        } else {
            assert!(matches!(list, Err(AppError::Forbidden { .. })), "{role}");
        }
    }
}

#[tokio::test]
async fn granting_and_revoking_are_audited_and_listed() {
    let w = world().await;
    w.put_secret("stripe-key", CANARY).await;
    w.secrets
        .grant(&w.admin, ns(&w), "stripe-key", "dora", &ctx())
        .await
        .expect("grant");
    w.secrets
        .grant(&w.admin, ns(&w), "stripe-key", "dora", &ctx())
        .await
        .expect("granting twice is harmless");
    assert_eq!(
        w.secrets
            .access(&w.admin, ns(&w), "stripe-key", &ctx())
            .await
            .expect("list"),
        vec!["dora".to_owned()]
    );
    assert_eq!(
        w.secrets
            .grant(&w.admin, ns(&w), "missing-key", "dora", &ctx())
            .await,
        Err(AppError::NotFound)
    );
    w.secrets
        .revoke(&w.admin, ns(&w), "stripe-key", "dora", &ctx())
        .await
        .expect("revoke");
    assert_eq!(
        w.secrets
            .revoke(&w.admin, ns(&w), "stripe-key", "dora", &ctx())
            .await,
        Err(AppError::NotFound),
        "there was nothing left to revoke"
    );
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT action, reason FROM audit_records \
         WHERE action IN ('secret.access_granted', 'secret.access_revoked') ORDER BY created_at",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        rows,
        vec![
            (
                "secret.access_granted".to_owned(),
                "subject dora".to_owned()
            ),
            (
                "secret.access_granted".to_owned(),
                "subject dora".to_owned()
            ),
            (
                "secret.access_revoked".to_owned(),
                "subject dora".to_owned()
            ),
        ]
    );
}

#[tokio::test]
async fn each_secret_operation_audits_a_probe_of_a_foreign_namespace_on_its_own() {
    // Identical probes are collapsed into one audit row, and the foreign key would refuse a write
    // into another tenant's namespace anyway, so each operation gets a world of its own to prove
    // that it records the probe itself rather than relying on another one or on the database.
    for operation in ["list", "put", "delete", "grant", "revoke", "access"] {
        let w = world().await;
        let here = ns(&w);
        let foreign = &w.other_admin;
        let result = match operation {
            "list" => w.secrets.list(foreign, here, &ctx()).await.map(|_| ()),
            "put" => w
                .secrets
                .put(
                    foreign,
                    here,
                    "stripe-key",
                    b"some-secret-value".to_vec(),
                    &ctx(),
                )
                .await
                .map(|_| ()),
            "delete" => w.secrets.delete(foreign, here, "stripe-key", &ctx()).await,
            "grant" => {
                w.secrets
                    .grant(foreign, here, "stripe-key", "bob", &ctx())
                    .await
            }
            "revoke" => {
                w.secrets
                    .revoke(foreign, here, "stripe-key", "bob", &ctx())
                    .await
            }
            _ => w
                .secrets
                .access(foreign, here, "stripe-key", &ctx())
                .await
                .map(|_| ()),
        };
        assert_eq!(result, Err(AppError::NotFound), "{operation}");
        let tenant = foreign.tenant_id();
        let audited = crate::support::eventually(operation, || async {
            let rows: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM audit_records WHERE tenant_id = $1 \
                 AND action = 'access.denied.cross_tenant' AND resource_type = 'namespace'",
            )
            .bind(tenant.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
            (rows > 0).then_some(rows)
        })
        .await;
        assert_eq!(audited, 1, "{operation}");
    }
}
