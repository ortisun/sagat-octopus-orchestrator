//! Ingress: a provider that retries what it was not told was accepted, while connections are cut,
//! ends up with every event admitted once and every run started once.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use sagat_application::error::AppError;
use sagat_application::events::WebhookRequest;
use serde_json::{Value, json};

use crate::support::engine::{activity, ctx, document, world};
use crate::support::slug;

const SECRET: &str = "whsec-0123456789abcdef0123456789abcdef";

#[tokio::test]
async fn connections_killed_under_load_lose_no_accepted_event_and_admit_none_twice() {
    let w = Arc::new(world().await);
    w.put_secret("provider-signing", SECRET).await;
    let endpoint = w
        .events
        .create_endpoint(
            &w.admin,
            w.namespace.id(),
            slug("billing"),
            sagat_application::events::EndpointSettings {
                secret_name: Some(
                    sagat_domain::secrets::SecretName::parse("provider-signing").expect("secret"),
                ),
                authentication: sagat_domain::events::WebhookAuthentication::Hmac,
                default_event_type: None,
                limits: sagat_domain::events::EndpointLimits::new(
                    sagat_domain::events::DEFAULT_DEDUPE_TTL,
                    sagat_domain::events::DEFAULT_SKEW,
                    4096,
                    10_000,
                )
                .expect("limits"),
                status: sagat_domain::events::EndpointStatus::Active,
            },
            &ctx(),
        )
        .await
        .expect("endpoint");
    let mut ir = document("on-invoice", "work", vec![activity("work")]);
    ir["triggers"] = json!([{"kind": "webhook", "endpoint": "billing"}]);
    w.publish(ir).await;

    // Four providers, each sending fifty events and retrying any that is not answered with an
    // acceptance, while backends are terminated over and over.
    let accepted = Arc::new(AtomicUsize::new(0));
    let storm = {
        let pool = w.db.pool.clone();
        tokio::spawn(async move {
            let until = std::time::Instant::now() + Duration::from_secs(3);
            while std::time::Instant::now() < until {
                let _ = sqlx::query(
                    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                     WHERE datname = current_database() AND pid <> pg_backend_pid()",
                )
                .execute(&pool)
                .await;
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        })
    };
    let providers: Vec<_> = (0..4)
        .map(|p| {
            let w = Arc::clone(&w);
            let accepted = Arc::clone(&accepted);
            tokio::spawn(async move {
                for i in 0..50 {
                    let id = format!("evt-{p}-{i}");
                    let body = format!(r#"{{"provider":{p},"n":{i}}}"#);
                    let mut tries = 0;
                    loop {
                        let timestamp = time::OffsetDateTime::now_utc().unix_timestamp();
                        let signature = crate::support::signed::inbound(
                            SECRET,
                            timestamp,
                            &id,
                            "invoice.paid",
                            body.as_bytes(),
                        );
                        let ts = timestamp.to_string();
                        let request = WebhookRequest {
                            trace: None,
                            signature: Some(&signature),
                            bearer: None,
                            certificate_sha256: None,
                            timestamp: Some(&ts),
                            event_id: Some(&id),
                            event_type: Some("invoice.paid"),
                            subject: None,
                            correlation_key: None,
                            body: body.as_bytes(),
                        };
                        match w.events.receive(endpoint.id, &request).await {
                            Ok(_) => {
                                accepted.fetch_add(1, Ordering::SeqCst);
                                break;
                            }
                            Err(
                                AppError::Unavailable | AppError::Internal | AppError::Busy { .. },
                            ) => {
                                tries += 1;
                                assert!(tries < 200, "{id} was never accepted");
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }
                            Err(other) => panic!("{id}: {other:?}"),
                        }
                    }
                }
            })
        })
        .collect();
    for provider in providers {
        provider.await.expect("provider");
    }
    storm.await.expect("storm");

    // Every acceptance is an event in the inbox, once, and then (dispatched without interference)
    // a run, once.
    let total = 4 * 50;
    assert_eq!(accepted.load(Ordering::SeqCst), total);
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT event_id FROM inbox_events ORDER BY event_id")
            .fetch_all(&w.db.pool)
            .await
            .expect("ids");
    assert_eq!(ids.len(), total, "every accepted event is in the inbox");
    assert_eq!(
        ids.iter().collect::<BTreeSet<_>>().len(),
        total,
        "none twice"
    );
    for _ in 0..20 {
        w.dispatcher.tick().await;
    }
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&w.db.pool)
        .await
        .expect("runs");
    assert_eq!(
        runs,
        i64::try_from(total).expect("small"),
        "one run per accepted event"
    );
    let _: Option<Value> = None;
}
