//! The saga machinery is counted, and the counters name no tenant, run or payment (NFR-003-OBS).

use serde_json::json;

use crate::support::worker::platform;
use crate::{decide, input, payment_saga, with_compensation_attempts};
use sagat_telemetry::Metrics;

/// The value of the series `name` whose labels include all of `labels`, summed.
fn counted(page: &str, name: &str, labels: &[(&str, &str)]) -> f64 {
    page.lines()
        .filter(|line| {
            line.starts_with(name)
                && matches!(line[name.len()..].chars().next(), Some('{' | ' '))
                && labels
                    .iter()
                    .all(|(k, v)| line.contains(&format!("{k}=\"{v}\"")))
        })
        .filter_map(|line| line.rsplit(' ').next()?.parse::<f64>().ok())
        .sum()
}

#[tokio::test]
async fn compensations_reconciliations_and_decisions_are_counted_without_naming_anyone() {
    // The recorder is process-wide and other tests of this binary count too: every check is "at
    // least this many more than before", read from the same page.
    let metrics = Metrics::global();
    let p = platform().await;
    let wf = p
        .publish(
            "corporate-payment",
            &with_compensation_attempts(payment_saga(), 1),
        )
        .await;
    let before = metrics.render();

    let run = p.start(&wf, &input()).await;
    let mut w = p.worker("wendy").await;
    w.register(&[
        "payment.validate",
        "limit.reserve",
        "payment.submit",
        "limit.release",
        "payment.return.request",
        "payment.lookup.status",
    ])
    .await;
    let validate = w.one_task().await;
    w.complete(&validate, &json!({})).await.expect("validate");
    let reserve = w.one_task().await;
    w.complete(&reserve, &json!({})).await.expect("reserve");
    let submit = w.one_task().await;

    // The response is lost; the lookup says the payment did not happen; the retry is declined.
    p.expire_lease(&run, "submit").await;
    p.tick().await;
    let lookup = w.one_task().await;
    w.complete(&lookup, &json!({"effect": "not_applied"}))
        .await
        .expect("answer");
    p.elapse_retry_delay(&run, "submit").await;
    p.tick().await;
    let again = w.one_task().await;
    w.fail(&again, false).await.expect("declined");
    let _ = submit;

    // The release fails once and waits for a person, who gives up on it.
    let release = w.one_task().await;
    w.fail_with(&release, "LIMIT_SERVICE_DOWN", true)
        .await
        .expect("down");
    let (status, body) = decide(
        &p,
        None,
        &run,
        &json!({"action": "SKIP_COMPENSATION", "stepId": "reserve~c", "reason": "released by hand, OPS-1"}),
    )
    .await;
    assert_eq!(status, 202, "{body}");

    let after = metrics.render();
    let more = |name: &str, labels: &[(&str, &str)]| {
        counted(&after, name, labels) - counted(&before, name, labels)
    };
    assert!(
        more("saga_transitions_total", &[("event", "entered")]) >= 1.0,
        "{after}"
    );
    assert!(more("saga_transitions_total", &[("event", "aborting")]) >= 1.0);
    assert!(more("saga_outcomes_total", &[("outcome", "partial")]) >= 1.0);
    assert!(more("reconciliations_total", &[("stage", "requested")]) >= 1.0);
    assert!(more("reconciliations_total", &[("stage", "not_applied")]) >= 1.0);
    assert!(more("operator_decisions_total", &[("action", "requested")]) >= 1.0);
    assert!(more("operator_decisions_total", &[("action", "skip")]) >= 1.0);
    assert!(more("compensation_attempts_total", &[("result", "needs_review")]) >= 1.0);

    // Closed label sets: not a tenant, a run, a node or a payment anywhere in these series.
    let series: Vec<&str> = after
        .lines()
        .filter(|l| {
            [
                "saga_",
                "compensation_",
                "reconciliations_",
                "operator_decisions_",
            ]
            .iter()
            .any(|prefix| l.starts_with(prefix))
        })
        .collect();
    assert!(!series.is_empty());
    for line in series {
        assert!(!line.contains(&run), "a run id in {line}");
        assert!(
            !line.contains("p-1") && !line.contains("acme") && !line.contains("reserve"),
            "{line}"
        );
    }
}
