//! The ceilings feature 018 adds say which they are (`FR-018-043`, `contracts/errors.md`): a refusal at
//! one of them carries `details.reason` next to `details.limit`, with the remediation of its own; a
//! refusal at any other limit is what it always was.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_api_http::error::ApiError;
use sagat_application::error::AppError;
use sagat_domain::quota::QuotaResource;

fn details_of(limit: &'static str) -> (serde_json::Value, &'static str) {
    let error = ApiError::from(AppError::QuotaExceeded(limit));
    let parts = error.parts();
    let details = parts.details.clone().expect("details");
    let advice = parts.code.remediation(details["reason"].as_str());
    (details, advice)
}

#[test]
fn the_ceilings_of_schedules_backfills_and_batches_name_themselves() {
    for (limit, reason) in [
        (QuotaResource::Schedules.limit_name(), "SCHEDULE_LIMIT"),
        (
            QuotaResource::ScheduleBackfills.limit_name(),
            "BACKFILL_CONCURRENCY",
        ),
        ("schedule_backfills_per_schedule", "BACKFILL_CONCURRENCY"),
        (QuotaResource::BatchJobs.limit_name(), "BATCH_LIMIT"),
        (QuotaResource::TenantBatchJobs.limit_name(), "BATCH_LIMIT"),
    ] {
        let (details, advice) = details_of(limit);
        assert_eq!(details["reason"], reason, "{limit}");
        assert_eq!(details["limit"], limit, "the limit stays named: {limit}");
        assert!(
            advice.contains("ceiling is reached"),
            "{limit} has the sentence of its row: {advice}"
        );
    }
}

#[test]
fn another_limit_is_refused_as_it_always_was() {
    let (details, advice) = details_of(QuotaResource::Workflows.limit_name());
    assert!(details.get("reason").is_none(), "{details}");
    assert_eq!(details["limit"], QuotaResource::Workflows.limit_name());
    assert!(advice.contains("Release something"), "{advice}");
}
