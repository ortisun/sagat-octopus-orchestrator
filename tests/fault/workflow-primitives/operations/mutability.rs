//! The state rules of an Update over public HTTP and PostgreSQL: only a RUNNING or WAITING run
//! takes a new one. In every other status the answer is a `409 CONFLICT` with the reason
//! `RUN_NOT_MUTABLE`, and the refusal writes nothing, while a Query still reads the last snapshot.

use reqwest::StatusCode;
use sagat_domain::execution::RunStatus;
use serde_json::json;

use super::{Fixture, fixture, query_path, update_body, update_path};
use crate::support::api::json_of;

/// What a refused Update must not change.
#[derive(Debug, PartialEq, Eq)]
struct Footprint {
    run_version: i64,
    state_revision: i64,
    receipts: i64,
    journal: i64,
    state_events: i64,
    outbox: i64,
    update_audits: i64,
}

async fn footprint(fixture: &Fixture) -> Footprint {
    let row: (i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT r.run_version, s.state_revision, \
                (SELECT count(*) FROM operation_update_receipts x \
                  WHERE x.tenant_id=r.tenant_id AND x.resolved_run_id=r.id), \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id), \
                (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id \
                    AND e.event_type='state.updated'), \
                (SELECT count(*) FROM event_outbox o \
                  WHERE o.tenant_id=r.tenant_id AND o.aggregate_id=r.id), \
                (SELECT count(*) FROM audit_records a WHERE a.tenant_id=r.tenant_id \
                    AND a.resource_id=r.id::text AND a.action='run.updated') \
         FROM runs r JOIN workflow_operation_states s \
           ON s.tenant_id=r.tenant_id AND s.run_id=r.id \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .fetch_one(&fixture.api.db.pool)
    .await
    .expect("run footprint");
    Footprint {
        run_version: row.0,
        state_revision: row.1,
        receipts: row.2,
        journal: row.3,
        state_events: row.4,
        outbox: row.5,
        update_audits: row.6,
    }
}

async fn set_status(fixture: &Fixture, status: RunStatus) {
    sqlx::query(
        "UPDATE runs SET status=$3, \
                finished_at = CASE WHEN $4 THEN now() ELSE NULL END, updated_at=now() \
         WHERE tenant_id=$1 AND id=$2",
    )
    .bind(fixture.tenant.as_uuid())
    .bind(fixture.run.as_uuid())
    .bind(status.as_str())
    .bind(status.is_terminal())
    .execute(&fixture.api.db.pool)
    .await
    .unwrap_or_else(|error| panic!("set status {status}: {error}"));
}

#[tokio::test]
async fn every_status_but_running_and_waiting_refuses_an_update_and_still_serves_the_query() {
    let fixture = fixture("operations-not-mutable").await;
    let mut refused = 0;
    for status in RunStatus::ALL {
        if matches!(status, RunStatus::Running | RunStatus::Waiting) {
            continue;
        }
        set_status(&fixture, status).await;
        let before = footprint(&fixture).await;

        let response = fixture
            .api
            .post_as(
                &update_path(&fixture),
                "alice",
                "acme",
                &update_body(&format!("refused-{status}"), 77),
            )
            .await;
        assert_eq!(response.status(), StatusCode::CONFLICT, "{status}");
        let body = json_of(response).await;
        assert_eq!(body["code"], "CONFLICT", "{status}");
        assert_eq!(body["details"]["reason"], "RUN_NOT_MUTABLE", "{status}");
        assert_eq!(
            footprint(&fixture).await,
            before,
            "{status}: a refusal wrote"
        );

        // The refusal is not remembered as a receipt: nothing replays it, and the same key is
        // refused again for the same reason, not answered from a stored rejection.
        let again = fixture
            .api
            .post_as(
                &update_path(&fixture),
                "alice",
                "acme",
                &update_body(&format!("refused-{status}"), 77),
            )
            .await;
        assert_eq!(again.status(), StatusCode::CONFLICT, "{status}");
        assert_eq!(
            json_of(again).await["details"]["reason"],
            "RUN_NOT_MUTABLE",
            "{status}"
        );

        // A Query reads the last committed snapshot in every status.
        let query = fixture
            .api
            .post_as(&query_path(&fixture), "alice", "acme", &json!({"args": 3}))
            .await;
        assert_eq!(query.status(), StatusCode::OK, "{status}");
        let query = json_of(query).await;
        assert_eq!(query["result"], 13, "{status}");
        assert_eq!(query["stateRevision"], 0, "{status}");
        assert_eq!(
            query["runVersion"].as_i64(),
            Some(before.run_version),
            "{status}"
        );
        refused += 1;
    }
    assert_eq!(
        refused,
        RunStatus::ALL.len() - 2,
        "every other status was tried"
    );

    // Mutability is read from the run, not remembered: back to WAITING, the same refused key now
    // applies once, and survives the run finishing as a replay.
    set_status(&fixture, RunStatus::Waiting).await;
    let body = update_body("refused-PAUSED", 77);
    let applied = fixture
        .api
        .post_as(&update_path(&fixture), "alice", "acme", &body)
        .await;
    assert_eq!(applied.status(), StatusCode::OK);
    let applied = json_of(applied).await;
    assert_eq!(applied["status"], "APPLIED");
    assert_eq!(applied["replayed"], false);
    set_status(&fixture, RunStatus::Succeeded).await;
    let replay = fixture
        .api
        .post_as(&update_path(&fixture), "alice", "acme", &body)
        .await;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay = json_of(replay).await;
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["result"], 77);
    let after = footprint(&fixture).await;
    assert_eq!(
        (after.state_revision, after.receipts, after.state_events),
        (1, 1, 1)
    );
}
