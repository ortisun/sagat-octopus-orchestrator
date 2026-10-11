//! What publishing, approving and moving the pointer leave on the metrics page, and what they do
//! not: closed sets of labels, and nobody named (NFR-011-OBS).

use reqwest::Method;
use sagat_domain::identity::Role;
use sagat_telemetry::Metrics;
use serde_json::json;

use crate::support::worker::platform;
use crate::{Dir, cli, flow, rest_in, workflow_id};

/// The sum of every series of `name` whose line carries all of `labels`.
fn total(page: &str, name: &str, labels: &[(&str, &str)]) -> f64 {
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
async fn approvals_refusals_activations_and_promotions_are_counted_without_naming_anyone() {
    // The recorder is process-wide and the other stories count too: every check is "at least one
    // more than before".
    let metrics = Metrics::global();
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    p.grant(p.tenant, "dev2", Role::Developer).await;
    p.grant(p.tenant, "olga", Role::Operator).await;
    let (_, workspaces) = rest_in(&p, "alice", "payments", Method::GET, "/workspaces", None).await;
    let workspace = workspaces["items"][0]["id"]
        .as_str()
        .expect("workspace")
        .to_owned();
    let (status, prod) = rest_in(
        &p,
        "alice",
        "payments",
        Method::POST,
        &format!("/workspaces/{workspace}/namespaces"),
        Some(&json!({"slug": "billing-prod", "environment": "prod"})),
    )
    .await;
    assert_eq!(status, 201);
    let prod_id = prod["id"].as_str().expect("id").to_owned();
    let before = metrics.render();

    // A direct publish into production is refused; a request is opened, approved and published.
    let (_, created) = rest_in(
        &p,
        "dev1",
        "billing-prod",
        Method::POST,
        "/workflows",
        Some(&json!({"name": "invoicing", "ir": flow("invoicing", "one")})),
    )
    .await;
    let invoicing = created["id"].as_str().expect("id").to_owned();
    let (status, _) = rest_in(
        &p,
        "dev1",
        "billing-prod",
        Method::POST,
        &format!("/workflows/{invoicing}/publish"),
        Some(&json!({"draftRevision": 1})),
    )
    .await;
    assert_eq!(status, 409);
    let (status, request) = rest_in(
        &p,
        "dev1",
        "billing-prod",
        Method::POST,
        &format!("/workflows/{invoicing}/publication-requests"),
        Some(&json!({"draftRevision": 1})),
    )
    .await;
    assert_eq!(status, 201, "{request}");
    let id = request["id"].as_str().expect("id");
    let (status, _) = rest_in(
        &p,
        "dev2",
        "billing-prod",
        Method::POST,
        &format!("/publication-requests/{id}/approve"),
        Some(&json!({"reason": "reviewed the document"})),
    )
    .await;
    assert_eq!(status, 200);
    let (status, _) = rest_in(
        &p,
        "dev1",
        "billing-prod",
        Method::POST,
        &format!("/publication-requests/{id}/publish"),
        None,
    )
    .await;
    assert_eq!(status, 201);

    // A promotion into production that waits, a rejection and a withdrawal.
    let dir = Dir::new();
    dir.document("checkout", &flow("checkout", "one"));
    dir.manifest("payments", &["checkout"]);
    cli(&p, &dir, "dev1", &["publish"]).await.expect(0);
    cli(
        &p,
        &dir,
        "dev1",
        &[
            "promote",
            "--workflow",
            "checkout",
            "--to",
            "billing-prod",
            "--reason",
            "release train 42",
            "--namespace",
            "payments",
        ],
    )
    .await
    .expect(3);
    let waiting = cli(
        &p,
        &dir,
        "dev1",
        &[
            "requests",
            "--workflow",
            "checkout",
            "--namespace",
            "billing-prod",
        ],
    )
    .await;
    let waiting_id = waiting
        .out
        .split_whitespace()
        .next()
        .expect("id")
        .to_owned();
    cli(
        &p,
        &dir,
        "dev2",
        &[
            "reject",
            "--request",
            &waiting_id,
            "--reason",
            "not this week",
            "--namespace",
            "billing-prod",
        ],
    )
    .await
    .expect(0);
    // A request that its author takes back.
    let (status, again) = rest_in(
        &p,
        "dev1",
        "billing-prod",
        Method::POST,
        &format!("/workflows/{invoicing}/publication-requests"),
        Some(&json!({"draftRevision": 1})),
    )
    .await;
    assert_eq!(status, 201, "{again}");
    let again_id = again["id"].as_str().expect("id").to_owned();
    let (status, _) = rest_in(
        &p,
        "dev1",
        "billing-prod",
        Method::POST,
        &format!("/publication-requests/{again_id}/withdraw"),
        None,
    )
    .await;
    assert_eq!(status, 200);

    // A rollback and a promotion that publishes.
    dir.document("checkout", &flow("checkout", "two"));
    cli(&p, &dir, "dev1", &["publish"]).await.expect(0);
    cli(
        &p,
        &dir,
        "olga",
        &[
            "rollback",
            "--workflow",
            "checkout",
            "--reason",
            "two misbehaves",
        ],
    )
    .await
    .expect(0);
    let checkout = workflow_id(&p, "payments", "checkout").await;
    let (status, _) = rest_in(
        &p,
        "dev1",
        "payments",
        Method::POST,
        &format!("/workflows/{checkout}/versions/1/promote"),
        Some(&json!({"namespaceId": prod_id, "reason": "release train 43"})),
    )
    .await;
    assert!(status == 202 || status == 409, "{status}");

    let after = metrics.render();
    for (name, labels) in [
        (
            "publication_refusals_total",
            vec![("reason", "APPROVAL_REQUIRED")],
        ),
        ("publication_events_total", vec![("event", "requested")]),
        ("publication_events_total", vec![("event", "approved")]),
        ("publication_events_total", vec![("event", "rejected")]),
        ("publication_events_total", vec![("event", "published")]),
        ("publication_events_total", vec![("event", "withdrawn")]),
        ("version_activations_total", vec![("kind", "rollback")]),
        ("promotions_total", vec![("outcome", "requested")]),
    ] {
        assert!(
            total(&after, name, &labels) >= total(&before, name, &labels) + 1.0,
            "{name} {labels:?} did not move:\n{after}"
        );
    }

    // Nobody is named: not the people, not the tenant, not the workflows, not the requests.
    for forbidden in [
        "dev1",
        "dev2",
        "olga",
        "alice",
        "acme",
        "invoicing",
        "checkout",
        "billing-prod",
        id,
        &waiting_id,
        &prod_id,
    ] {
        assert!(
            !after.contains(forbidden),
            "the metrics page names {forbidden:?}"
        );
    }
}
