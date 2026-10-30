//! The JSON of a schedule: how the spec, the policies and the target travel between the REST contract,
//! the database and the domain (`contracts/schedule-spec.schema.json`, `T018-016`).
#![allow(clippy::expect_used, clippy::panic)]

use sagat_domain::business_ids::{ConflictPolicy, ReusePolicy};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::schedules::{
    CatchupMode, Ceilings, DstAmbiguous, DstGap, Overlap, ScheduleError, VersionPolicy,
};
use sagat_infra_ir::schedule_codec::{
    CodecError, CodecReason, policies_from_json, policies_to_json, spec_from_json, spec_to_json,
    target_from_json, target_to_json,
};
use serde_json::{Value, json};

fn spec(value: &Value) -> Result<sagat_domain::schedules::ScheduleSpec, CodecError> {
    spec_from_json(value, &Ceilings::default())
}

fn failure(result: Result<impl std::fmt::Debug, CodecError>) -> CodecError {
    result.expect_err("the document should be refused")
}

// --- the spec -----------------------------------------------------------------------------------

fn rich() -> Value {
    json!({
        "calendars": [{
            "second": [{"start": 0, "end": 30, "step": 30}],
            "minute": [{"start": 0, "end": 59, "step": 15}],
            "hour": [{"start": 9, "end": 17, "step": 1}],
            "dayOfMonth": [{"start": 1, "end": 31, "step": 1}],
            "month": [{"start": 1, "end": 12, "step": 1}],
            "year": [{"start": 2026, "end": 2030, "step": 1}],
            "dayOfWeek": [{"start": 1, "end": 5, "step": 1}],
            "comment": "business hours"
        }],
        "cron": ["0 9 * * 1-5"],
        "intervals": [{"everyMs": 3_600_000, "offsetMs": 900_000}],
        "exclusions": [{"dayOfMonth": [{"start": 25, "end": 25, "step": 1}], "month": [{"start": 12, "end": 12, "step": 1}]}],
        "startAt": "2026-01-01T00:00:00Z",
        "endAt": "2031-01-01T00:00:00Z",
        "timezone": "Europe/Lisbon",
        "jitterMs": 5_000,
        "dst": {"gap": "SHIFT_FORWARD", "ambiguous": "TWICE"}
    })
}

#[test]
fn a_canonical_spec_survives_a_round_trip_unchanged() {
    let doc = rich();
    let decoded = spec(&doc).expect("decode");
    assert_eq!(spec_to_json(&decoded), doc);
    let again = spec(&spec_to_json(&decoded)).expect("decode again");
    assert_eq!(again, decoded);
}

#[test]
fn a_minimal_spec_gets_its_defaults_and_encodes_them_explicitly() {
    let decoded = spec(&json!({"cron": ["0 9 * * *"]})).expect("decode");
    assert_eq!(decoded.timezone(), "UTC");
    assert_eq!(decoded.jitter_ms(), 0);
    assert_eq!(decoded.dst().gap(), DstGap::Skip);
    assert_eq!(decoded.dst().ambiguous(), DstAmbiguous::OnceEarlier);
    let encoded = spec_to_json(&decoded);
    assert_eq!(encoded["timezone"], "UTC");
    assert_eq!(encoded["jitterMs"], 0);
    assert_eq!(
        encoded["dst"],
        json!({"gap": "SKIP", "ambiguous": "ONCE_EARLIER"})
    );
    assert_eq!(encoded["cron"], json!(["0 9 * * *"]));
    assert!(
        encoded.get("startAt").is_none(),
        "an absent bound stays absent"
    );
}

#[test]
fn a_range_without_end_or_step_is_one_value() {
    let decoded = spec(&json!({"calendars": [{"hour": [{"start": 9}]}]})).expect("decode");
    let encoded = spec_to_json(&decoded);
    assert_eq!(
        encoded["calendars"][0]["hour"],
        json!([{"start": 9, "end": 9, "step": 1}])
    );
}

#[test]
fn an_offset_in_a_bound_is_normalised_to_utc() {
    let decoded = spec(&json!({"cron": ["0 9 * * *"], "startAt": "2026-03-01T10:00:00+02:00"}))
        .expect("decode");
    assert_eq!(spec_to_json(&decoded)["startAt"], "2026-03-01T08:00:00Z");
}

#[test]
fn an_unknown_field_is_refused_where_it_is_with_the_path() {
    let cases: [(Value, &str); 5] = [
        (json!({"cron": ["0 9 * * *"], "cronn": []}), "spec.cronn"),
        (
            json!({"intervals": [{"everyMs": 60_000, "every": 1}]}),
            "spec.intervals[0].every",
        ),
        (
            json!({"calendars": [{"hour": [{"start": 1, "stop": 2}]}]}),
            "spec.calendars[0].hour[0].stop",
        ),
        (
            json!({"calendars": [{"hours": []}]}),
            "spec.calendars[0].hours",
        ),
        (
            json!({"cron": ["0 9 * * *"], "dst": {"gap": "SKIP", "when": 1}}),
            "spec.dst.when",
        ),
    ];
    for (doc, path) in cases {
        let err = failure(spec(&doc));
        assert_eq!(
            (err.path.as_str(), err.reason),
            (path, CodecReason::UnknownField),
            "{doc}"
        );
    }
}

#[test]
fn a_value_of_the_wrong_kind_is_refused_where_it_is() {
    let cases: [(Value, &str); 6] = [
        (json!({"cron": "0 9 * * *"}), "spec.cron"),
        (json!({"cron": [5]}), "spec.cron[0]"),
        (
            json!({"intervals": [{"everyMs": "60000"}]}),
            "spec.intervals[0].everyMs",
        ),
        (
            json!({"intervals": [{"everyMs": -1}]}),
            "spec.intervals[0].everyMs",
        ),
        (
            json!({"cron": ["0 9 * * *"], "jitterMs": 1.5}),
            "spec.jitterMs",
        ),
        (
            json!({"cron": ["0 9 * * *"], "timezone": 3}),
            "spec.timezone",
        ),
    ];
    for (doc, path) in cases {
        let err = failure(spec(&doc));
        assert_eq!(
            (err.path.as_str(), err.reason),
            (path, CodecReason::WrongType),
            "{doc}"
        );
    }
    assert_eq!(
        failure(spec_from_json(&json!([]), &Ceilings::default())).path,
        "spec"
    );
}

#[test]
fn a_time_that_is_not_rfc_3339_is_refused() {
    for bad in [
        "yesterday",
        "2026-13-01T00:00:00Z",
        "2026-03-01",
        "2026-03-01T10:00:00",
    ] {
        let err = failure(spec(&json!({"cron": ["0 9 * * *"], "endAt": bad})));
        assert_eq!(
            (err.path.as_str(), err.reason),
            ("spec.endAt", CodecReason::BadTime),
            "{bad}"
        );
    }
}

#[test]
fn what_the_domain_refuses_comes_back_with_its_reason_and_the_place() {
    let err = failure(spec(&json!({})));
    assert_eq!(
        (err.path.as_str(), err.reason),
        ("spec", CodecReason::Invalid(ScheduleError::NoEntries))
    );

    let err = failure(spec(&json!({"intervals": [{"everyMs": 9_999}]})));
    assert_eq!(err.path, "spec.intervals[0]");
    assert_eq!(err.reason, CodecReason::Invalid(ScheduleError::BadInterval));

    let err = failure(spec(&json!({"calendars": [{"hour": [{"start": 24}]}]})));
    assert_eq!(err.path, "spec.calendars[0]");
    assert_eq!(
        err.reason,
        CodecReason::Invalid(ScheduleError::FieldOutOfDomain { field: "hour" })
    );

    let err = failure(spec(
        &json!({"calendars": [{"hour": [{"start": 5, "end": 4}]}]}),
    ));
    assert_eq!(err.path, "spec.calendars[0].hour[0]");
    assert_eq!(err.reason, CodecReason::Invalid(ScheduleError::BadRange));

    let err = failure(spec(&json!({"cron": ["0 9 * *"]})));
    assert_eq!(err.reason, CodecReason::Invalid(ScheduleError::BadCron));

    let err = failure(spec(
        &json!({"cron": ["0 9 * * *"], "jitterMs": 86_400_001u64}),
    ));
    assert_eq!(
        err.reason,
        CodecReason::Invalid(ScheduleError::JitterTooLarge)
    );

    let err = failure(spec(
        &json!({"cron": ["0 9 * * *"], "dst": {"gap": "FORWARD"}}),
    ));
    assert_eq!(
        (err.path.as_str(), err.reason),
        ("spec.dst.gap", CodecReason::UnknownValue)
    );
}

#[test]
fn the_entry_ceiling_is_the_one_of_the_namespace() {
    let doc = json!({"cron": vec!["0 9 * * *"; 11]});
    assert_eq!(
        failure(spec(&doc)).reason,
        CodecReason::Invalid(ScheduleError::TooManyEntries { limit: 10 })
    );
    let raised = Ceilings {
        spec_entries: 32,
        ..Ceilings::default()
    };
    assert!(spec_from_json(&doc, &raised).is_ok());
}

// --- the policies -------------------------------------------------------------------------------

#[test]
fn policies_default_and_round_trip() {
    let defaults = policies_from_json(&json!({}), &Ceilings::default()).expect("defaults");
    assert_eq!(defaults.overlap(), Overlap::Skip);
    assert_eq!(defaults.catchup_mode(), CatchupMode::One);
    assert_eq!(defaults.catchup_window_ms(), 31_622_400_000);
    assert!(!defaults.pause_on_failure());
    assert_eq!(defaults.buffer_limit(), 100);
    assert_eq!(
        policies_to_json(&defaults),
        json!({
            "overlap": "SKIP", "catchupWindowMs": 31_622_400_000u64, "catchupMode": "ONE",
            "pauseOnFailure": false, "bufferLimit": 100
        })
    );
    let doc = json!({
        "overlap": "CANCEL_OTHER", "catchupWindowMs": 3_600_000, "catchupMode": "ALL",
        "pauseOnFailure": true, "bufferLimit": 7
    });
    let decoded = policies_from_json(&doc, &Ceilings::default()).expect("decode");
    assert_eq!(policies_to_json(&decoded), doc);
}

#[test]
fn policies_refuse_what_the_contract_does_not_name() {
    let ceilings = Ceilings::default();
    for (doc, path, reason) in [
        (
            json!({"overlap": "TERMINATE_OTHER"}),
            "policies.overlap",
            CodecReason::UnknownValue,
        ),
        (
            json!({"catchupMode": "SOME"}),
            "policies.catchupMode",
            CodecReason::UnknownValue,
        ),
        (
            json!({"pauseOnFailure": "yes"}),
            "policies.pauseOnFailure",
            CodecReason::WrongType,
        ),
        (
            json!({"bufferSize": 3}),
            "policies.bufferSize",
            CodecReason::UnknownField,
        ),
        (
            json!({"catchupWindowMs": 5}),
            "policies",
            CodecReason::Invalid(ScheduleError::BadCatchupWindow),
        ),
        (
            json!({"bufferLimit": 0}),
            "policies",
            CodecReason::Invalid(ScheduleError::BadBufferLimit),
        ),
    ] {
        let err = failure(policies_from_json(&doc, &ceilings));
        assert_eq!((err.path.as_str(), err.reason), (path, reason), "{doc}");
    }
}

#[test]
fn a_buffer_limit_above_the_namespace_ceiling_is_lowered_to_it() {
    let ceilings = Ceilings {
        buffer_limit: 20,
        ..Ceilings::default()
    };
    let decoded = policies_from_json(&json!({"bufferLimit": 500}), &ceilings).expect("decode");
    assert_eq!(decoded.buffer_limit(), 20);
}

// --- the target ---------------------------------------------------------------------------------

#[test]
fn a_target_round_trips_with_its_defaults_made_explicit() {
    let workflow = WorkflowId::generate();
    let decoded = target_from_json(&json!({"workflowId": workflow.to_string()})).expect("decode");
    assert_eq!(decoded.workflow(), workflow);
    assert_eq!(decoded.version(), VersionPolicy::Active);
    assert_eq!(
        target_to_json(&decoded),
        json!({"workflowId": workflow.to_string(), "versionPolicy": "ACTIVE"})
    );

    let version = WorkflowVersionId::generate();
    let doc = json!({
        "workflowId": workflow.to_string(),
        "versionPolicy": "PINNED",
        "versionId": version.to_string(),
        "nominalTimeField": "scheduledFor",
        "runTimeoutMs": 60_000,
        "businessIdPrefix": "order",
        "businessIdConflictPolicy": "USE_EXISTING",
        "businessIdReusePolicy": "REJECT_DUPLICATE"
    });
    let decoded = target_from_json(&doc).expect("decode");
    assert_eq!(decoded.version(), VersionPolicy::Pinned(version));
    let id = decoded.business_id().expect("business id");
    assert_eq!(
        (id.conflict(), id.reuse()),
        (ConflictPolicy::UseExisting, ReusePolicy::RejectDuplicate)
    );
    assert_eq!(target_to_json(&decoded), doc);
}

#[test]
fn a_pinned_target_needs_a_version_and_an_active_one_refuses_it() {
    let workflow = WorkflowId::generate().to_string();
    let version = WorkflowVersionId::generate().to_string();
    let err = failure(target_from_json(
        &json!({"workflowId": workflow, "versionPolicy": "PINNED"}),
    ));
    assert_eq!(
        (err.path.as_str(), err.reason),
        ("action.versionId", CodecReason::Missing)
    );
    let err = failure(target_from_json(
        &json!({"workflowId": workflow, "versionId": version}),
    ));
    assert_eq!(
        (err.path.as_str(), err.reason),
        ("action.versionId", CodecReason::Unexpected)
    );
    let err = failure(target_from_json(
        &json!({"workflowId": workflow, "versionPolicy": "LATEST"}),
    ));
    assert_eq!(
        (err.path.as_str(), err.reason),
        ("action.versionPolicy", CodecReason::UnknownValue)
    );
}

#[test]
fn a_target_needs_a_workflow_and_refuses_what_it_does_not_know() {
    let err = failure(target_from_json(&json!({})));
    assert_eq!(
        (err.path.as_str(), err.reason),
        ("action.workflowId", CodecReason::Missing)
    );
    let err = failure(target_from_json(&json!({"workflowId": "nope"})));
    assert_eq!(
        (err.path.as_str(), err.reason),
        ("action.workflowId", CodecReason::WrongType)
    );
    let workflow = WorkflowId::generate().to_string();
    let err = failure(target_from_json(
        &json!({"workflowId": workflow, "input": {}}),
    ));
    assert_eq!(
        (err.path.as_str(), err.reason),
        ("action.input", CodecReason::UnknownField),
        "the input is a payload of its own, not part of the stored target"
    );
    let err = failure(target_from_json(
        &json!({"workflowId": workflow, "runTimeoutMs": 5}),
    ));
    assert_eq!(
        err.reason,
        CodecReason::Invalid(ScheduleError::BadRunTimeout)
    );
    let err = failure(target_from_json(
        &json!({"workflowId": workflow, "businessIdPrefix": "bad id"}),
    ));
    assert_eq!(
        err.reason,
        CodecReason::Invalid(ScheduleError::BadBusinessIdPrefix)
    );
}
