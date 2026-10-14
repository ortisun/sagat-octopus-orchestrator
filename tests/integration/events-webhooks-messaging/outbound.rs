//! US04: a finished run is delivered, signed, to the subscriptions that asked, retried with backoff
//! when the receiver is down, and set aside where it can be seen and redriven when it never answers
//! (FR-004-006).

use std::time::Duration;

use sagat_application::error::{AppError, ConflictKind};
use sagat_application::outbound::{
    DeliveryRecord, OutboundStore, SubscriptionRecord, SubscriptionSettings,
};
use sagat_domain::identity::Role;
use sagat_domain::outbound::{
    AttemptOutcome, DeliveryLimits, DeliveryStatus, OutboundDestination, OutboundKind,
    SubscriptionId, SubscriptionStatus, TargetUrl,
};
use sagat_domain::quota::QuotaResource;
use sagat_domain::secrets::SecretName;
use serde_json::{Value, json};

use crate::support::engine::{World, activity, ctx, document, world};
use crate::support::receiver::{Receiver, Reply};
use crate::support::{now, slug};
use crate::{SECRET, SECRET_NAME};

/// A world with the signing secret stored.
async fn with_secret() -> World {
    let w = world().await;
    w.put_secret(SECRET_NAME, SECRET).await;
    w
}

fn settings_for(url: &str) -> SubscriptionSettings {
    SubscriptionSettings {
        url: TargetUrl::parse(url).expect("url"),
        secret_name: SecretName::parse(SECRET_NAME).expect("secret"),
        kinds: vec![OutboundKind::RunFinished],
        limits: DeliveryLimits::new(100, Duration::from_secs(2)).expect("limits"),
        status: SubscriptionStatus::Active,
    }
}

/// A subscription to `receiver`, stored directly: a receiver on loopback is not something an
/// egress policy can allow, so the check the service makes on the destination would refuse it.
async fn subscribe(w: &World, name: &str, receiver: &Receiver) -> SubscriptionRecord {
    subscribe_with(w, name, receiver, settings_for(&receiver.url)).await
}

async fn subscribe_with(
    w: &World,
    name: &str,
    _receiver: &Receiver,
    settings: SubscriptionSettings,
) -> SubscriptionRecord {
    let now = now();
    let record = SubscriptionRecord {
        id: SubscriptionId::generate(),
        tenant_id: w.admin.tenant_id(),
        namespace_id: w.namespace.id(),
        name: slug(name),
        destination: OutboundDestination::Webhook {
            url: settings.url,
            signing_secret: settings.secret_name,
        },
        kinds: settings.kinds,
        limits: settings.limits,
        status: settings.status,
        health: sagat_application::outbound::Health::default(),
        created_by: "test".to_owned(),
        created_at: now,
        updated_at: now,
    };
    let audit = sagat_domain::audit::AuditRecord::provisioning(
        w.admin.tenant_id(),
        sagat_domain::audit::AuditAction::WebhookSubscriptionCreated,
        sagat_domain::audit::ResourceType::WebhookSubscription,
        record.id.to_string(),
        "test",
        now,
    );
    w.store
        .create_subscription(&record, 100, &audit)
        .await
        .expect("subscribe");
    record
}

/// Runs a one-activity workflow to the end: a `run.finished` is committed.
async fn finish_a_run(w: &World, name: &str) -> sagat_domain::execution::RunId {
    let (workflow, _) = w
        .publish(document(name, "work", vec![activity("work")]))
        .await;
    let run = w.start(workflow, &json!({})).await;
    let session = w.session(&["act.work"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({"ok": true}))
        .await
        .expect("complete");
    run
}

async fn deliveries_of(w: &World, subscription: &SubscriptionRecord) -> Vec<DeliveryRecord> {
    w.outbound
        .deliveries(&w.admin, subscription.id, None, None, 100)
        .await
        .expect("deliveries")
}

/// Makes everything pending due now and every subscription ready to be offered.
async fn make_due(w: &World) {
    sqlx::query("UPDATE webhook_deliveries SET next_attempt_at = clock_timestamp() WHERE status = 'PENDING'")
        .execute(&w.db.pool)
        .await
        .expect("due");
    sqlx::query("UPDATE webhook_subscriptions SET next_slot_at = '-infinity'")
        .execute(&w.db.pool)
        .await
        .expect("slots");
}

#[tokio::test]
async fn a_finished_run_is_delivered_signed_to_every_subscription_that_asked() {
    let w = with_secret().await;
    let (wanted, other) = (Receiver::start().await, Receiver::start().await);
    let a = subscribe(&w, "first", &wanted).await;
    let b = subscribe(&w, "second", &wanted).await;
    let mut paused_settings = settings_for(&other.url);
    paused_settings.status = SubscriptionStatus::Paused;
    let paused = subscribe_with(&w, "paused", &other, paused_settings).await;

    let run = finish_a_run(&w, "pay").await;
    let round = w.deliveries.tick().await;
    assert_eq!((round.attempted, round.delivered), (2, 2));

    assert_eq!(wanted.seen().len(), 2, "one request per subscription");
    assert!(
        other.seen().is_empty(),
        "a paused subscription is owed nothing"
    );
    assert!(deliveries_of(&w, &paused).await.is_empty());

    let delivery = &deliveries_of(&w, &a).await[0];
    assert_eq!(delivery.status, DeliveryStatus::Delivered);
    assert_eq!(delivery.attempts, 1);
    // The event is the one that was committed with the run: same id as the outbox message.
    let outbox: String =
        sqlx::query_scalar("SELECT id::text FROM event_outbox WHERE event_type = 'run.finished'")
            .fetch_one(&w.db.pool)
            .await
            .expect("outbox");
    assert_eq!(delivery.event_id, outbox);
    assert_eq!(
        deliveries_of(&w, &b).await[0].event_id,
        outbox,
        "the same event for both"
    );

    // The two subscriptions are delivered at the same time, so which request arrives first is not
    // decided: the one that is asserted on is the one of the first subscription's delivery.
    let all = wanted.seen();
    let seen = all
        .iter()
        .find(|request| request.headers["x-octopus-delivery-id"] == delivery.id.to_string())
        .expect("the request of the first subscription's delivery");
    let body: Value = serde_json::from_slice(&seen.body).expect("json");
    assert_eq!(body["id"], outbox);
    assert_eq!(body["type"], "run.finished");
    assert_eq!(body["data"]["runId"], run.to_string());
    assert_eq!(body["data"]["status"], "SUCCEEDED");
    assert_eq!(body["data"]["workflow"], "pay");

    // Signed the way the receiver can reproduce: with the shared secret, over the instant, the id, the
    // type and the body.
    assert!(
        crate::support::signed::delivery_verifies(SECRET, &seen.headers, &seen.body),
        "the receiver can verify what it was sent"
    );
    assert_eq!(seen.headers["x-octopus-event-id"], outbox);
    assert_eq!(
        seen.headers["x-octopus-delivery-id"],
        delivery.id.to_string()
    );
    assert_eq!(seen.headers["x-octopus-attempt"], "1");
    assert_eq!(seen.headers["x-octopus-event-type"], "run.finished");
    assert_eq!(seen.headers["content-type"], "application/json");
    assert!(
        !String::from_utf8_lossy(&seen.body).contains(SECRET),
        "the secret is never in what is sent"
    );
}

#[tokio::test]
async fn a_temporary_503_is_retried_with_backoff_and_every_attempt_is_recorded() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    receiver.script([Reply::Status(503), Reply::Status(503), Reply::Status(200)]);
    let subscription = subscribe(&w, "flaky", &receiver).await;
    finish_a_run(&w, "pay").await;

    // First attempt: 503, and the next one is a few seconds away, not now.
    assert_eq!(w.deliveries.tick().await.retried, 1);
    let first = deliveries_of(&w, &subscription).await.remove(0);
    assert_eq!((first.status, first.attempts), (DeliveryStatus::Pending, 1));
    assert_eq!(first.last_outcome, Some(AttemptOutcome::ServerError));
    assert_eq!(first.last_http_status, Some(503));
    let wait: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM next_attempt_at - clock_timestamp())::float8 FROM webhook_deliveries",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("wait");
    assert!(
        (3.0..=6.5).contains(&wait),
        "5 seconds spread by jitter, not {wait}"
    );
    assert!(
        !w.deliveries.tick().await.did_work(),
        "and nothing is sent before then"
    );

    // Time passes. The second attempt fails too and waits longer.
    make_due(&w).await;
    assert_eq!(w.deliveries.tick().await.retried, 1);
    let wait: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM next_attempt_at - clock_timestamp())::float8 FROM webhook_deliveries",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("wait");
    assert!(
        (20.0..=37.0).contains(&wait),
        "30 seconds spread by jitter, not {wait}"
    );

    // The third gets through.
    make_due(&w).await;
    assert_eq!(w.deliveries.tick().await.delivered, 1);

    let (delivery, attempts) = w
        .outbound
        .delivery(&w.admin, subscription.id, first.id)
        .await
        .expect("delivery");
    assert_eq!(
        (delivery.status, delivery.attempts),
        (DeliveryStatus::Delivered, 3)
    );
    let outcomes: Vec<_> = attempts
        .iter()
        .map(|a| (a.attempt, a.outcome, a.http_status))
        .collect();
    assert_eq!(
        outcomes,
        vec![
            (1, AttemptOutcome::ServerError, Some(503)),
            (2, AttemptOutcome::ServerError, Some(503)),
            (3, AttemptOutcome::Delivered, Some(200)),
        ],
        "all the attempts are recorded"
    );
    // The receiver saw the same delivery id each time, which is what lets it drop a repeat.
    let seen = receiver.seen();
    assert_eq!(seen.len(), 3);
    assert!(
        seen.iter()
            .all(|s| s.headers["x-octopus-delivery-id"] == first.id.to_string())
    );
    assert_eq!(
        seen.iter()
            .map(|s| s.headers["x-octopus-attempt"].as_str())
            .collect::<Vec<_>>(),
        ["1", "2", "3"]
    );
}

#[tokio::test]
async fn a_receiver_that_asks_for_patience_is_obeyed() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    receiver.script([Reply::StatusAfter(429, 600)]);
    let subscription = subscribe(&w, "throttled", &receiver).await;
    finish_a_run(&w, "pay").await;

    w.deliveries.tick().await;
    let delivery = deliveries_of(&w, &subscription).await.remove(0);
    assert_eq!(delivery.last_outcome, Some(AttemptOutcome::Throttled));
    let wait: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM next_attempt_at - clock_timestamp())::float8 FROM webhook_deliveries",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("wait");
    assert!(
        wait > 590.0,
        "the receiver asked for 10 minutes, the schedule says 5 seconds: {wait}"
    );
}

#[tokio::test]
async fn a_refusal_is_set_aside_at_once_and_a_redrive_delivers_it_with_its_history_kept() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    receiver.script([Reply::Status(400)]);
    let subscription = subscribe(&w, "strict", &receiver).await;
    finish_a_run(&w, "pay").await;

    let round = w.deliveries.tick().await;
    assert_eq!((round.dead, round.retried), (1, 0), "a 400 is not retried");
    let dead = w
        .outbound
        .deliveries(
            &w.admin,
            subscription.id,
            Some(DeliveryStatus::Dead),
            None,
            10,
        )
        .await
        .expect("dead");
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].last_outcome, Some(AttemptOutcome::Rejected));

    // An operator who can read but not manage cannot redrive; one who can, does.
    let viewer = w.member("viewer-1", Role::Viewer).await;
    assert!(matches!(
        w.outbound
            .redrive(&viewer, subscription.id, None, 10, &ctx())
            .await,
        Err(AppError::Forbidden { .. })
    ));
    assert_eq!(
        w.outbound
            .redrive(&w.admin, subscription.id, Some(dead[0].id), 1, &ctx())
            .await,
        Ok(1)
    );
    assert_eq!(
        w.outbound
            .redrive(&w.admin, subscription.id, Some(dead[0].id), 1, &ctx())
            .await,
        Ok(0),
        "a delivery that is no longer dead is not put back twice"
    );
    make_due(&w).await;
    assert_eq!(w.deliveries.tick().await.delivered, 1);

    let (delivery, attempts) = w
        .outbound
        .delivery(&w.admin, subscription.id, dead[0].id)
        .await
        .expect("delivery");
    assert_eq!(delivery.status, DeliveryStatus::Delivered);
    assert_eq!(
        attempts
            .iter()
            .map(|a| (a.attempt, a.outcome))
            .collect::<Vec<_>>(),
        vec![
            (1, AttemptOutcome::Rejected),
            (2, AttemptOutcome::Delivered)
        ],
        "the attempt that failed is not forgotten by the one that worked"
    );
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'webhook.delivery_redriven'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        audited, 2,
        "every redrive that was allowed is audited, even one that moved nothing"
    );
}

#[tokio::test]
async fn a_delivery_that_fails_every_attempt_is_set_aside_and_a_redrive_gives_it_a_full_set_again()
{
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    receiver.answer(500);
    let subscription = subscribe(&w, "down", &receiver).await;
    finish_a_run(&w, "pay").await;

    for round in 1..=12 {
        make_due(&w).await;
        let result = w.deliveries.tick().await;
        assert_eq!(result.attempted, 1, "round {round}");
        if round < 12 {
            assert_eq!(result.retried, 1, "round {round}");
        } else {
            assert_eq!(result.dead, 1, "the twelfth attempt is the last");
        }
    }
    let delivery = deliveries_of(&w, &subscription).await.remove(0);
    assert_eq!(
        (delivery.status, delivery.attempts),
        (DeliveryStatus::Dead, 12)
    );

    // Back in the queue with a full set of attempts again, numbered on from where it stopped.
    assert_eq!(
        w.outbound
            .redrive(&w.admin, subscription.id, None, 100, &ctx())
            .await,
        Ok(1)
    );
    receiver.answer(200);
    make_due(&w).await;
    assert_eq!(w.deliveries.tick().await.delivered, 1);
    let (_, attempts) = w
        .outbound
        .delivery(&w.admin, subscription.id, delivery.id)
        .await
        .expect("delivery");
    assert_eq!(attempts.len(), 13);
    assert_eq!(attempts.last().map(|a| a.attempt), Some(13));
}

#[tokio::test]
async fn a_receiver_that_does_not_answer_in_time_is_a_timeout_and_is_retried() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    receiver.script([Reply::Slow(Duration::from_secs(4))]);
    let mut settings = settings_for(&receiver.url);
    settings.limits = DeliveryLimits::new(100, Duration::from_secs(1)).expect("limits");
    let subscription = subscribe_with(&w, "slow", &receiver, settings).await;
    finish_a_run(&w, "pay").await;

    let started = std::time::Instant::now();
    assert_eq!(w.deliveries.tick().await.retried, 1);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the worker did not wait for the receiver"
    );
    let (delivery, attempts) = w
        .outbound
        .delivery(
            &w.admin,
            subscription.id,
            deliveries_of(&w, &subscription).await[0].id,
        )
        .await
        .expect("delivery");
    assert_eq!(delivery.status, DeliveryStatus::Pending);
    assert_eq!(attempts[0].outcome, AttemptOutcome::Timeout);
    assert_eq!(attempts[0].http_status, None);
}

#[tokio::test]
async fn what_the_receiver_says_is_never_stored() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    receiver.script([Reply::StatusBody(500, "internal token=hunter2-do-not-keep")]);
    let subscription = subscribe(&w, "chatty", &receiver).await;
    finish_a_run(&w, "pay").await;
    w.deliveries.tick().await;

    let delivery = deliveries_of(&w, &subscription).await.remove(0);
    let error = delivery.last_error.expect("a class of failure is kept");
    assert!(!error.contains("hunter2"), "{error}");
    let everything: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(coalesce(error, '') || coalesce(last_error, ''), ' '), '') \
         FROM webhook_deliveries d LEFT JOIN webhook_attempts a ON a.delivery_id = d.id",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("text");
    assert!(!everything.contains("hunter2"));
}

#[tokio::test]
async fn the_rate_of_a_subscription_is_enforced_across_claims() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    let mut settings = settings_for(&receiver.url);
    settings.limits = DeliveryLimits::new(2, Duration::from_secs(2)).expect("limits");
    let subscription = subscribe_with(&w, "metered", &receiver, settings).await;
    for i in 0..5 {
        sqlx::query(
            "INSERT INTO webhook_deliveries (id, tenant_id, namespace_id, subscription_id, event_id, \
             kind, payload, next_attempt_at) \
             VALUES (gen_random_uuid(), $1, $2, $3, $4, 'run.finished', '{}', clock_timestamp())",
        )
        .bind(subscription.tenant_id.as_uuid())
        .bind(subscription.namespace_id.as_uuid())
        .bind(subscription.id.as_uuid())
        .bind(format!("event-{i}"))
        .execute(&w.db.pool)
        .await
        .expect("queue");
    }

    assert_eq!(
        w.deliveries.tick().await.attempted,
        1,
        "a claim takes a share of one second's rate"
    );
    assert_eq!(
        w.deliveries.tick().await.attempted,
        0,
        "and the subscription is not offered again until its slot has come"
    );
    assert_eq!(receiver.seen().len(), 1);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(w.deliveries.tick().await.attempted, 1);
    assert_eq!(receiver.seen().len(), 2);
}

#[tokio::test]
async fn a_subscription_whose_deliveries_keep_dying_is_switched_off_and_turning_it_on_redrives() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    receiver.answer(404);
    let subscription = subscribe(&w, "gone", &receiver).await;
    for i in 0..25 {
        sqlx::query(
            "INSERT INTO webhook_deliveries (id, tenant_id, namespace_id, subscription_id, event_id, \
             kind, payload, next_attempt_at) \
             VALUES (gen_random_uuid(), $1, $2, $3, $4, 'run.finished', '{}', clock_timestamp())",
        )
        .bind(subscription.tenant_id.as_uuid())
        .bind(subscription.namespace_id.as_uuid())
        .bind(subscription.id.as_uuid())
        .bind(format!("event-{i}"))
        .execute(&w.db.pool)
        .await
        .expect("queue");
    }
    let mut switched = 0;
    for _ in 0..40 {
        make_due(&w).await;
        switched += w.deliveries.tick().await.switched_off;
        let status = w
            .outbound
            .get(&w.admin, subscription.id)
            .await
            .expect("get")
            .status;
        if status == SubscriptionStatus::Disabled {
            break;
        }
    }
    let off = w
        .outbound
        .get(&w.admin, subscription.id)
        .await
        .expect("get");
    assert_eq!(off.status, SubscriptionStatus::Disabled);
    assert_eq!(switched, 1, "it is switched off once");
    assert!(
        off.health.consecutive_dead >= 20,
        "the deliveries already in flight when it was switched off still ended: {}",
        off.health.consecutive_dead
    );
    assert!(off.health.disabled_reason.is_some());
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'webhook.subscription_disabled'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(audited, 1, "and the platform says so in the trail");

    // Nothing new is queued for it, and what is pending is not sent.
    finish_a_run(&w, "pay").await;
    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM webhook_deliveries WHERE subscription_id = $1 AND event_id NOT LIKE 'event-%'",
    )
    .bind(subscription.id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(queued, 0);
    let before = receiver.seen().len();
    make_due(&w).await;
    w.deliveries.tick().await;
    assert_eq!(receiver.seen().len(), before);

    // The administrator turns it back on: the failures are forgotten, the dead are delivered again.
    // (The destination is checked again against the policy, which cannot allow a loopback address:
    // the administrator points it at the receiver's public name.)
    w.egress
        .set(
            &w.admin,
            w.namespace.id(),
            &["hooks.example.com".to_owned()],
            &[],
            false,
            &ctx(),
        )
        .await
        .expect("policy");
    let mut on = settings_for("https://hooks.example.com/in");
    on.status = SubscriptionStatus::Active;
    let back = w
        .outbound
        .update(&w.admin, subscription.id, on, &ctx())
        .await
        .expect("on");
    assert_eq!(back.health.consecutive_dead, 0);
    let dead_left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM webhook_deliveries WHERE subscription_id = $1 AND status = 'DEAD'",
    )
    .bind(subscription.id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(dead_left, 0, "what was set aside is queued again");
}

#[tokio::test]
async fn a_secret_that_is_gone_stops_deliveries_without_losing_them() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    let subscription = subscribe(&w, "unsigned", &receiver).await;
    finish_a_run(&w, "pay").await;
    w.secrets
        .delete(&w.admin, w.namespace.id(), SECRET_NAME, &ctx())
        .await
        .expect("delete the secret");

    assert_eq!(w.deliveries.tick().await.retried, 1);
    assert!(receiver.seen().is_empty(), "nothing unsigned is ever sent");
    let delivery = deliveries_of(&w, &subscription).await.remove(0);
    assert_eq!(delivery.status, DeliveryStatus::Pending);
    assert_eq!(delivery.last_outcome, Some(AttemptOutcome::Unsigned));

    // The secret comes back; the delivery goes through.
    w.put_secret(SECRET_NAME, SECRET).await;
    make_due(&w).await;
    assert_eq!(w.deliveries.tick().await.delivered, 1);
}

#[tokio::test]
async fn after_a_rotation_a_delivery_carries_a_signature_for_each_of_the_two_newest_secrets() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    subscribe(&w, "rotating", &receiver).await;
    let newer = "second-secret-second-secret-second-0";
    w.put_secret(SECRET_NAME, newer).await;
    finish_a_run(&w, "pay").await;
    w.deliveries.tick().await;

    let seen = &receiver.seen()[0];
    for secret in [SECRET, newer] {
        assert!(
            crate::support::signed::delivery_verifies(secret, &seen.headers, &seen.body),
            "a receiver that still holds the previous secret verifies the request too"
        );
    }
}

#[tokio::test]
async fn subscriptions_belong_to_their_tenant_and_the_run_of_another_is_not_theirs() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    let mine = subscribe(&w, "mine", &receiver).await;
    finish_a_run(&w, "pay").await;

    assert_eq!(
        w.outbound.get(&w.other_admin, mine.id).await.map(|_| ()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.outbound
            .update(&w.other_admin, mine.id, settings_for(&receiver.url), &ctx())
            .await
            .map(|_| ()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.outbound.delete(&w.other_admin, mine.id, &ctx()).await,
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.outbound
            .deliveries(&w.other_admin, mine.id, None, None, 10)
            .await
            .map(|d| d.len()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.outbound
            .redrive(&w.other_admin, mine.id, None, 10, &ctx())
            .await,
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.outbound
            .list(&w.other_admin, w.namespace.id(), &ctx())
            .await
            .map(|v| v.len()),
        Err(AppError::NotFound),
        "nor list the subscriptions of a namespace that is not theirs"
    );

    // Another tenant's subscription (in a namespace of the same name) is owed nothing by my run.
    let now = now();
    let theirs = SubscriptionRecord {
        id: SubscriptionId::generate(),
        tenant_id: w.other_admin.tenant_id(),
        namespace_id: w.other_namespace.id(),
        name: slug("theirs"),
        destination: OutboundDestination::Webhook {
            url: TargetUrl::parse(&receiver.url).expect("url"),
            signing_secret: SecretName::parse(SECRET_NAME).expect("secret"),
        },
        kinds: vec![OutboundKind::RunFinished],
        limits: DeliveryLimits::default(),
        status: SubscriptionStatus::Active,
        health: sagat_application::outbound::Health::default(),
        created_by: "test".to_owned(),
        created_at: now,
        updated_at: now,
    };
    let audit = sagat_domain::audit::AuditRecord::provisioning(
        w.other_admin.tenant_id(),
        sagat_domain::audit::AuditAction::WebhookSubscriptionCreated,
        sagat_domain::audit::ResourceType::WebhookSubscription,
        theirs.id.to_string(),
        "test",
        now,
    );
    w.store
        .create_subscription(&theirs, 100, &audit)
        .await
        .expect("their subscription");
    finish_a_run(&w, "second-pay").await;
    let owed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM webhook_deliveries WHERE subscription_id = $1")
            .bind(theirs.id.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(
        owed, 0,
        "an event is owed only to the subscriptions of its own tenant and namespace"
    );
    assert_eq!(deliveries_of(&w, &mine).await.len(), 2);
}

#[tokio::test]
async fn the_service_checks_the_destination_the_secret_the_kinds_the_name_and_the_quota() {
    let w = with_secret().await;
    let url = "https://hooks.example.com/in";

    // Default-deny: the namespace may reach nothing until its policy says so.
    let refused = w
        .outbound
        .create(
            &w.admin,
            w.namespace.id(),
            slug("billing-hook"),
            settings_for(url),
            &ctx(),
        )
        .await;
    assert_eq!(
        refused.map(|_| ()),
        Err(AppError::NotAllowed(
            "the egress policy of the namespace does not allow that destination"
        ))
    );
    w.egress
        .set(
            &w.admin,
            w.namespace.id(),
            &["hooks.example.com".to_owned()],
            &[],
            false,
            &ctx(),
        )
        .await
        .expect("policy");
    let created = w
        .outbound
        .create(
            &w.admin,
            w.namespace.id(),
            slug("billing-hook"),
            settings_for(url),
            &ctx(),
        )
        .await
        .expect("created");
    assert_eq!(created.status, SubscriptionStatus::Active);

    // Plain http needs the policy to allow it, and another host needs a rule.
    for refused in ["http://hooks.example.com/in", "https://evil.example.net/in"] {
        assert!(
            matches!(
                w.outbound
                    .update(&w.admin, created.id, settings_for(refused), &ctx())
                    .await,
                Err(AppError::NotAllowed(_))
            ),
            "{refused}"
        );
    }
    // The name is unique in the namespace, the kinds are checked and the platform alone disables.
    assert_eq!(
        w.outbound
            .create(
                &w.admin,
                w.namespace.id(),
                slug("billing-hook"),
                settings_for(url),
                &ctx()
            )
            .await
            .map(|_| ()),
        Err(AppError::Conflict(ConflictKind::WebhookSubscriptionName))
    );
    let mut none = settings_for(url);
    none.kinds = vec![];
    assert!(matches!(
        w.outbound
            .create(&w.admin, w.namespace.id(), slug("no-kinds"), none, &ctx())
            .await,
        Err(AppError::NotAllowed(_))
    ));
    let mut off = settings_for(url);
    off.status = SubscriptionStatus::Disabled;
    assert!(matches!(
        w.outbound
            .create(&w.admin, w.namespace.id(), slug("off"), off, &ctx())
            .await,
        Err(AppError::NotAllowed(_))
    ));
    // A secret that does not exist, or that the caller may not use, is refused without saying which.
    let mut no_secret = settings_for(url);
    no_secret.secret_name = SecretName::parse("nothing-here").expect("name");
    assert!(matches!(
        w.outbound
            .create(
                &w.admin,
                w.namespace.id(),
                slug("no-secret"),
                no_secret,
                &ctx()
            )
            .await,
        Err(AppError::SecretUnavailable { .. })
    ));
    // The quota counts subscriptions per namespace.
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::WebhookSubscriptions,
            1,
            &ctx(),
        )
        .await
        .expect("limit");
    assert_eq!(
        w.outbound
            .create(
                &w.admin,
                w.namespace.id(),
                slug("one-too-many"),
                settings_for(url),
                &ctx()
            )
            .await
            .map(|_| ()),
        Err(AppError::QuotaExceeded(
            "outbound webhook subscriptions in the namespace"
        ))
    );
    // Reading needs only `event:read`, changing needs `event:manage`.
    let viewer = w.member("viewer-2", Role::Viewer).await;
    assert!(w.outbound.get(&viewer, created.id).await.is_ok());
    assert!(matches!(
        w.outbound.delete(&viewer, created.id, &ctx()).await,
        Err(AppError::Forbidden { .. })
    ));
}

#[tokio::test]
async fn deleting_a_subscription_removes_what_was_queued_for_it_and_audits_it() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    let subscription = subscribe(&w, "short-lived", &receiver).await;
    finish_a_run(&w, "pay").await;
    assert_eq!(deliveries_of(&w, &subscription).await.len(), 1);

    w.outbound
        .delete(&w.admin, subscription.id, &ctx())
        .await
        .expect("deleted");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM webhook_deliveries")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(left, 0);
    assert!(!w.deliveries.tick().await.did_work());
    assert!(receiver.seen().is_empty());
}

#[tokio::test]
async fn changing_the_settings_or_resuming_a_subscription_touches_nothing_that_was_set_aside() {
    let w = with_secret().await;
    w.egress
        .set(
            &w.admin,
            w.namespace.id(),
            &["hooks.example.com".to_owned()],
            &[],
            false,
            &ctx(),
        )
        .await
        .expect("policy");
    let url = "https://hooks.example.com/in";
    let receiver = Receiver::start().await;
    let subscription = subscribe_with(&w, "running", &receiver, settings_for(url)).await;
    sqlx::query(
        "INSERT INTO webhook_deliveries (id, tenant_id, namespace_id, subscription_id, event_id, \
         kind, payload, status, attempts, next_attempt_at) \
         VALUES (gen_random_uuid(), $1, $2, $3, 'dead-one', 'run.finished', '{}', 'DEAD', 12, clock_timestamp())",
    )
    .bind(subscription.tenant_id.as_uuid())
    .bind(subscription.namespace_id.as_uuid())
    .bind(subscription.id.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("a dead delivery");
    sqlx::query("UPDATE webhook_subscriptions SET consecutive_dead = 7 WHERE id = $1")
        .bind(subscription.id.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("failures");

    // An edit of a subscription that is running: the dead stay dead and the failures are kept.
    let mut edit = settings_for(url);
    edit.limits = DeliveryLimits::new(5, Duration::from_secs(3)).expect("limits");
    let edited = w
        .outbound
        .update(&w.admin, subscription.id, edit, &ctx())
        .await
        .expect("edit");
    assert_eq!(
        edited.health.consecutive_dead, 7,
        "an edit does not hide failures"
    );
    assert_eq!(edited.limits.rate_per_second(), 5);
    let dead = w
        .outbound
        .deliveries(
            &w.admin,
            subscription.id,
            Some(DeliveryStatus::Dead),
            None,
            10,
        )
        .await
        .expect("dead");
    assert_eq!(dead.len(), 1, "nothing was queued again by an edit");

    // Paused and resumed: that changes nothing that was set aside. Turning on a subscription the
    // platform switched off is what says the receiver is back (the next test); a resume is not.
    let mut pause = settings_for(url);
    pause.status = SubscriptionStatus::Paused;
    w.outbound
        .update(&w.admin, subscription.id, pause, &ctx())
        .await
        .expect("pause");
    let on = w
        .outbound
        .update(&w.admin, subscription.id, settings_for(url), &ctx())
        .await
        .expect("on");
    assert_eq!(
        on.health.consecutive_dead, 7,
        "a resume does not forget failures"
    );
    let pending = w
        .outbound
        .deliveries(
            &w.admin,
            subscription.id,
            Some(DeliveryStatus::Pending),
            None,
            10,
        )
        .await
        .expect("pending");
    assert!(pending.is_empty(), "and the dead stay dead until redriven");
    let dead = w
        .outbound
        .deliveries(
            &w.admin,
            subscription.id,
            Some(DeliveryStatus::Dead),
            None,
            10,
        )
        .await
        .expect("dead");
    assert_eq!(dead.len(), 1);
}

async fn queue_one(w: &World, subscription: &SubscriptionRecord, event: &str) {
    sqlx::query(
        "INSERT INTO webhook_deliveries (id, tenant_id, namespace_id, subscription_id, event_id, \
         kind, payload, next_attempt_at) \
         VALUES (gen_random_uuid(), $1, $2, $3, $4, 'run.finished', '{}', clock_timestamp())",
    )
    .bind(subscription.tenant_id.as_uuid())
    .bind(subscription.namespace_id.as_uuid())
    .bind(subscription.id.as_uuid())
    .bind(event)
    .execute(&w.db.pool)
    .await
    .expect("queue");
}

fn attempt(
    number: u32,
    outcome: AttemptOutcome,
    status: u16,
) -> sagat_application::outbound::AttemptRecord {
    sagat_application::outbound::AttemptRecord {
        attempt: number,
        started_at: now(),
        duration_ms: 5,
        outcome,
        http_status: Some(status),
        error: None,
    }
}

#[tokio::test]
async fn a_worker_whose_lease_ran_out_cannot_undo_what_another_one_recorded() {
    use sagat_application::outbound::{DeliveryStore, Next};
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    let subscription = subscribe(&w, "slow", &receiver).await;
    queue_one(&w, &subscription, "event-1").await;

    // Worker A claims the delivery and stalls; its lease runs out; worker B claims it and delivers.
    let a = w
        .store
        .claim_deliveries(10, Duration::from_secs(90))
        .await
        .expect("claim");
    assert_eq!(a.len(), 1);
    make_due(&w).await;
    let b = w
        .store
        .claim_deliveries(10, Duration::from_secs(90))
        .await
        .expect("claim");
    assert_eq!(b.len(), 1);
    w.store
        .record_attempt(
            &b[0],
            &attempt(1, AttemptOutcome::Delivered, 200),
            &Next::Delivered,
            None,
        )
        .await
        .expect("B records");

    // A wakes up with a failure. What B recorded stands; A's attempt is not the delivery's story.
    w.store
        .record_attempt(
            &a[0],
            &attempt(1, AttemptOutcome::ServerError, 503),
            &Next::Retry {
                delay: Duration::from_secs(5),
            },
            None,
        )
        .await
        .expect("A records");
    let (status, attempts): (String, i32) = sqlx::query_as(
        "SELECT status, attempts FROM webhook_deliveries WHERE event_id = 'event-1'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("row");
    assert_eq!(
        (status.as_str(), attempts),
        ("DELIVERED", 1),
        "a delivered delivery is not sent again"
    );

    // Nor does a late verdict of "set aside" count against the subscription or change the delivery.
    let off = w
        .store
        .record_attempt(
            &a[0],
            &attempt(1, AttemptOutcome::Rejected, 404),
            &Next::Dead,
            None,
        )
        .await
        .expect("A records");
    assert!(!off);
    let (status, dead): (String, i32) = sqlx::query_as(
        "SELECT d.status, s.consecutive_dead FROM webhook_deliveries d \
         JOIN webhook_subscriptions s ON s.id = d.subscription_id WHERE d.event_id = 'event-1'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("row");
    assert_eq!((status.as_str(), dead), ("DELIVERED", 0));
    let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM webhook_attempts")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(recorded, 1, "one attempt, B's");
}

#[tokio::test]
async fn the_deliveries_of_a_subscription_are_listed_newest_first_and_paged_without_gaps() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    let subscription = subscribe(&w, "listed", &receiver).await;
    let (workflow, _) = w
        .publish(document("finishes", "work", vec![activity("work")]))
        .await;
    let session = w.session(&["act.work"]).await;
    let mut events = Vec::new();
    for _ in 0..10 {
        let run = w.start(workflow, &json!({})).await;
        let task = w.one_task(&session).await;
        assert_eq!(task.run_id, run);
        w.complete(&session, &task, &json!({}))
            .await
            .expect("complete");
        // Distinct instants, as runs that finish one after the other have.
        tokio::time::sleep(Duration::from_millis(3)).await;
        events.push(run.to_string());
    }

    let mut seen = Vec::new();
    let mut before = None;
    loop {
        let page = w
            .outbound
            .deliveries(&w.admin, subscription.id, None, before, 4)
            .await
            .expect("page");
        if page.is_empty() {
            break;
        }
        before = page.last().map(|d| d.id);
        seen.extend(page);
    }
    assert_eq!(
        seen.len(),
        10,
        "every delivery once, none missed between pages"
    );
    let created: Vec<_> = seen.iter().map(|d| d.created_at).collect();
    let mut newest_first = created.clone();
    newest_first.sort_by(|a, b| b.cmp(a));
    assert_eq!(created, newest_first, "newest first");
    let order: Vec<String> = seen
        .iter()
        .map(|d| {
            serde_json::from_slice::<Value>(&d.payload).expect("json")["data"]["runId"]
                .as_str()
                .expect("run")
                .to_owned()
        })
        .collect();
    let mut expected = events;
    expected.reverse();
    assert_eq!(
        order, expected,
        "in the order the runs finished, latest first"
    );
}

#[tokio::test]
async fn a_subscription_that_cannot_keep_up_is_switched_off_instead_of_queueing_for_ever() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    let slow = subscribe(&w, "slow", &receiver).await;
    let healthy = subscribe(&w, "healthy", &receiver).await;
    // Ten thousand deliveries waiting for the slow one already.
    sqlx::query(
        "INSERT INTO webhook_deliveries (id, tenant_id, namespace_id, subscription_id, event_id, \
         kind, payload, next_attempt_at) \
         SELECT gen_random_uuid(), $1, $2, $3, 'old-' || n, 'run.finished', '{}', \
                clock_timestamp() + interval '1 day' \
         FROM generate_series(1, 10000) AS n",
    )
    .bind(slow.tenant_id.as_uuid())
    .bind(slow.namespace_id.as_uuid())
    .bind(slow.id.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("a backlog");

    finish_a_run(&w, "pay").await;

    let (status, reason): (String, Option<String>) =
        sqlx::query_as("SELECT status, disabled_reason FROM webhook_subscriptions WHERE id = $1")
            .bind(slow.id.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("row");
    assert_eq!(status, "DISABLED");
    assert!(reason.expect("a reason").contains("waiting"));
    let queued = |id: SubscriptionId| {
        let pool = w.db.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM webhook_deliveries WHERE subscription_id = $1 \
                 AND event_id NOT LIKE 'old-%'",
            )
            .bind(id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("count")
        }
    };
    assert_eq!(queued(slow.id).await, 0, "nothing more is queued for it");
    assert_eq!(queued(healthy.id).await, 1, "the others get theirs");
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'webhook.subscription_disabled' \
         AND resource_id = $1",
    )
    .bind(slow.id.to_string())
    .fetch_one(&w.db.pool)
    .await
    .expect("count");
    assert_eq!(audited, 1, "and the platform says so in the trail");
}

#[tokio::test]
async fn a_receiver_that_answers_with_a_page_has_still_been_delivered_to() {
    let w = with_secret().await;
    let receiver = Receiver::start().await;
    // Over a megabyte of thanks (more than any client of ours reads), then a 404 page as long.
    receiver.script([Reply::Big(200, 1_500_000), Reply::Big(404, 1_500_000)]);
    let subscription = subscribe(&w, "chatty", &receiver).await;
    finish_a_run(&w, "pay").await;
    finish_a_run(&w, "pay-again").await;
    make_due(&w).await;

    let sender = sagat_infra_connectors::webhook::GuardedSender::with_factory(
        w.store.clone(),
        crate::support::receiver::OpenFactory,
    );
    let worker = sagat_scheduler::DeliveryWorker::new(
        w.store.clone(),
        sender,
        sagat_application::entropy::SystemEntropy,
        w.keys.cipher(),
        sagat_scheduler::DeliverySettings::default(),
    );
    let round = worker.tick().await;
    assert_eq!(
        (round.attempted, round.delivered, round.dead, round.retried),
        (2, 1, 1, 0),
        "the answer is judged by its status, whatever the size of its body"
    );
    let delivered = deliveries_of(&w, &subscription).await;
    assert_eq!(
        delivered
            .iter()
            .filter(|d| d.status == DeliveryStatus::Delivered)
            .count(),
        1
    );
    assert_eq!(
        delivered
            .iter()
            .filter(|d| d.status == DeliveryStatus::Dead)
            .count(),
        1,
        "a refusal with a page is a refusal, not a network failure to retry"
    );
}
