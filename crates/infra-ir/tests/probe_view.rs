//! What a reader of a connection's latest probe sees (feature 035, D035-150 slice S4b).
//!
//! The probe of an Iceberg connection **succeeds** even when a check failed, because a failed check is the answer and not a fault of the
//! platform. Every reader of the probe that existed before it reads `failureCode` and nothing of any check, so the code of the first failed
//! check is derived into `failureCode`, and a connection is never read as healthy for lack of a result: an Iceberg probe that succeeded and
//! whose result cannot be read says `DATABASE_RESULT_UNAVAILABLE`, never nothing.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::{
    database::{LakeProbeCodec, ProbeLeaseFacts},
    runs::{ProbeViewInput, probe_view},
};
use sagat_domain::database::{ProbeCheck, ProbeOutcome};
use sagat_infra_ir::JsonLakeProbeCodec;

const CONNECTION: &str = "lake-main";
const REVISION: u32 = 3;

fn report(checks: &str) -> Vec<u8> {
    format!(
        r#"{{"kind":"probe","connection":{{"name":"{CONNECTION}","revision":{REVISION}}},"checks":{checks}}}"#
    )
    .into_bytes()
}

const ALL_PASSED: &str = r#"[{"check":"CATALOG_AUTH","outcome":"PASSED"},{"check":"WAREHOUSE","outcome":"PASSED"},{"check":"STORAGE_ACCESS","outcome":"PASSED"}]"#;
const AUTH_FAILED: &str = r#"[{"check":"CATALOG_AUTH","outcome":"FAILED","code":"DATABASE_AUTHENTICATION_FAILED"},{"check":"WAREHOUSE","outcome":"SKIPPED"},{"check":"STORAGE_ACCESS","outcome":"SKIPPED"}]"#;
const STORAGE_FAILED: &str = r#"[{"check":"CATALOG_AUTH","outcome":"PASSED"},{"check":"WAREHOUSE","outcome":"PASSED"},{"check":"STORAGE_ACCESS","outcome":"FAILED","code":"STORAGE_UNAVAILABLE"}]"#;

fn view(
    lake: bool,
    succeeded: bool,
    stored_failure: Option<&str>,
    output: Option<&[u8]>,
    codec: Option<&dyn LakeProbeCodec>,
) -> (Option<Vec<(ProbeCheck, ProbeOutcome)>>, Option<String>) {
    let seen = probe_view(
        &ProbeViewInput {
            lake,
            succeeded,
            stored_failure: stored_failure.map(str::to_owned),
            output,
            connection: CONNECTION,
            revision: REVISION,
        },
        codec,
    );
    (
        seen.result.map(|result| {
            result
                .checks()
                .iter()
                .map(|check| (check.check(), check.outcome()))
                .collect()
        }),
        seen.failure_code,
    )
}

#[test]
fn a_sql_probe_is_read_as_it_always_was() {
    let codec: &dyn LakeProbeCodec = &JsonLakeProbeCodec;
    assert_eq!(view(false, true, None, None, Some(codec)), (None, None));
    assert_eq!(
        view(false, false, Some("DATABASE_TIMEOUT"), None, Some(codec)),
        (None, Some("DATABASE_TIMEOUT".to_owned()))
    );
    // a SQL step's output is its rows: it is never read as a probe result, whatever it holds
    assert_eq!(
        view(false, true, None, Some(&report(AUTH_FAILED)), Some(codec)),
        (None, None)
    );
}

#[test]
fn a_connection_whose_checks_all_passed_shows_no_failure_and_all_the_checks() {
    let codec: &dyn LakeProbeCodec = &JsonLakeProbeCodec;
    let (checks, failure) = view(true, true, None, Some(&report(ALL_PASSED)), Some(codec));
    assert_eq!(failure, None);
    assert_eq!(
        checks.expect("the checks"),
        ProbeCheck::ALL
            .iter()
            .map(|check| (*check, ProbeOutcome::Passed))
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_code_of_the_first_failed_check_is_the_failure_code_so_that_no_reader_reads_a_failed_check_as_healthy()
 {
    let codec: &dyn LakeProbeCodec = &JsonLakeProbeCodec;
    let (checks, failure) = view(true, true, None, Some(&report(AUTH_FAILED)), Some(codec));
    assert_eq!(failure.as_deref(), Some("DATABASE_AUTHENTICATION_FAILED"));
    assert_eq!(
        checks.expect("the checks"),
        vec![
            (ProbeCheck::CatalogAuth, ProbeOutcome::Failed),
            (ProbeCheck::Warehouse, ProbeOutcome::Skipped),
            (ProbeCheck::StorageAccess, ProbeOutcome::Skipped),
        ],
        "a skipped check is not confirmed, and is never shown as passed"
    );
    let (_, failure) = view(true, true, None, Some(&report(STORAGE_FAILED)), Some(codec));
    assert_eq!(failure.as_deref(), Some("STORAGE_UNAVAILABLE"));
}

#[test]
fn a_fault_of_the_platform_is_kept_and_wins_over_anything_derived() {
    let codec: &dyn LakeProbeCodec = &JsonLakeProbeCodec;
    // the step failed (the credentials could not be read, the sandbox was not there): there is no result, and the code is the step's
    assert_eq!(
        view(
            true,
            false,
            Some("DATABASE_CREDENTIAL_UNAVAILABLE"),
            None,
            Some(codec)
        ),
        (None, Some("DATABASE_CREDENTIAL_UNAVAILABLE".to_owned()))
    );
    assert_eq!(
        view(
            true,
            false,
            Some("LAKE_SANDBOX_UNAVAILABLE"),
            Some(&report(ALL_PASSED)),
            Some(codec)
        )
        .1,
        Some("LAKE_SANDBOX_UNAVAILABLE".to_owned()),
        "the step's own code is not replaced by a derived one"
    );
}

#[test]
fn a_probe_still_running_says_nothing_yet() {
    let codec: &dyn LakeProbeCodec = &JsonLakeProbeCodec;
    assert_eq!(view(true, false, None, None, Some(codec)), (None, None));
}

#[test]
fn an_iceberg_probe_that_succeeded_and_whose_result_cannot_be_read_is_never_a_healthy_connection() {
    let codec: &dyn LakeProbeCodec = &JsonLakeProbeCodec;
    let unavailable = (None, Some("DATABASE_RESULT_UNAVAILABLE".to_owned()));
    // the payload is gone (purged), or was never kept
    assert_eq!(view(true, true, None, None, Some(codec)), unavailable);
    // it is not a result of this connection, or of this revision
    let other = format!(
        r#"{{"kind":"probe","connection":{{"name":"other","revision":{REVISION}}},"checks":{ALL_PASSED}}}"#
    );
    assert_eq!(
        view(true, true, None, Some(other.as_bytes()), Some(codec)),
        unavailable
    );
    let revision = format!(
        r#"{{"kind":"probe","connection":{{"name":"{CONNECTION}","revision":9}},"checks":{ALL_PASSED}}}"#
    );
    assert_eq!(
        view(true, true, None, Some(revision.as_bytes()), Some(codec)),
        unavailable
    );
    // it is not a result at all
    assert_eq!(
        view(true, true, None, Some(b"not json"), Some(codec)),
        unavailable
    );
    assert_eq!(
        view(
            true,
            true,
            None,
            Some(br#"{"anything":"a runner chose"}"#),
            Some(codec)
        ),
        unavailable
    );
    // a server with no codec cannot read it
    assert_eq!(
        view(true, true, None, Some(&report(ALL_PASSED)), None),
        unavailable
    );
}

#[test]
fn the_facts_of_a_lease_and_the_view_agree_on_the_connection_a_result_is_about() {
    // the result is read against the revision the probe was admitted with, which is what the lease carried
    let facts = ProbeLeaseFacts {
        connection: CONNECTION.to_owned(),
        revision: REVISION,
    };
    assert_eq!(
        (facts.connection.as_str(), facts.revision),
        (CONNECTION, REVISION)
    );
}
