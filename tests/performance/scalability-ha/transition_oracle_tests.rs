use super::*;
use crate::support::bounded;

#[test]
fn duplicate_task_id_is_a_fixed_safe_integrity_failure() {
    let run = RunId::generate();
    let completions = [
        CompletedTask {
            task_id: "same-task".to_owned(),
            run_id: run,
            node: "a".to_owned(),
        },
        CompletedTask {
            task_id: "same-task".to_owned(),
            run_id: run,
            node: "b".to_owned(),
        },
        CompletedTask {
            task_id: "third-task".to_owned(),
            run_id: run,
            node: "c".to_owned(),
        },
    ];

    let failure = verify_completion_identities(&[run], &completions)
        .expect_err("a duplicate TaskId must invalidate the configuration");
    assert_eq!(failure.label(), "duplicate_task_id");
}

#[tokio::test]
async fn pending_integrity_oracle_returns_a_safe_timeout_failure() {
    bounded::within(
        "pending integrity oracle test",
        Duration::from_secs(5),
        async {
            let result = bounded_integrity_oracle(Duration::from_millis(10), async {
                std::future::pending::<Result<(), IntegrityFailure>>().await
            })
            .await;

            assert!(matches!(result, Err(IntegrityFailure::TimedOut)));
            assert_eq!(
                result.expect_err("pending oracle must time out").label(),
                "integrity_oracle_timed_out"
            );
        },
    )
    .await;
}
