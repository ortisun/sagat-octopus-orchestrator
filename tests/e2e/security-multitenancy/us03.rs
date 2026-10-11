//! US03 — a script cannot reach a forbidden endpoint through either the mediated host call or its
//! container, and the attempted egress is recorded without its path, query, or credentials.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use reqwest::Method;
use sagat_domain::scripts::{Egress, Policy};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

use crate::support::api::Options;
use crate::support::scripts::{Lab, lab_with};

async fn egress_lab(port: u16) -> Lab {
    let lab = lab_with(Options {
        script_ceiling: Policy {
            egress: Egress::Namespace,
            ..Policy::default()
        },
        ..Options::default()
    })
    .await;
    let (status, body) = lab
        .call(
            Method::PUT,
            &format!("/namespaces/{}/egress-policy", lab.ns),
            Some(&json!({
                "rules": ["127.0.0.1"],
                "ports": [port],
                "allowHttp": true
            })),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    lab
}

async fn run_script(lab: &Lab, name: &str, language: &str, source: String) -> (String, Value) {
    let script = lab
        .script_with(
            name,
            language,
            &source,
            json!({"policy": {"egress": "namespace"}}),
        )
        .await;
    let digest = lab.ready(&script).await;
    let workflow = lab
        .workflow(&format!("wf-{name}"), &digest, json!({}))
        .await;
    let run = lab.p.start(&workflow, &json!({})).await;
    assert_eq!(lab.drive(&run).await, "SUCCEEDED");
    (digest, lab.output(&run, "s").await.expect("script output"))
}

#[tokio::test]
async fn javascript_and_python_cannot_reach_a_forbidden_endpoint_and_the_incident_is_audited() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind canary");
    let port = listener.local_addr().expect("canary address").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&hits);
    let canary = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
                .await;
        }
    });
    let lab = egress_lab(port).await;
    let target = format!("http://127.0.0.1:{port}/private?credential=CANARY-SECRET");

    let (typescript_digest, typescript_output) = run_script(
        &lab,
        "blocked-typescript",
        "typescript",
        format!(
            "export default async function (_input: unknown, octopus: any) {{\n\
             let mediated = 'reached';\n\
             try {{ await octopus.fetch('{target}'); }} catch (e) {{ mediated = String((e as Error).message); }}\n\
             let direct = 'reached';\n\
             try {{ await fetch('{target}', {{ signal: AbortSignal.timeout(1000) }}); }} catch {{ direct = 'unreachable'; }}\n\
             return {{ mediated, direct }};\n\
             }}\n"
        ),
    )
    .await;
    assert_eq!(
        typescript_output,
        json!({"mediated": "denied", "direct": "unreachable"})
    );

    let (python_digest, python_output) = run_script(
        &lab,
        "blocked-python",
        "python",
        format!(
            r#"def run(input, octopus):
    mediated = 'reached'
    try:
        octopus.fetch('{target}')
    except Exception as error:
        mediated = str(error)
    direct = 'reached'
    try:
        import urllib.request
        urllib.request.urlopen('{target}', timeout=1)
    except Exception:
        direct = 'unreachable'
    return {{'mediated': mediated, 'direct': direct}}
"#
        ),
    )
    .await;
    assert_eq!(
        python_output,
        json!({"mediated": "denied", "direct": "unreachable"})
    );

    for digest in [typescript_digest, python_digest] {
        let executions = lab.executions(&digest).await;
        assert_eq!(
            executions[0]["denied"],
            json!([format!("fetch http://127.0.0.1:{port}")])
        );
        assert!(!executions[0].to_string().contains("CANARY-SECRET"));
    }
    assert_eq!(
        lab.audited("script.violation").await,
        vec![
            "FORBIDDEN_ADDRESS".to_owned(),
            "FORBIDDEN_ADDRESS".to_owned(),
        ]
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "neither the runner nor a container connected to the canary"
    );
    canary.abort();
}
