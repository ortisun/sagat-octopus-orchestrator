//! Bugfix T016-066 (G665, FR-016-030, D016-001): no read returns a digest of a payload.
//!
//! The journal read used to return `run_events.payload_digest`, the unsalted SHA-256 of an event's payload, as
//! `payloadRef`. A caller with `run:read` and no reveal grant could hash a guess of a classified step output and
//! compare. These tests run a real step whose output holds one `pii` boolean, show that the digest the server keeps
//! is exactly the SHA-256 of the guess (the attack works on what is stored), and then require that no surface a
//! `run:read` caller reaches carries it: the journal, the graph, its changes and the stream, for every role that
//! holds `run:read`, a reveal grant or not. The reference that replaces it names the payload's place and is
//! different for two equal outputs.
#![allow(clippy::expect_used, clippy::panic)]

use reqwest::Method;
use sagat_domain::identity::Role;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::support::worker::{HUMAN, NAMESPACE, Platform, TENANT, Worker, platform};

/// The output every run of the fixture makes, as the worker sends it: one `pii` flag, the kind of value
/// classification is most used for and the easiest to guess.
const OUTPUT: &[u8] = br#"{"approved":true}"#;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Every run of 64 hexadecimal characters in `text`: a SHA-256 in any field, under any name.
fn sha256_shaped(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut run = String::new();
    for c in text.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_hexdigit() {
            run.push(c);
        } else {
            if run.len() >= 64 {
                found.push(run.clone());
            }
            run.clear();
        }
    }
    found
}

fn classified_ir(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "decide",
        "nodes": [{
            "id": "decide", "type": "activity", "activity": "act.decide",
            "outputSchema": {
                "type": "object", "additionalProperties": false,
                "x-sagat-classification": "public",
                "properties": {"approved": {"type": "boolean", "x-sagat-classification": "pii"}}
            }
        }]
    })
}

async fn finished_run(p: &Platform, workflow: &str, worker: &mut Worker<'_>) -> String {
    let run = p.start(workflow, &json!({})).await;
    let lease = worker.one_task().await;
    let output: Value = serde_json::from_slice(OUTPUT).expect("output");
    worker
        .complete(&lease, &output)
        .await
        .expect("complete the classified step");
    // The stream below must end: let the scheduler finish whatever the completion left to do.
    p.tick().await;
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    run
}

/// What the server keeps for the step's output: the journal sequence and the digest.
async fn stored(p: &Platform, run: &str) -> (u64, String) {
    let (seq, digest): (i64, Vec<u8>) = sqlx::query_as(
        "SELECT seq, payload_digest FROM run_events \
         WHERE run_id = $1::uuid AND event_type = 'step.completed' AND payload_digest IS NOT NULL",
    )
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the completion keeps its output");
    (u64::try_from(seq).expect("positive sequence"), hex(&digest))
}

async fn raw_get(p: &Platform, subject: &str, path: &str) -> (u16, String) {
    let response = p
        .api
        .as_user(Method::GET, path, subject, TENANT)
        .header("x-octopus-namespace", NAMESPACE)
        .send()
        .await
        .expect("send");
    let status = response.status().as_u16();
    (status, response.text().await.expect("body"))
}

async fn grant_reveal(p: &Platform, subject: &str) {
    let namespace = p.namespace_id().await;
    sqlx::query(
        "INSERT INTO payload_reveal_grants (tenant_id,namespace_id,subject,granted_by,granted_at) \
         VALUES ($1,$2::uuid,$3,'alice',clock_timestamp())",
    )
    .bind(p.tenant.as_uuid())
    .bind(&namespace)
    .bind(subject)
    .execute(&p.api.db.pool)
    .await
    .expect("reveal grant");
}

#[tokio::test]
async fn no_run_read_surface_returns_the_digest_of_a_classified_output() {
    let p = platform().await;
    for (subject, role) in [
        ("vera", Role::Viewer),
        ("devon", Role::Developer),
        ("olivia", Role::Operator),
        ("gina", Role::Operator),
    ] {
        p.grant(p.tenant, subject, role).await;
    }
    // A caller who may reveal every value of the output still gets no digest (D016-001): they can disclose it.
    grant_reveal(&p, "gina").await;
    grant_reveal(&p, HUMAN).await;
    let workflow = p
        .publish(
            "classified-journal-reference",
            &classified_ir("classified-journal-reference"),
        )
        .await;
    let mut worker = p.worker("decider").await;
    worker.register(&["act.decide"]).await;
    let first = finished_run(&p, &workflow, &mut worker).await;
    let second = finished_run(&p, &workflow, &mut worker).await;

    let (first_seq, first_digest) = stored(&p, &first).await;
    let (second_seq, second_digest) = stored(&p, &second).await;
    // The attack: the digest the server keeps is the plain SHA-256 of the guessed bytes, the same in both runs.
    let guess = hex(&Sha256::digest(OUTPUT));
    assert_eq!(
        first_digest, guess,
        "the stored digest is the SHA-256 of the output"
    );
    assert_eq!(second_digest, guess);

    for run in [&first, &second] {
        for subject in ["vera", "devon", "olivia", "gina", HUMAN] {
            for suffix in [
                "/events?limit=200",
                "/graph",
                "/graph/changes?since=0",
                "/stream",
            ] {
                let path = format!("/runs/{run}{suffix}");
                let (status, body) = raw_get(&p, subject, &path).await;
                assert_eq!(status, 200, "{subject} {path}: {body}");
                assert!(
                    !body.to_ascii_lowercase().contains(&guess),
                    "{subject} reads the digest of a pii output on {path}: {body}"
                );
                assert_eq!(
                    sha256_shaped(&body),
                    Vec::<String>::new(),
                    "{subject} reads a SHA-256-shaped value on {path}"
                );
            }
        }
    }

    // The reference names the place: the run and the sequence, the selector inspection accepts.
    let reference = |run: &str, body: &Value| -> String {
        body["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|event| event["eventType"] == "step.completed")
            .and_then(|event| event["payloadRef"].as_str())
            .unwrap_or_else(|| panic!("the completion of {run} has a reference: {body}"))
            .to_owned()
    };
    let (status, first_page) = p
        .rest_as(
            "vera",
            Method::GET,
            &format!("/runs/{first}/events?limit=200"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{first_page}");
    let (status, second_page) = p
        .rest_as(
            "vera",
            Method::GET,
            &format!("/runs/{second}/events?limit=200"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{second_page}");
    let first_ref = reference(&first, &first_page);
    let second_ref = reference(&second, &second_page);
    assert_eq!(first_ref, format!("journal:{first}:{first_seq}"));
    assert_eq!(second_ref, format!("journal:{second}:{second_seq}"));
    assert_ne!(
        first_ref, second_ref,
        "two equal outputs must not have equal references: equality is an oracle too"
    );
    // An event that carries no payload still has none.
    assert!(
        first_page["items"]
            .as_array()
            .expect("items")
            .iter()
            .filter(|event| event["eventType"] != "step.completed")
            .all(|event| event["payloadRef"].is_null()),
        "{first_page}"
    );
    // The selector the reference names is the one inspection accepts.
    let (status, inspected) = p
        .rest_as(
            "olivia",
            Method::POST,
            &format!("/runs/{first}/payload-inspections"),
            Some(&json!({"source": "journal", "sequence": first_seq})),
        )
        .await;
    assert_eq!(status, 200, "{inspected}");
    assert_eq!(inspected["kind"], "inline-json", "{inspected}");
}
