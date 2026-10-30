//! Builders shared by the schedule store tests.
#![allow(clippy::expect_used)]

use sagat_application::schedule_codec::{policies_from_json, spec_from_json, target_from_json};
use sagat_application::schedules::{ScheduleOrigin, ScheduleRecord, ScheduleState};
use sagat_domain::definition::WorkflowId;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::schedules::{Ceilings, ScheduleId, ScheduleName};
use serde_json::json;
use time::OffsetDateTime;

/// A schedule at revision 1, due now, that runs a workflow of its own every weekday morning.
pub fn record(tenant: TenantId, namespace: NamespaceId, name: &str) -> ScheduleRecord {
    let ceilings = Ceilings::default();
    let now = OffsetDateTime::now_utc();
    ScheduleRecord {
        id: ScheduleId::generate(),
        tenant_id: tenant,
        namespace_id: namespace,
        name: ScheduleName::new(name).expect("name"),
        origin: ScheduleOrigin::Api,
        trigger_id: None,
        state: ScheduleState::Active,
        revision: 1,
        state_version: 1,
        spec: spec_from_json(
            &json!({"cron": ["0 9 * * 1-5"], "timezone": "Europe/Lisbon", "jitterMs": 1000}),
            &ceilings,
        )
        .expect("spec"),
        policies: policies_from_json(
            &json!({"overlap": "BUFFER_ONE", "pauseOnFailure": true}),
            &ceilings,
        )
        .expect("policies"),
        target: target_from_json(&json!({
            "workflowId": WorkflowId::generate().to_string(),
            "nominalTimeField": "scheduledFor",
            "businessIdPrefix": "report"
        }))
        .expect("target"),
        input_digest: None,
        note: None,
        note_by: None,
        note_at: None,
        authorized_by: "alice".to_owned(),
        remaining_actions: Some(10),
        next_action_at: Some(now),
        next_actual_at: Some(now),
        retry_at: None,
        recheck_at: None,
        tracked_runs: vec![],
        buffered: 0,
        created_at: now,
        updated_at: now,
    }
}
