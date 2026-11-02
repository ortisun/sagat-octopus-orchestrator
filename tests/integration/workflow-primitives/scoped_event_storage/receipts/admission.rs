//! Owner corruption of the current row must never become a retained event value.
use super::*;

#[tokio::test]
async fn current_inbox_data_requires_its_admission_fingerprint_json_object_and_body_bound() {
    let (fixture, store, candidate, event) = source("event-current-row-integrity").await;
    for full in [false, true] {
        for case in ["fingerprint", "scalar", "invalid-json", "oversize"] {
            let mut current = event.clone();
            current.payload = match case {
                "scalar" => b"1".to_vec(),
                "invalid-json" => b"{".to_vec(),
                "oversize" => serde_json::to_vec(
                    &json!({"value":"x".repeat(sagat_domain::events::DEFAULT_MAX_BODY)}),
                )
                .expect("bounded owner corruption fixture"),
                _ => event.payload.clone(),
            };
            let digest = if case == "fingerprint" {
                vec![0_u8; 32]
            } else {
                sagat_application::events::event_admission_digest(
                    &current.envelope,
                    &current.payload,
                )
            };
            let changed = sqlx::query("UPDATE inbox_events SET payload=$5,payload_digest=$6 WHERE tenant_id=$1 AND namespace_id=$2 AND source=$3 AND event_id=$4")
                .bind(candidate.tenant_id.as_uuid()).bind(candidate.namespace_id.as_uuid()).bind(event.envelope.key.source.as_str()).bind(event.envelope.key.event_id.as_str()).bind(&current.payload).bind(digest).execute(&fixture.world.db.pool).await;
            if case == "oversize" {
                assert!(
                    matches!(&changed, Err(sqlx::Error::Database(error)) if error.constraint()==Some("inbox_events_payload_limit")),
                    "the authoritative existing database bound must refuse oversized input"
                );
                continue;
            }
            changed.expect("CHECK-valid owner-only row corruption");
            let before = guards::snapshot(&fixture).await;
            assert!(
                probe(&store, &candidate, &current, full).await.is_err(),
                "corrupt current admission accepted: {case}, full={full}"
            );
            assert_eq!(guards::snapshot(&fixture).await, before);
            sqlx::query("UPDATE inbox_events SET payload=$5,payload_digest=$6 WHERE tenant_id=$1 AND namespace_id=$2 AND source=$3 AND event_id=$4")
                .bind(candidate.tenant_id.as_uuid()).bind(candidate.namespace_id.as_uuid()).bind(event.envelope.key.source.as_str()).bind(event.envelope.key.event_id.as_str()).bind(&event.payload).bind(sagat_application::events::event_admission_digest(&event.envelope,&event.payload)).execute(&fixture.world.db.pool).await.expect("restore real admitted row");
        }
    }
    assert!(
        probe(&store, &candidate, &event, false)
            .await
            .expect("restored admission")
            .is_some()
    );
}
