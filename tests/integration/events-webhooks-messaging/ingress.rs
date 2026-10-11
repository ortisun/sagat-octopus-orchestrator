//! US01 and the ingress promises: an event is admitted once, only if it verifies, and what is
//! refused leaves nothing behind but a collapsed audit record (FR-004-002..004, FR-004-008).

use sagat_application::error::{AppError, ConflictKind};
use sagat_application::events::{
    Admission, BACKLOG_LIMIT, EventStore, NewEvent, PublishRequest, WebhookRequest,
};
use sagat_domain::events::{
    EndpointLimits, EndpointStatus, EventEnvelope, EventId, EventKey, InboxStatus,
};
use sagat_domain::identity::Role;
use sagat_domain::workflow::EventType;

use crate::support::engine::ctx;
use crate::support::{self, slug};
use crate::{SECRET, SECRET_NAME, Signed, now_seconds, settings, with_endpoint};

async fn inbox_count(w: &support::engine::World) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM inbox_events")
        .fetch_one(&w.db.pool)
        .await
        .expect("count")
}

async fn audit_count(w: &support::engine::World, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_records WHERE action = $1")
        .bind(action)
        .fetch_one(&w.db.pool)
        .await
        .expect("count")
}

#[tokio::test]
async fn a_signed_webhook_is_admitted_once_and_a_retry_is_a_duplicate() {
    let (w, endpoint) = with_endpoint().await;
    let signed = Signed::typed(
        SECRET,
        now_seconds(),
        "evt-1",
        "invoice.paid",
        r#"{"amount":10}"#,
    );

    let first = w
        .events
        .receive(endpoint.id, &signed.request(Some("invoice.paid")))
        .await
        .expect("admitted");
    assert!(!first.duplicate);
    assert_eq!(first.key.source.as_str(), "billing");

    let again = w
        .events
        .receive(endpoint.id, &signed.request(Some("invoice.paid")))
        .await
        .expect("a retry is answered the same");
    assert!(again.duplicate, "the second is reported as a duplicate");
    assert_eq!(inbox_count(&w).await, 1, "and nothing was written for it");

    let row: (String, String, i32) = sqlx::query_as(
        "SELECT event_type, status, attempts FROM inbox_events WHERE event_id = 'evt-1'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("row");
    assert_eq!(
        row,
        (
            "invoice.paid".to_owned(),
            InboxStatus::Pending.as_str().to_owned(),
            0
        )
    );
}

#[tokio::test]
async fn everything_that_does_not_verify_is_refused_alike_and_leaves_no_event() {
    let (w, endpoint) = with_endpoint().await;
    let now = now_seconds();
    let good = Signed::typed(SECRET, now, "evt-1", "invoice.paid", r#"{"a":1}"#);

    let mut wrong_secret = Signed::typed(
        "another-secret-another-secret-0000",
        now,
        "evt-1",
        "invoice.paid",
        r#"{"a":1}"#,
    );
    wrong_secret.body = good.body.clone();
    let mut tampered = Signed::typed(SECRET, now, "evt-1", "invoice.paid", r#"{"a":1}"#);
    tampered.body = br#"{"a":2}"#.to_vec();
    let stale = Signed::typed(SECRET, now - 3600, "evt-1", "invoice.paid", r#"{"a":1}"#);
    let future = Signed::typed(SECRET, now + 3600, "evt-1", "invoice.paid", r#"{"a":1}"#);
    let mut other_id = Signed::typed(SECRET, now, "evt-1", "invoice.paid", r#"{"a":1}"#);
    other_id.event_id = "evt-2".to_owned();

    for (what, signed) in [
        ("signed with another secret", &wrong_secret),
        ("a body changed after signing", &tampered),
        ("an old timestamp", &stale),
        ("a timestamp from the future", &future),
        ("an event id changed after signing", &other_id),
    ] {
        let result = w
            .events
            .receive(endpoint.id, &signed.request(Some("invoice.paid")))
            .await;
        assert_eq!(result, Err(AppError::Unauthenticated), "{what}");
    }

    let missing = |signature: Option<&'static str>,
                   timestamp: Option<&'static str>,
                   event_id: Option<&'static str>| {
        let body = br#"{"a":1}"#;
        let request = WebhookRequest {
            trace: None,
            signature,
            timestamp,
            event_id,
            event_type: Some("invoice.paid"),
            subject: None,
            correlation_key: None,
            body,
        };
        let events = &w.events;
        async move { events.receive(endpoint.id, &request).await }
    };
    for (what, result) in [
        ("no signature", missing(None, Some("1"), Some("e1")).await),
        (
            "no timestamp",
            missing(Some("v1=00"), None, Some("e1")).await,
        ),
        ("no event id", missing(Some("v1=00"), Some("1"), None).await),
        (
            "a timestamp that is not a number",
            missing(Some("v1=00"), Some("soon"), Some("e1")).await,
        ),
        (
            "an event id with spaces",
            missing(Some("v1=00"), Some("1"), Some("a b")).await,
        ),
    ] {
        assert_eq!(result, Err(AppError::Unauthenticated), "{what}");
    }

    // An endpoint nobody made answers exactly like a bad signature: its existence is not a secret to
    // find out by asking.
    assert_eq!(
        w.events
            .receive(
                sagat_domain::events::EndpointId::random(),
                &good.request(Some("invoice.paid"))
            )
            .await,
        Err(AppError::Unauthenticated)
    );
    assert_eq!(inbox_count(&w).await, 0, "nothing of it was admitted");
}

#[tokio::test]
async fn refusals_are_audited_once_per_endpoint_and_reason() {
    let (w, endpoint) = with_endpoint().await;
    let stale = Signed::typed(SECRET, now_seconds() - 7200, "evt-1", "invoice.paid", "{}");
    for _ in 0..5 {
        let _ = w
            .events
            .receive(endpoint.id, &stale.request(Some("invoice.paid")))
            .await;
    }
    let forged = Signed::typed(
        "not-the-secret-not-the-secret-000000",
        now_seconds(),
        "evt-2",
        "invoice.paid",
        "{}",
    );
    for _ in 0..5 {
        let _ = w
            .events
            .receive(endpoint.id, &forged.request(Some("invoice.paid")))
            .await;
    }
    // The audit write is off the response path: wait for it to land.
    let mut rows: Vec<(String, Option<String>)> = Vec::new();
    for _ in 0..50 {
        rows = sqlx::query_as(
            "SELECT resource_id, reason FROM audit_records WHERE action = 'event.rejected' \
             ORDER BY resource_id",
        )
        .fetch_all(&w.db.pool)
        .await
        .expect("audit");
        if rows.len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        rows.len(),
        2,
        "one per reason, not one per request: {rows:?}"
    );
    let reasons: Vec<_> = rows
        .iter()
        .map(|(id, _)| id.rsplit('/').next().unwrap_or(""))
        .collect();
    assert!(reasons.contains(&"stale_timestamp") && reasons.contains(&"bad_signature"));
    assert!(
        rows.iter().all(|(_, reason)| {
            let reason = reason.as_deref().unwrap_or("");
            !reason.contains(SECRET) && !reason.contains("v1=")
        }),
        "the trail never carries a signature or a secret"
    );
}

#[tokio::test]
async fn a_rotated_secret_keeps_working_for_the_previous_versions_and_not_for_older_ones() {
    let (w, endpoint) = with_endpoint().await;
    let old = SECRET;
    w.put_secret(SECRET_NAME, "second-secret-second-secret-second-0")
        .await;
    w.put_secret(SECRET_NAME, "third-secret-third-secret-third-0000")
        .await;

    // The provider has not moved yet: the first version still verifies...
    let with_old = Signed::typed(old, now_seconds(), "evt-old", "a.b", "{}");
    assert!(
        w.events
            .receive(endpoint.id, &with_old.request(Some("a.b")))
            .await
            .is_ok()
    );
    // ...and so does the newest.
    let with_new = Signed::typed(
        "third-secret-third-secret-third-0000",
        now_seconds(),
        "evt-new",
        "a.b",
        "{}",
    );
    assert!(
        w.events
            .receive(endpoint.id, &with_new.request(Some("a.b")))
            .await
            .is_ok()
    );

    // Two more rotations push the first out of the window of versions that are tried.
    w.put_secret(SECRET_NAME, "fourth-secret-fourth-secret-fourth-0")
        .await;
    w.put_secret(SECRET_NAME, "fifth-secret-fifth-secret-fifth-0000")
        .await;
    let too_old = Signed::typed(old, now_seconds(), "evt-too-old", "a.b", "{}");
    assert_eq!(
        w.events
            .receive(endpoint.id, &too_old.request(Some("a.b")))
            .await,
        Err(AppError::Unauthenticated)
    );
}

#[tokio::test]
async fn an_event_id_reused_for_another_payload_is_a_conflict_and_the_first_is_kept() {
    let (w, endpoint) = with_endpoint().await;
    let first = Signed::typed(
        SECRET,
        now_seconds(),
        "evt-1",
        "invoice.paid",
        r#"{"amount":10}"#,
    );
    w.events
        .receive(endpoint.id, &first.request(Some("invoice.paid")))
        .await
        .expect("admitted");

    let other = Signed::typed(
        SECRET,
        now_seconds(),
        "evt-1",
        "invoice.paid",
        r#"{"amount":999}"#,
    );
    assert_eq!(
        w.events
            .receive(endpoint.id, &other.request(Some("invoice.paid")))
            .await,
        Err(AppError::Conflict(ConflictKind::EventIdReused))
    );
    // The same payload under another type is a different event too (signed as such: the type is
    // covered, so the first signature cannot be carried over to it).
    let voided = Signed::typed(
        SECRET,
        now_seconds(),
        "evt-1",
        "invoice.voided",
        r#"{"amount":10}"#,
    );
    assert_eq!(
        w.events
            .receive(endpoint.id, &voided.request(Some("invoice.voided")))
            .await,
        Err(AppError::Conflict(ConflictKind::EventIdReused))
    );
    assert_eq!(
        w.events
            .receive(endpoint.id, &first.request(Some("invoice.voided")))
            .await,
        Err(AppError::Unauthenticated),
        "and the signature of the first does not vouch for it"
    );
    let kept: Vec<u8> =
        sqlx::query_scalar("SELECT payload FROM inbox_events WHERE event_id = 'evt-1'")
            .fetch_one(&w.db.pool)
            .await
            .expect("payload");
    assert_eq!(
        kept, br#"{"amount":10}"#,
        "the first payload was not replaced"
    );
    assert_eq!(inbox_count(&w).await, 1);
}

#[tokio::test]
async fn the_deduplication_window_ends_and_the_same_id_is_a_new_event() {
    let (w, endpoint) = with_endpoint().await;
    let signed = Signed::typed(SECRET, now_seconds(), "evt-1", "a.b", r#"{"n":1}"#);
    w.events
        .receive(endpoint.id, &signed.request(Some("a.b")))
        .await
        .expect("admitted");
    // Dispatched, and then the window ends. (An event still waiting to be dispatched keeps its id
    // for as long as it waits: see the dispatch suite.)
    sqlx::query(
        "UPDATE inbox_events SET status = 'DISPATCHED', dispatched_at = clock_timestamp(), \
             expires_at = clock_timestamp() - interval '1 second'",
    )
    .execute(&w.db.pool)
    .await
    .expect("age");

    let again = Signed::typed(SECRET, now_seconds(), "evt-1", "a.b", r#"{"n":2}"#);
    let result = w
        .events
        .receive(endpoint.id, &again.request(Some("a.b")))
        .await
        .expect("admitted again");
    assert!(
        !result.duplicate,
        "outside the window it is new, with the new payload"
    );
    let payload: Vec<u8> =
        sqlx::query_scalar("SELECT payload FROM inbox_events WHERE event_id = 'evt-1'")
            .fetch_one(&w.db.pool)
            .await
            .expect("payload");
    assert_eq!(payload, br#"{"n":2}"#);
}

#[tokio::test]
async fn a_paused_endpoint_refuses_and_a_resumed_one_takes_requests_again() {
    let (w, endpoint) = with_endpoint().await;
    let mut paused = settings();
    paused.status = EndpointStatus::Paused;
    w.events
        .update_endpoint(&w.admin, endpoint.id, paused, &ctx())
        .await
        .expect("pause");

    let signed = Signed::typed(SECRET, now_seconds(), "evt-1", "a.b", "{}");
    assert_eq!(
        w.events
            .receive(endpoint.id, &signed.request(Some("a.b")))
            .await,
        Err(AppError::Unavailable),
        "the provider is told to retry"
    );
    assert_eq!(inbox_count(&w).await, 0);

    w.events
        .update_endpoint(&w.admin, endpoint.id, settings(), &ctx())
        .await
        .expect("resume");
    assert!(
        w.events
            .receive(endpoint.id, &signed.request(Some("a.b")))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn a_body_over_the_endpoint_limit_is_refused_before_it_is_verified() {
    let (w, endpoint) = with_endpoint().await;
    let mut small = settings();
    small.limits = EndpointLimits::new(
        sagat_domain::events::DEFAULT_DEDUPE_TTL,
        sagat_domain::events::DEFAULT_SKEW,
        1024,
        50,
    )
    .expect("limits");
    w.events
        .update_endpoint(&w.admin, endpoint.id, small, &ctx())
        .await
        .expect("limit");

    let big = format!(r#"{{"pad":"{}"}}"#, "x".repeat(2000));
    let signed = Signed::typed(SECRET, now_seconds(), "evt-1", "a.b", &big);
    assert_eq!(
        w.events
            .receive(endpoint.id, &signed.request(Some("a.b")))
            .await,
        Err(AppError::PayloadTooLarge("webhook body"))
    );
    assert_eq!(inbox_count(&w).await, 0);
}

#[tokio::test]
async fn a_verified_body_that_is_not_a_json_object_is_refused_and_not_stored() {
    let (w, endpoint) = with_endpoint().await;
    for (id, body) in [
        ("e1", "[1,2]"),
        ("e2", "\"text\""),
        ("e3", "not json"),
        ("e4", "null"),
    ] {
        let signed = Signed::typed(SECRET, now_seconds(), id, "a.b", body);
        assert_eq!(
            w.events
                .receive(endpoint.id, &signed.request(Some("a.b")))
                .await,
            Err(AppError::NotAllowed("the body must be a JSON object")),
            "{body}"
        );
    }
    assert_eq!(inbox_count(&w).await, 0);
}

#[tokio::test]
async fn a_request_without_a_type_uses_the_endpoint_default_and_without_either_is_refused() {
    let (w, endpoint) = with_endpoint().await;
    let signed = Signed::typed(SECRET, now_seconds(), "evt-1", "", "{}");
    assert_eq!(
        w.events.receive(endpoint.id, &signed.request(None)).await,
        Err(AppError::Unauthenticated),
        "an event without a type is not one"
    );

    let mut with_default = settings();
    with_default.default_event_type = Some(crate::event_type("invoice.paid"));
    w.events
        .update_endpoint(&w.admin, endpoint.id, with_default, &ctx())
        .await
        .expect("default");
    assert!(
        w.events
            .receive(endpoint.id, &signed.request(None))
            .await
            .is_ok()
    );
    let kind: String = sqlx::query_scalar("SELECT event_type FROM inbox_events")
        .fetch_one(&w.db.pool)
        .await
        .expect("type");
    assert_eq!(kind, "invoice.paid");
}

#[tokio::test]
async fn an_endpoint_takes_its_rate_and_asks_the_sender_to_wait_beyond_it() {
    let (w, endpoint) = with_endpoint().await;
    let mut slow = settings();
    slow.limits = EndpointLimits::new(
        sagat_domain::events::DEFAULT_DEDUPE_TTL,
        sagat_domain::events::DEFAULT_SKEW,
        1024,
        1,
    )
    .expect("limits");
    w.events
        .update_endpoint(&w.admin, endpoint.id, slow, &ctx())
        .await
        .expect("rate");

    let mut accepted = 0;
    let mut busy = 0;
    for i in 0..6 {
        let signed = Signed::typed(SECRET, now_seconds(), &format!("evt-{i}"), "a.b", "{}");
        match w
            .events
            .receive(endpoint.id, &signed.request(Some("a.b")))
            .await
        {
            Ok(_) => accepted += 1,
            Err(AppError::Busy { retry_after_secs }) => {
                assert!(
                    retry_after_secs >= 1,
                    "never an invitation to retry at once"
                );
                busy += 1;
            }
            Err(other) => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(accepted, 2, "the burst of one second's worth, twice");
    assert_eq!(busy, 4);
    assert_eq!(
        inbox_count(&w).await,
        2,
        "a refused request was not admitted"
    );
}

#[tokio::test]
async fn a_full_backlog_refuses_new_events_and_still_answers_duplicates() {
    let (w, endpoint) = with_endpoint().await;
    assert_eq!(BACKLOG_LIMIT, 10_000, "the limit the service applies");
    let event = |id: &str| {
        let key = EventKey {
            source: slug("billing"),
            event_id: EventId::parse(id).expect("id"),
        };
        NewEvent {
            trace: None,
            tenant_id: endpoint.tenant_id,
            namespace_id: endpoint.namespace_id,
            envelope: EventEnvelope::new(key, EventType::parse("a.b").expect("type"), None, None)
                .expect("envelope"),
            occurred_at: None,
            payload: b"{}".to_vec(),
            digest: vec![id.len() as u8; 32],
            dedupe_ttl: std::time::Duration::from_secs(3600),
        }
    };
    assert_eq!(
        w.store.admit_event(&event("e1"), 2, None).await,
        Ok(Admission::Accepted)
    );
    assert_eq!(
        w.store.admit_event(&event("e2"), 2, None).await,
        Ok(Admission::Accepted)
    );
    assert_eq!(
        w.store.admit_event(&event("e3"), 2, None).await,
        Ok(Admission::Backlogged),
        "the third does not fit"
    );
    assert_eq!(
        w.store.admit_event(&event("e1"), 2, None).await,
        Ok(Admission::Duplicate),
        "but a duplicate costs nothing and is still answered"
    );
    assert_eq!(inbox_count(&w).await, 2);

    // Once the matcher has taken events off the backlog, there is room again.
    sqlx::query("UPDATE inbox_events SET status = 'DISPATCHED', dispatched_at = clock_timestamp() WHERE event_id = 'e1'")
        .execute(&w.db.pool)
        .await
        .expect("dispatch");
    assert_eq!(
        w.store.admit_event(&event("e3"), 2, None).await,
        Ok(Admission::Accepted)
    );
}

#[tokio::test]
async fn the_same_source_and_event_id_in_two_namespaces_are_two_events() {
    let (w, endpoint) = with_endpoint().await;
    let other = w.add_namespace("billing-eu").await;
    w.put_secret_in(other.id(), SECRET_NAME, SECRET).await;
    let second = w
        .events
        .create_endpoint(&w.admin, other.id(), slug("billing"), settings(), &ctx())
        .await
        .expect("second endpoint, same slug in another namespace");

    let signed = Signed::typed(SECRET, now_seconds(), "evt-1", "a.b", "{}");
    let a = w
        .events
        .receive(endpoint.id, &signed.request(Some("a.b")))
        .await
        .expect("a");
    let b = w
        .events
        .receive(second.id, &signed.request(Some("a.b")))
        .await
        .expect("b");
    assert!(
        !a.duplicate && !b.duplicate,
        "one namespace's event is not another's duplicate"
    );
    assert_eq!(inbox_count(&w).await, 2);
}

#[tokio::test]
async fn publishing_through_the_api_dedupes_by_event_id_and_needs_the_permission() {
    let w = support::engine::world().await;
    let request = |id: &str, data: &str| PublishRequest {
        namespace: "payments".to_owned(),
        event_id: Some(EventId::parse(id).expect("id")),
        event_type: EventType::parse("order.created").expect("type"),
        subject: Some("orders/7".to_owned()),
        correlation_key: None,
        occurred_at: None,
        data: data.as_bytes().to_vec(),
        dedupe_ttl: None,
    };
    let first = w
        .events
        .publish(&w.admin, request("o-1", r#"{"n":1}"#), &ctx())
        .await
        .expect("published");
    assert!(!first.duplicate);
    assert_eq!(first.key.source.as_str(), "api");
    let again = w
        .events
        .publish(&w.admin, request("o-1", r#"{"n":1}"#), &ctx())
        .await
        .expect("again");
    assert!(again.duplicate);
    assert_eq!(
        w.events
            .publish(&w.admin, request("o-1", r#"{"n":2}"#), &ctx())
            .await,
        Err(AppError::Conflict(ConflictKind::EventIdReused))
    );
    assert_eq!(
        w.events
            .publish(&w.admin, request("o-2", "[1]"), &ctx())
            .await,
        Err(AppError::NotAllowed("the event data must be a JSON object"))
    );

    // A role without `event:publish` cannot, and one in another tenant cannot name this namespace.
    let viewer = w.member("viewer-1", Role::Viewer).await;
    assert!(matches!(
        w.events
            .publish(&viewer, request("o-3", "{}"), &ctx())
            .await,
        Err(AppError::Forbidden { .. })
    ));
    assert_eq!(
        w.events
            .publish(&w.other_admin, request("o-4", "{}"), &ctx())
            .await
            .map(|r| r.duplicate),
        Ok(false),
        "the other tenant has its own namespace called payments: its event lands there"
    );
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT t.slug FROM inbox_events e JOIN tenants t ON t.id = e.tenant_id ORDER BY e.event_id",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("rows");
    assert_eq!(rows.len(), 2, "o-1 (once) and o-4, and no more");

    // Who published what is in the trail, once per event that was admitted: not for the repeat, not
    // for the refusals. (Whatever a published event starts is attributed to the engine, and the
    // inbox row is purged with its window: this is where the principal stays.)
    let published: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT actor, resource_type, resource_id FROM audit_records \
         WHERE action = 'event.published' ORDER BY resource_id",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    let ids: Vec<&str> = published.iter().map(|(_, _, id)| id.as_str()).collect();
    assert_eq!(
        ids,
        ["api/o-1", "api/o-4"],
        "once each, however often o-1 was published, and not for the refusals"
    );
    assert!(
        published
            .iter()
            .all(|(actor, kind, _)| !actor.is_empty() && kind == "event"),
        "{published:?}"
    );
    assert_eq!(audit_count(&w, "event.published").await, 2, "o-1 and o-4");
}

#[tokio::test]
async fn endpoints_are_scoped_to_their_tenant_and_to_a_namespace_quota() {
    let (w, endpoint) = with_endpoint().await;

    // Another tenant cannot see, change or delete it: for them it does not exist.
    assert_eq!(
        w.events.get_endpoint(&w.other_admin, endpoint.id).await,
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.events
            .update_endpoint(&w.other_admin, endpoint.id, settings(), &ctx())
            .await,
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.events
            .delete_endpoint(&w.other_admin, endpoint.id, &ctx())
            .await,
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.events
            .list_endpoints(&w.other_admin, w.namespace.id(), &ctx())
            .await
            .map(|v| v.len()),
        Err(AppError::NotFound),
        "nor list the endpoints of a namespace that is not theirs"
    );
    assert!(
        w.events.get_endpoint(&w.admin, endpoint.id).await.is_ok(),
        "the owner still can"
    );

    // The quota counts endpoints per namespace.
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            sagat_domain::quota::QuotaResource::EventEndpoints,
            1,
            &ctx(),
        )
        .await
        .expect("limit");
    assert_eq!(
        w.events
            .create_endpoint(
                &w.admin,
                w.namespace.id(),
                slug("shipping"),
                settings(),
                &ctx()
            )
            .await,
        Err(AppError::QuotaExceeded(
            "webhook endpoints in the namespace"
        ))
    );
}

#[tokio::test]
async fn an_endpoint_needs_a_signing_secret_its_creator_may_use_and_not_the_reserved_slug() {
    let w = support::engine::world().await;
    // No such secret.
    let missing = w
        .events
        .create_endpoint(
            &w.admin,
            w.namespace.id(),
            slug("billing"),
            settings(),
            &ctx(),
        )
        .await;
    assert!(
        matches!(missing, Err(AppError::SecretUnavailable { .. })),
        "{missing:?}"
    );

    w.put_secret(SECRET_NAME, SECRET).await;
    // The source of API events, and of schedules, are not an endpoint's to take: a workflow that
    // trusts `event.source == "cron"` must be able to.
    for reserved in ["api", "cron"] {
        assert!(
            matches!(
                w.events
                    .create_endpoint(
                        &w.admin,
                        w.namespace.id(),
                        slug(reserved),
                        settings(),
                        &ctx()
                    )
                    .await,
                Err(AppError::NotAllowed(_))
            ),
            "{reserved}"
        );
    }
    // A developer may use a secret only when it is on the list.
    let dev = w.member("dev-1", Role::Developer).await;
    assert!(matches!(
        w.events
            .create_endpoint(&dev, w.namespace.id(), slug("billing"), settings(), &ctx())
            .await,
        Err(AppError::Forbidden { .. } | AppError::SecretUnavailable { .. })
    ));
    // A taken slug is a conflict.
    w.events
        .create_endpoint(
            &w.admin,
            w.namespace.id(),
            slug("billing"),
            settings(),
            &ctx(),
        )
        .await
        .expect("first");
    assert_eq!(
        w.events
            .create_endpoint(
                &w.admin,
                w.namespace.id(),
                slug("billing"),
                settings(),
                &ctx()
            )
            .await
            .map(|_| ()),
        Err(AppError::Conflict(ConflictKind::EventEndpointSlug))
    );
    assert_eq!(
        audit_count(&w, "event.endpoint_created").await,
        1,
        "and only the one that exists is audited"
    );
}

#[tokio::test]
async fn a_database_failure_while_verifying_is_never_a_verdict_on_the_request() {
    let (w, endpoint) = with_endpoint().await;
    let signed = Signed::typed(SECRET, now_seconds(), "evt-1", "a.b", "{}");
    // The secret cannot be read: the table it lives in is not there.
    sqlx::query("ALTER TABLE secret_versions RENAME TO secret_versions_away")
        .execute(&w.db.pool)
        .await
        .expect("break the store");
    let result = w
        .events
        .receive(endpoint.id, &signed.request(Some("a.b")))
        .await;
    assert!(
        matches!(result, Err(AppError::Unavailable | AppError::Internal)),
        "a correct request must not be told it is unauthenticated because a table is down: {result:?}"
    );
    assert_eq!(inbox_count(&w).await, 0);

    // And once the store is back the same request is accepted: nothing was remembered against it.
    sqlx::query("ALTER TABLE secret_versions_away RENAME TO secret_versions")
        .execute(&w.db.pool)
        .await
        .expect("repair");
    assert!(
        w.events
            .receive(endpoint.id, &signed.request(Some("a.b")))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn what_the_sender_says_the_event_is_is_covered_by_the_signature() {
    let (w, endpoint) = with_endpoint().await;
    let now = now_seconds();
    let body = r#"{"amount":10}"#;
    let key = "ord-7";
    let signature = crate::support::signed::inbound_with(
        SECRET,
        now,
        "evt-1",
        "invoice.paid",
        "subject/1",
        key,
        body.as_bytes(),
    );
    let timestamp = now.to_string();
    let request = |event_type: Option<&'static str>,
                   subject: Option<&'static str>,
                   correlation_key: Option<&'static str>| WebhookRequest {
        trace: None,
        signature: Some(&signature),
        timestamp: Some(&timestamp),
        event_id: Some("evt-1"),
        event_type,
        subject,
        correlation_key,
        body: body.as_bytes(),
    };

    // A header changed on the way (by a proxy, or by whoever saw the request) is a bad signature.
    for (what, tampered) in [
        (
            "another event type",
            request(Some("invoice.voided"), Some("subject/1"), Some(key)),
        ),
        ("no event type", request(None, Some("subject/1"), Some(key))),
        (
            "another subject",
            request(Some("invoice.paid"), Some("subject/2"), Some(key)),
        ),
        (
            "another correlation key",
            request(Some("invoice.paid"), Some("subject/1"), Some("ord-8")),
        ),
        (
            "no correlation key",
            request(Some("invoice.paid"), Some("subject/1"), None),
        ),
    ] {
        assert_eq!(
            w.events.receive(endpoint.id, &tampered).await,
            Err(AppError::Unauthenticated),
            "{what}"
        );
    }
    assert_eq!(inbox_count(&w).await, 0, "none of it was admitted");

    // What was signed is what is admitted, with exactly that envelope.
    assert!(
        w.events
            .receive(
                endpoint.id,
                &request(Some("invoice.paid"), Some("subject/1"), Some(key))
            )
            .await
            .is_ok()
    );
    let row: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT event_type, subject, correlation_key FROM inbox_events WHERE event_id = 'evt-1'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("row");
    assert_eq!(
        row,
        (
            "invoice.paid".to_owned(),
            Some("subject/1".to_owned()),
            Some(key.to_owned())
        )
    );
}

#[tokio::test]
async fn a_request_that_cannot_be_genuine_is_refused_before_any_secret_is_read() {
    let (w, endpoint) = with_endpoint().await;
    let now = now_seconds();
    let plausible = Signed::typed(SECRET, now, "evt-1", "a.b", "{}");
    // Reading a secret now fails. Anything refused alike still being a refusal (and not a failure of
    // the store) shows that the store was not asked.
    sqlx::query("ALTER TABLE secret_versions RENAME TO secret_versions_away")
        .execute(&w.db.pool)
        .await
        .expect("break the store");

    let stale = Signed::typed(SECRET, now - 3600, "evt-1", "a.b", "{}");
    let one = format!("v1={}", "0".repeat(64));
    let crowded = vec![one.as_str(); 6_000].join(",");
    let few_too_many = [one.as_str(); 5].join(",");
    let timestamp = now.to_string();
    let with_header = |header: &'static str| {
        receive_with_header(&w, endpoint.id, header.to_owned(), timestamp.clone())
    };
    for (what, result) in [
        (
            "a timestamp an hour old",
            w.events
                .receive(endpoint.id, &stale.request(Some("a.b")))
                .await,
        ),
        (
            "six thousand signatures",
            receive_with_header(&w, endpoint.id, crowded, timestamp.clone()).await,
        ),
        (
            "five signatures",
            receive_with_header(&w, endpoint.id, few_too_many, timestamp.clone()).await,
        ),
        ("not a signature at all", with_header("garbage").await),
    ] {
        assert_eq!(result, Err(AppError::Unauthenticated), "{what}");
    }

    // A request that could be genuine does need the secret, and the failure to read it is the
    // platform's, not the sender's.
    let result = w
        .events
        .receive(endpoint.id, &plausible.request(Some("a.b")))
        .await;
    assert!(
        matches!(result, Err(AppError::Unavailable | AppError::Internal)),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_secret_too_short_to_mean_anything_verifies_nothing() {
    let w = support::engine::world().await;
    w.put_secret(SECRET_NAME, "short").await;
    let endpoint = w
        .events
        .create_endpoint(
            &w.admin,
            w.namespace.id(),
            slug("billing"),
            settings(),
            &ctx(),
        )
        .await
        .expect("an endpoint can be made; the secret is judged when it is used");
    // A signature made with the very same key, the way the scheme says.
    let now = now_seconds();
    let body = "{}";
    let forged = {
        use hmac::{Hmac, KeyInit, Mac};
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"short").expect("any length");
        mac.update(format!("octopus-event-in-v1\n{now}\nevt-1\na.b\n\n\n{body}").as_bytes());
        let digest: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        format!("v1={digest}")
    };
    let timestamp = now.to_string();
    let request = WebhookRequest {
        trace: None,
        signature: Some(&forged),
        timestamp: Some(&timestamp),
        event_id: Some("evt-1"),
        event_type: Some("a.b"),
        subject: None,
        correlation_key: None,
        body: body.as_bytes(),
    };
    assert_eq!(
        w.events.receive(endpoint.id, &request).await,
        Err(AppError::Unauthenticated)
    );
    assert_eq!(inbox_count(&w).await, 0);
}

/// A request whose signature header is `header`, whatever it is, at `timestamp`.
async fn receive_with_header(
    w: &crate::World,
    endpoint: sagat_domain::events::EndpointId,
    header: String,
    timestamp: String,
) -> Result<sagat_application::events::Received, AppError> {
    let request = WebhookRequest {
        trace: None,
        signature: Some(&header),
        timestamp: Some(&timestamp),
        event_id: Some("evt-1"),
        event_type: Some("a.b"),
        subject: None,
        correlation_key: None,
        body: b"{}",
    };
    w.events.receive(endpoint, &request).await
}

#[tokio::test]
async fn an_event_published_through_the_api_may_have_a_short_window_and_not_a_silly_one() {
    use sagat_application::events::PublishRequest;
    let w = support::engine::world().await;
    let publish = |id: &str, seconds: u64| {
        let request = PublishRequest {
            namespace: "payments".to_owned(),
            event_id: Some(sagat_domain::events::EventId::parse(id).expect("id")),
            event_type: crate::event_type("a.b"),
            subject: None,
            correlation_key: None,
            occurred_at: None,
            data: b"{}".to_vec(),
            dedupe_ttl: Some(std::time::Duration::from_secs(seconds)),
        };
        let w = &w;
        async move { w.events.publish(&w.admin, request, &ctx()).await }
    };
    // Nothing is signed, so there is no skew for the window to outlast: a minute is a window.
    assert!(publish("short", 60).await.is_ok());
    // Below a minute it is not a window, and above a month it is a table to fill.
    assert!(matches!(
        publish("shorter", 59).await,
        Err(AppError::NotAllowed(_))
    ));
    assert!(matches!(
        publish("longer", 31 * 86_400).await,
        Err(AppError::NotAllowed(_))
    ));
}

#[tokio::test]
async fn an_event_that_was_never_dispatched_keeps_its_id_after_its_window_and_is_not_made_new() {
    let (w, endpoint) = with_endpoint().await;
    let first = Signed::typed(SECRET, now_seconds(), "evt-1", "a.b", r#"{"n":1}"#);
    w.events
        .receive(endpoint.id, &first.request(Some("a.b")))
        .await
        .expect("admitted");
    // The window ends while the matcher has not got to it: the event is still owed to its triggers.
    sqlx::query("UPDATE inbox_events SET expires_at = clock_timestamp() - interval '1 second'")
        .execute(&w.db.pool)
        .await
        .expect("age");

    let again = w
        .events
        .receive(endpoint.id, &first.request(Some("a.b")))
        .await
        .expect("answered");
    assert!(
        again.duplicate,
        "it is still the same event: it was never dispatched, so it has not yet done what it is for"
    );
    assert_eq!(inbox_count(&w).await, 1, "and nothing was replaced");
}

#[tokio::test]
async fn a_flood_of_requests_that_prove_nothing_is_turned_away_before_anything_is_read() {
    let (w, endpoint) = with_endpoint().await;
    // A modest endpoint: the bound on what an unproven caller may spend is a multiple of its rate.
    let mut modest = settings();
    modest.limits = EndpointLimits::new(
        sagat_domain::events::DEFAULT_DEDUPE_TTL,
        sagat_domain::events::DEFAULT_SKEW,
        1024,
        1,
    )
    .expect("limits");
    w.events
        .update_endpoint(&w.admin, endpoint.id, modest, &ctx())
        .await
        .expect("rate");

    // Correctly formed, signed with the wrong secret.
    let mut refused = 0;
    let mut turned_away = 0;
    for i in 0..80 {
        let junk = Signed::typed(
            "not-the-secret-not-the-secret-000000",
            now_seconds(),
            &format!("junk-{i}"),
            "a.b",
            "{}",
        );
        match w
            .events
            .receive(endpoint.id, &junk.request(Some("a.b")))
            .await
        {
            Err(AppError::Unauthenticated) => refused += 1,
            Err(AppError::Busy { .. }) => turned_away += 1,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(
        refused >= 15,
        "the first requests are looked at and refused: {refused}"
    );
    assert!(
        turned_away >= 40,
        "and the flood after them is told to wait, without being looked at: {turned_away}"
    );
    assert_eq!(inbox_count(&w).await, 0);
}
