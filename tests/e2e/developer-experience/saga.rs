//! US02 with an SDK (`US02-AC01`, `FR-012-002`): the payment saga of `examples/payment-saga` is
//! served by a worker written with the TypeScript SDK against a provider that is a few lines of
//! memory, and the day at the provider decides what the platform does: it compensates a declined
//! charge and reconciles a charge whose answer was lost, instead of charging again.
//!
//! `#[ignore]`: it needs Node. The `sdks` job of CI runs it with `--ignored`.

use reqwest::Method;

use crate::support::worker::platform;
use crate::us03::script;
use crate::{Dir, cli, engine, rest};

async fn event_types(p: &crate::support::worker::Platform, run: &str) -> Vec<String> {
    let (_, body) = rest(
        p,
        Method::GET,
        &format!("/runs/{run}/events?limit=200"),
        None,
    )
    .await;
    body["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|e| e["eventType"].as_str().map(str::to_owned))
        .collect()
}

fn position(lines: &[&str], text: &str) -> usize {
    lines
        .iter()
        .position(|l| *l == text)
        .unwrap_or_else(|| panic!("no line '{text}' in {lines:?}"))
}

#[tokio::test]
#[ignore = "needs Node: the sdks job of CI runs it with --ignored"]
async fn us02_the_payment_saga_of_the_example_compensates_a_decline_and_reconciles_a_lost_answer() {
    let p = platform().await;
    let engine = engine(&p).await;
    let dir = Dir::new();
    cli(&p, &dir, &["new", "compensation"]).await.expect(0);
    let file = "compensation/workflow.json";
    cli(&p, &dir, &["workflow", "publish", file])
        .await
        .expect(0);
    // Six names: the three steps, the two undos and the reconciler.
    cli(&p, &dir, &["worker", "bind", "--workflow", file])
        .await
        .expect(0)
        .mentions("bound alice to 6 activities in payments");

    let token = p.api.idp.token("alice", "acme").build();
    let setting = vec![
        ("SAGAT_SERVER", p.api.base.clone()),
        ("SAGAT_GRPC", engine.grpc_url.clone()),
        ("SAGAT_NAMESPACE", "payments".to_owned()),
        ("SAGAT_TOKEN", token.clone()),
    ];
    let prepared = script("payment-saga", "typescript", "prepare.sh", 900, &setting).await;
    assert_eq!(
        prepared.code,
        Some(0),
        "prepare.sh:\n{}\n{}",
        prepared.out,
        prepared.err
    );

    let day = |name: &'static str| {
        let setting = setting.clone();
        async move {
            // The day at the provider is the first argument of run.sh.
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("repo")
                .join("examples/payment-saga/typescript/run.sh");
            let mut command = tokio::process::Command::new(&path);
            command
                .arg(name)
                .current_dir(path.parent().expect("dir"))
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true);
            for (k, v) in &setting {
                command.env(k, v);
            }
            let out = tokio::time::timeout(std::time::Duration::from_secs(180), command.output())
                .await
                .unwrap_or_else(|_| panic!("{name} did not end within 180 s"))
                .expect("run");
            (
                out.status.code(),
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            )
        }
    };
    let run_id = |out: &str| {
        out.lines()
            .find_map(|l| l.strip_prefix("run ")?.strip_suffix(" started"))
            .unwrap_or_else(|| panic!("no run id in:\n{out}"))
            .to_owned()
    };

    // A good day: the reservation is made, the charge works, the customer is told.
    let (code, out, err) = day("ok").await;
    assert_eq!(code, Some(0), "ok:\n{out}\n{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(position(&lines, "charge p-1: ch-p-1") > position(&lines, "reserve p-1: r-p-1 for 10"));
    position(&lines, "notify p-1");
    position(
        &lines,
        "ledger: reservations=1 released=0 charges=1 refunded=0",
    );
    let events = event_types(&p, &run_id(&out)).await;
    assert!(!events.iter().any(|e| e.contains("compens")), "{events:?}");

    // The card is declined for good: the saga undoes the reservation, newest effect first, and
    // nobody is told.
    let (code, out, err) = day("decline").await;
    assert_eq!(code, Some(0), "decline:\n{out}\n{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        position(&lines, "release p-1: the reservation is released")
            > position(&lines, "charge p-1: declined (CARD_DECLINED)")
    );
    position(
        &lines,
        "ledger: reservations=1 released=1 charges=0 refunded=0",
    );
    assert!(!lines.contains(&"notify p-1"), "{out}");
    assert!(lines.iter().any(|l| l.ends_with(" COMPENSATED")), "{out}");
    let events = event_types(&p, &run_id(&out)).await;
    assert!(events.iter().any(|e| e.contains("compens")), "{events:?}");

    // The provider took the money and the answer was lost: the platform asks the provider whether
    // it happened (it did) and goes on. There is one charge, not two.
    let (code, out, err) = day("lost-answer").await;
    assert_eq!(code, Some(0), "lost-answer:\n{out}\n{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        position(&lines, "find-charge p-1: applied ch-p-1")
            > position(
                &lines,
                "charge p-1: taken by the provider, the answer was lost"
            )
    );
    position(&lines, "notify p-1");
    position(
        &lines,
        "ledger: reservations=1 released=0 charges=1 refunded=0",
    );
    let events = event_types(&p, &run_id(&out)).await;
    assert!(events.iter().any(|e| e.contains("reconcil")), "{events:?}");

    // And the token is not something the example prints.
    assert!(!out.contains(&token) && !err.contains(&token));
}
