//! Live conformance of the operations marked `x-implemented: '005'`.

use std::collections::BTreeSet;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sagat_domain::identity::{Membership, Role};
use serde_json::{Value, json};

use crate::support::api::{TestApi, start};
use crate::support::contract::{As, OpenApi, Scenario, check, check_body, scenario};
use crate::support::{now, subject};

const MISSING: &str = "019b5d6e-0000-7000-8000-000000000000";
const SOURCE: &str = "export default function (input: unknown) { return input; }\n";

async fn grant(api: &TestApi, tenant: sagat_domain::identity::TenantId, sub: &str, role: Role) {
    api.owner_store
        .insert_membership(&Membership::restore(tenant, subject(sub), role, now()))
        .await
        .expect("grant");
}

struct Run<'a> {
    api: &'a TestApi,
    spec: &'a OpenApi,
    covered: BTreeSet<(String, String)>,
}

impl Run<'_> {
    async fn all(&mut self, scenarios: Vec<Scenario>) {
        for s in &scenarios {
            let (method, template, _) = check(self.api, self.spec, s).await;
            self.covered.insert((method, template));
        }
    }

    async fn one(&mut self, s: Scenario) -> Value {
        let (method, template, _, body) = check_body(self.api, self.spec, &s).await;
        self.covered.insert((method, template));
        body
    }
}

#[tokio::test]
async fn every_script_operation_conforms_to_the_contract_on_its_normal_and_error_paths() {
    let spec = OpenApi::load();
    let api = start().await;
    let acme = api.tenant("acme", "alice").await;
    api.tenant("globex", "bob").await;
    grant(&api, acme, "vera", Role::Viewer).await;
    grant(&api, acme, "dave", Role::Developer).await;
    grant(&api, acme, "wanda", Role::Runner).await;
    let ws = api.create_workspace("alice", "acme", "platform").await;
    let ns = api
        .create_namespace("alice", "acme", &ws, "payments", "dev")
        .await;
    let globex_ws = api.create_workspace("bob", "globex", "platform").await;
    let other_ns = api
        .create_namespace("bob", "globex", &globex_ws, "payments", "dev")
        .await;

    let alice = As::User("alice", "acme");
    let vera = As::User("vera", "acme");
    let dave = As::User("dave", "acme");
    let wanda = As::User("wanda", "acme");
    let bob = As::User("bob", "globex");

    let mut run = Run {
        api: &api,
        spec: &spec,
        covered: BTreeSet::new(),
    };

    // ------------------------------------------------------------------------------ scripts
    let scripts_t = "/namespaces/{namespaceId}/scripts";
    let script_t = "/namespaces/{namespaceId}/scripts/{scriptId}";
    let scripts = format!("/namespaces/{ns}/scripts");
    let create = |who: &As, body: Value, expect: u16| {
        scenario("POST", scripts_t, &scripts, who.clone(), Some(body), expect)
    };
    let plain = json!({"name": "echo", "language": "typescript", "source": SOURCE});
    run.all(vec![
        create(&As::Anonymous, plain.clone(), 401),
        create(&As::Garbage, plain.clone(), 401),
        create(&vera, plain.clone(), 403),
        create(&wanda, plain.clone(), 403),
        create(&bob, plain.clone(), 404),
        create(&alice, json!({"name": "Bad_Name", "language": "typescript", "source": SOURCE}), 422),
        create(&alice, json!({"name": "echo", "language": "typescript", "source": SOURCE, "extra": 1}), 422),
        create(&alice, json!({"name": "echo", "language": "go", "source": SOURCE}), 422),
        create(&alice, json!({"name": "echo", "language": "cobol", "source": SOURCE}), 422),
        create(&alice, json!({"name": "echo", "language": "typescript", "source": ""}), 422),
        create(&alice, json!({"name": "echo", "language": "typescript", "source": SOURCE, "inputSchema": {"type": 12}}), 422),
        create(&alice, json!({"name": "echo", "language": "typescript", "source": SOURCE, "policy": {"memoryMib": 4096}}), 422),
        create(&alice, json!({"name": "echo", "language": "typescript", "source": SOURCE, "policy": {"egress": "namespace"}}), 422),
        create(&alice, json!({"name": "echo", "language": "typescript", "source": SOURCE, "retentionDays": 0}), 422),
    ])
    .await;
    let echo = run.one(create(&dave, plain.clone(), 201)).await;
    let echo_id = echo["id"].as_str().expect("id").to_owned();
    assert_eq!(echo["revision"], 1);
    assert_eq!(echo["policy"]["egress"], "none");
    let spare = run
        .one(create(
            &alice,
            json!({"name": "spare", "language": "python", "source": "def run(input, octopus):\n    return input\n",
                   "inputSchema": {"type": "object"}, "retentionDays": 30}),
            201,
        ))
        .await;
    let spare_id = spare["id"].as_str().expect("id").to_owned();

    let one = |id: &str| format!("/namespaces/{ns}/scripts/{id}");
    run.all(vec![
        create(&alice, plain.clone(), 409),
        scenario("GET", scripts_t, &scripts, vera.clone(), None, 200),
        scenario("GET", scripts_t, &format!("{scripts}?limit=1"), alice.clone(), None, 200),
        scenario("GET", scripts_t, &scripts, wanda.clone(), None, 403),
        scenario("GET", scripts_t, &scripts, bob.clone(), None, 404),
        scenario("GET", script_t, &one(&echo_id), vera.clone(), None, 200),
        scenario("GET", script_t, &one(&echo_id), bob.clone(), None, 404),
        scenario("GET", script_t, &one(MISSING), alice.clone(), None, 404),
        scenario("PATCH", script_t, &one(&echo_id), vera.clone(), Some(json!({"revision": 1, "source": SOURCE})), 403),
        scenario("PATCH", script_t, &one(&echo_id), bob.clone(), Some(json!({"revision": 1})), 404),
        scenario("PATCH", script_t, &one(&echo_id), alice.clone(), Some(json!({"revision": 1, "unknown": true})), 422),
        scenario("PATCH", script_t, &one(&echo_id), alice.clone(), Some(json!({"revision": 1, "outputSchema": {"type": 12}})), 422),
        scenario("PATCH", script_t, &one(&echo_id), alice.clone(), Some(json!({"revision": 1, "source": "export default () => 2\n", "outputSchema": {"type": "object"}})), 200),
        scenario("PATCH", script_t, &one(&echo_id), alice.clone(), Some(json!({"revision": 1, "source": "stale"})), 409),
        scenario("PATCH", script_t, &one(&echo_id), alice.clone(), Some(json!({"revision": 2, "outputSchema": null, "lockfile": null})), 200),
    ])
    .await;

    // -------------------------------------------------------------------------------- versions
    let versions_t = "/namespaces/{namespaceId}/scripts/{scriptId}/versions";
    let version_t = "/namespaces/{namespaceId}/scripts/{scriptId}/versions/{versionNumber}";
    let versions = format!("{}/versions", one(&echo_id));
    let published = run
        .one(scenario(
            "POST",
            versions_t,
            &versions,
            alice.clone(),
            None,
            202,
        ))
        .await;
    assert_eq!(published["version"]["status"], "BUILDING");
    assert_eq!(published["created"], true);
    run.all(vec![
        scenario("POST", versions_t, &versions, vera.clone(), None, 403),
        scenario("POST", versions_t, &versions, bob.clone(), None, 404),
        scenario(
            "POST",
            versions_t,
            &format!("{}/versions", one(MISSING)),
            alice.clone(),
            None,
            404,
        ),
        scenario("POST", versions_t, &versions, alice.clone(), None, 200),
        scenario("GET", versions_t, &versions, vera.clone(), None, 200),
        scenario(
            "GET",
            versions_t,
            &format!("{versions}?before=2&limit=5"),
            alice.clone(),
            None,
            200,
        ),
        scenario("GET", versions_t, &versions, bob.clone(), None, 404),
        scenario(
            "GET",
            version_t,
            &format!("{versions}/1"),
            vera.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            version_t,
            &format!("{versions}/99"),
            vera.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            version_t,
            &format!("{versions}/1"),
            bob.clone(),
            None,
            404,
        ),
    ])
    .await;

    // ---------------------------------------------------------------- the runner builds it
    let claim_t = "/script-runner/builds/claim";
    let heartbeat_t = "/script-runner/builds/{jobId}/heartbeat";
    let complete_t = "/script-runner/builds/{jobId}/complete";
    let claim = |who: &As, expect: u16| {
        scenario(
            "POST",
            claim_t,
            "/script-runner/builds/claim",
            who.clone(),
            Some(
                json!({"toolchains": {"typescript": "node@sha256:aa;shim=1", "python": "python@sha256:bb;shim=1"}}),
            ),
            expect,
        )
    };
    run.all(vec![
        claim(&As::Anonymous, 401),
        claim(&vera, 403),
        claim(&dave, 403),
        scenario(
            "POST",
            claim_t,
            "/script-runner/builds/claim",
            wanda.clone(),
            Some(json!({"toolchains": {"cobol": "x"}})),
            422,
        ),
        scenario(
            "POST",
            claim_t,
            "/script-runner/builds/claim",
            wanda.clone(),
            Some(json!({})),
            422,
        ),
        // Another tenant's runner is offered nothing of ours.
        claim(&bob, 204),
    ])
    .await;
    let job = run.one(claim(&wanda, 200)).await;
    let job_id = job["jobId"].as_str().expect("job id").to_owned();
    let lease = job["leaseToken"].as_str().expect("lease").to_owned();
    assert_eq!(job["language"], "typescript");
    assert!(
        job["policy"]
            .as_str()
            .expect("policy")
            .starts_with("v1;memory=")
    );
    run.all(vec![claim(&wanda, 204)]).await;

    let heartbeat = |id: &str, token: &str, who: &As, expect: u16| {
        scenario(
            "POST",
            heartbeat_t,
            &format!("/script-runner/builds/{id}/heartbeat"),
            who.clone(),
            Some(json!({"leaseToken": token})),
            expect,
        )
    };
    run.all(vec![
        heartbeat(&job_id, &lease, &vera, 403),
        heartbeat(&job_id, &"0".repeat(64), &wanda, 409),
        heartbeat(MISSING, &lease, &wanda, 409),
        heartbeat(&job_id, &lease, &wanda, 200),
        scenario(
            "POST",
            heartbeat_t,
            &format!("/script-runner/builds/{job_id}/heartbeat"),
            wanda.clone(),
            Some(json!({"leaseToken": lease, "x": 1})),
            422,
        ),
    ])
    .await;

    // What a runner reports says what the version says: the policy it was handed and its schemas.
    let bundle = |policy: &Value, code: &str| {
        serde_json::to_vec(&json!({
            "v": 1, "language": "typescript", "policy": policy,
            "inputSchema": job.get("inputSchema").cloned().unwrap_or(Value::Null),
            "outputSchema": job.get("outputSchema").cloned().unwrap_or(Value::Null),
            "entry": "main.mjs", "files": {"main.mjs": code}
        }))
        .expect("json")
    };
    let artifact = bundle(&job["policy"], "export default () => 2");
    // A bundle the version does not describe is not signed (and the runner may report again).
    let wider = bundle(
        &json!(
            "v1;memory=2147483648;cpu=4000;wall=900;pids=512;output=1048576;scratch=536870912;egress=namespace"
        ),
        "export default () => 2",
    );
    // Bodies between axum's default and the contract's limit reach the handler: this one is a
    // bundle of 1.9 MiB (2.5 MiB once encoded), which is not a bundle, and is refused as that.
    let huge = vec![b'{'; 1_900_000];
    let complete = |id: &str, body: Value, who: &As, expect: u16| {
        scenario(
            "POST",
            complete_t,
            &format!("/script-runner/builds/{id}/complete"),
            who.clone(),
            Some(body),
            expect,
        )
    };
    let built = json!({
        "leaseToken": lease, "outcome": "built", "artifact": STANDARD.encode(&artifact),
        "sbom": {"bomFormat": "CycloneDX"}, "toolchain": "node@sha256:aa;shim=1", "log": "built"
    });
    run.all(vec![
        complete(&job_id, built.clone(), &vera, 403),
        complete(
            &job_id,
            json!({"leaseToken": lease, "outcome": "built", "artifact": STANDARD.encode(&wider),
                   "sbom": {"bomFormat": "CycloneDX"}, "toolchain": "node@sha256:aa;shim=1"}),
            &wanda,
            422,
        ),
        complete(
            &job_id,
            json!({"leaseToken": lease, "outcome": "built", "artifact": STANDARD.encode(&huge),
                   "sbom": {"bomFormat": "CycloneDX"}, "toolchain": "node@sha256:aa;shim=1"}),
            &wanda,
            422,
        ),
        complete(
            &job_id,
            json!({"leaseToken": lease, "outcome": "sideways"}),
            &wanda,
            422,
        ),
        complete(
            &job_id,
            json!({"leaseToken": lease, "outcome": "built"}),
            &wanda,
            422,
        ),
        complete(
            &job_id,
            json!({"leaseToken": lease, "outcome": "built", "artifact": "!!!", "toolchain": "t"}),
            &wanda,
            422,
        ),
        complete(
            &job_id,
            json!({"leaseToken": lease, "outcome": "failed", "kind": "other"}),
            &wanda,
            422,
        ),
        complete(
            &job_id,
            json!({"leaseToken": "0".repeat(64), "outcome": "failed", "kind": "rejected"}),
            &wanda,
            409,
        ),
        complete(&job_id, built.clone(), &wanda, 204),
        // The answer was lost and the runner says it again.
        complete(&job_id, built.clone(), &wanda, 204),
    ])
    .await;

    // The version is ready and its artifact can be read by the author and fetched by the runner.
    let ready = run
        .one(scenario(
            "GET",
            version_t,
            &format!("{versions}/1"),
            alice.clone(),
            None,
            200,
        ))
        .await;
    assert_eq!(ready["status"], "READY", "{ready}");
    let digest = ready["artifactDigest"].as_str().expect("digest").to_owned();
    let script_artifact_t = "/script-artifacts/{digest}";
    let runner_artifact_t = "/script-runner/artifacts/{digest}";
    run.all(vec![
        scenario(
            "GET",
            script_artifact_t,
            &format!("/script-artifacts/{digest}"),
            vera.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            script_artifact_t,
            &format!("/script-artifacts/{digest}"),
            wanda.clone(),
            None,
            403,
        ),
        scenario(
            "GET",
            script_artifact_t,
            &format!("/script-artifacts/{digest}"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            script_artifact_t,
            &format!("/script-artifacts/sha256:{}", "ab".repeat(32)),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            runner_artifact_t,
            &format!("/script-runner/artifacts/{digest}"),
            wanda.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            runner_artifact_t,
            &format!("/script-runner/artifacts/{digest}"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            runner_artifact_t,
            &format!("/script-runner/artifacts/{digest}"),
            vera.clone(),
            None,
            403,
        ),
        scenario(
            "GET",
            runner_artifact_t,
            &format!("/script-runner/artifacts/{digest}"),
            dave.clone(),
            None,
            403,
        ),
        scenario(
            "GET",
            runner_artifact_t,
            &format!("/script-runner/artifacts/{digest}"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            runner_artifact_t,
            "/script-runner/artifacts/not-a-digest",
            wanda.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            "/script-runner/signing-key",
            "/script-runner/signing-key",
            wanda.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            "/script-runner/signing-key",
            "/script-runner/signing-key",
            vera.clone(),
            None,
            403,
        ),
    ])
    .await;

    // A failed build, and the policy and pending activities a runner reads.
    let failing = run
        .one(create(
            &alice,
            json!({"name": "failing", "language": "python", "source": "x = 1\n"}),
            201,
        ))
        .await;
    let failing_versions = format!("{}/versions", one(failing["id"].as_str().expect("id")));
    run.all(vec![scenario(
        "POST",
        versions_t,
        &failing_versions,
        alice.clone(),
        None,
        202,
    )])
    .await;
    let job2 = run.one(claim(&wanda, 200)).await;
    let job2_id = job2["jobId"].as_str().expect("job").to_owned();
    run.all(vec![complete(
        &job2_id,
        json!({"leaseToken": job2["leaseToken"], "outcome": "failed", "kind": "rejected",
               "message": "the script must define a function run(input, octopus)", "log": "no run"}),
        &wanda,
        204,
    )])
    .await;
    let failed = run
        .one(scenario(
            "GET",
            version_t,
            &format!("{failing_versions}/1"),
            alice.clone(),
            None,
            200,
        ))
        .await;
    assert_eq!(failed["status"], "FAILED", "{failed}");

    let egress_t = "/script-runner/namespaces/{namespaceId}/egress-policy";
    let pending_t = "/script-runner/namespaces/{namespaceId}/pending-activities";
    run.all(vec![
        scenario(
            "GET",
            egress_t,
            &format!("/script-runner/namespaces/{ns}/egress-policy"),
            wanda.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            egress_t,
            &format!("/script-runner/namespaces/{ns}/egress-policy"),
            vera.clone(),
            None,
            403,
        ),
        scenario(
            "GET",
            egress_t,
            &format!("/script-runner/namespaces/{ns}/egress-policy"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            pending_t,
            &format!("/script-runner/namespaces/{ns}/pending-activities"),
            wanda.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            pending_t,
            &format!("/script-runner/namespaces/{ns}/pending-activities"),
            dave.clone(),
            None,
            403,
        ),
        scenario(
            "GET",
            pending_t,
            &format!("/script-runner/namespaces/{ns}/pending-activities"),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            pending_t,
            &format!("/script-runner/namespaces/{other_ns}/pending-activities"),
            wanda.clone(),
            None,
            404,
        ),
    ])
    .await;

    // -------------------------------------------------------------------------- test runs
    let runs_t = "/namespaces/{namespaceId}/scripts/{scriptId}/test-runs";
    let test_t = "/namespaces/{namespaceId}/scripts/{scriptId}/test-runs/{testRunId}";
    let runs = format!("{}/test-runs", one(&spare_id));
    let queued = run
        .one(scenario(
            "POST",
            runs_t,
            &runs,
            dave.clone(),
            Some(json!({"input": {"n": 1}})),
            202,
        ))
        .await;
    assert_eq!(queued["status"], "QUEUED");
    let test_id = queued["id"].as_str().expect("id").to_owned();
    run.all(vec![
        scenario(
            "POST",
            runs_t,
            &runs,
            vera.clone(),
            Some(json!({"input": {}})),
            403,
        ),
        scenario(
            "POST",
            runs_t,
            &runs,
            bob.clone(),
            Some(json!({"input": {}})),
            404,
        ),
        scenario(
            "POST",
            runs_t,
            &runs,
            alice.clone(),
            Some(json!({"input": {}, "x": 1})),
            422,
        ),
        scenario("GET", runs_t, &runs, alice.clone(), None, 200),
        scenario("GET", runs_t, &runs, vera.clone(), None, 403),
        scenario(
            "GET",
            test_t,
            &format!("{runs}/{test_id}"),
            alice.clone(),
            None,
            200,
        ),
        scenario(
            "GET",
            test_t,
            &format!("{runs}/{MISSING}"),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            test_t,
            &format!("{runs}/{test_id}"),
            bob.clone(),
            None,
            404,
        ),
    ])
    .await;
    let claim_test_t = "/script-runner/test-runs/claim";
    let heartbeat_test_t = "/script-runner/test-runs/{testRunId}/heartbeat";
    let complete_test_t = "/script-runner/test-runs/{testRunId}/complete";
    let claim_test = |who: &As, languages: Value, expect: u16| {
        scenario(
            "POST",
            claim_test_t,
            "/script-runner/test-runs/claim",
            who.clone(),
            Some(json!({"languages": languages})),
            expect,
        )
    };
    run.all(vec![
        claim_test(&vera, json!(["python"]), 403),
        claim_test(&wanda, json!(["cobol"]), 422),
        claim_test(&wanda, json!(["typescript"]), 204),
        claim_test(&bob, json!(["python"]), 204),
    ])
    .await;
    let test_job = run.one(claim_test(&wanda, json!(["python"]), 200)).await;
    let test_lease = test_job["leaseToken"].as_str().expect("lease").to_owned();
    assert_eq!(test_job["input"], json!({"n": 1}));
    assert_eq!(test_job["inputSchema"], json!({"type": "object"}));
    let heartbeat_test = |token: &str, who: &As, expect: u16| {
        scenario(
            "POST",
            heartbeat_test_t,
            &format!("/script-runner/test-runs/{test_id}/heartbeat"),
            who.clone(),
            Some(json!({"leaseToken": token})),
            expect,
        )
    };
    let complete_test = |body: Value, who: &As, expect: u16| {
        scenario(
            "POST",
            complete_test_t,
            &format!("/script-runner/test-runs/{test_id}/complete"),
            who.clone(),
            Some(body),
            expect,
        )
    };
    // A script that returns nothing returns `null`: that is a result, and an absent one is not.
    let finished = json!({
        "leaseToken": test_lease, "result": null, "log": "hello\n", "denied": ["fetch http://169.254.169.254"],
        "exitCode": 0, "wallMs": 120, "startMs": 80, "memoryPeakBytes": 30000000,
        "inferredInputSchema": {"type": "object"}, "inferredOutputSchema": {"type": "object"}
    });
    run.all(vec![
        heartbeat_test(&test_lease, &vera, 403),
        heartbeat_test(&"0".repeat(64), &wanda, 409),
        heartbeat_test(&test_lease, &wanda, 200),
        complete_test(finished.clone(), &vera, 403),
        complete_test(json!({"leaseToken": test_lease}), &wanda, 422),
        complete_test(
            json!({"leaseToken": test_lease, "result": 1, "failure": "SCRIPT_ERROR"}),
            &wanda,
            422,
        ),
        complete_test(
            json!({"leaseToken": test_lease, "failure": "NOT_A_CODE"}),
            &wanda,
            422,
        ),
        complete_test(
            json!({"leaseToken": "0".repeat(64), "result": 1}),
            &wanda,
            409,
        ),
        complete_test(finished.clone(), &wanda, 204),
        complete_test(finished, &wanda, 204),
    ])
    .await;
    let done = run
        .one(scenario(
            "GET",
            test_t,
            &format!("{runs}/{test_id}"),
            alice.clone(),
            None,
            200,
        ))
        .await;
    assert_eq!(done["status"], "SUCCEEDED", "{done}");
    assert!(done["result"].is_null(), "{done}");
    assert_eq!(done["denied"], json!(["fetch http://169.254.169.254"]));

    // ------------------------------------------------------------------------------ executions
    let record_t = "/script-runner/executions";
    let executions_t = "/namespaces/{namespaceId}/script-executions";
    let execution = |who: &As, body: Value, expect: u16| {
        scenario(
            "POST",
            record_t,
            "/script-runner/executions",
            who.clone(),
            Some(body),
            expect,
        )
    };
    let report = json!({
        "namespaceId": ns, "digest": digest, "runId": MISSING, "node": "s", "attempt": 1,
        "status": "FAILED", "failure": "SCRIPT_TIMEOUT", "log": "…", "denied": ["secret payments-key"],
        "wallMs": 3000, "startMs": 90
    });
    run.all(vec![
        execution(&vera, report.clone(), 403),
        execution(&wanda, json!({"namespaceId": ns, "digest": "nope", "status": "FAILED"}), 422),
        execution(&wanda, json!({"namespaceId": ns, "digest": digest, "status": "SIDEWAYS"}), 422),
        execution(&wanda, json!({"namespaceId": ns, "digest": digest, "status": "FAILED", "failure": "boom"}), 422),
        execution(&wanda, json!({"namespaceId": ns, "digest": format!("sha256:{}", "cd".repeat(32)), "status": "SUCCEEDED"}), 404),
        execution(&wanda, json!({"namespaceId": other_ns, "digest": digest, "status": "SUCCEEDED"}), 404),
        execution(&wanda, report.clone(), 201),
        execution(&wanda, report, 201),
        scenario("GET", executions_t, &format!("/namespaces/{ns}/script-executions"), vera.clone(), None, 200),
        scenario("GET", executions_t, &format!("/namespaces/{ns}/script-executions?digest={digest}&limit=5"), alice.clone(), None, 200),
        scenario("GET", executions_t, &format!("/namespaces/{ns}/script-executions?digest=nope"), alice.clone(), None, 422),
        scenario("GET", executions_t, &format!("/namespaces/{ns}/script-executions?before=nope"), alice.clone(), None, 400),
        scenario("GET", executions_t, &format!("/namespaces/{ns}/script-executions"), wanda.clone(), None, 403),
        scenario("GET", executions_t, &format!("/namespaces/{ns}/script-executions"), bob.clone(), None, 404),
    ])
    .await;
    let listed = run
        .one(scenario(
            "GET",
            executions_t,
            &format!("/namespaces/{ns}/script-executions"),
            alice.clone(),
            None,
            200,
        ))
        .await;
    let execution_id = listed["items"][0]["id"]
        .as_str()
        .expect("an execution")
        .to_owned();
    assert!(
        listed["items"][0].get("log").is_none(),
        "a listing has no logs: {listed}"
    );
    let execution_t = "/namespaces/{namespaceId}/script-executions/{executionId}";
    let one_execution = |id: &str| format!("/namespaces/{ns}/script-executions/{id}");
    let full = run
        .one(scenario(
            "GET",
            execution_t,
            &one_execution(&execution_id),
            vera.clone(),
            None,
            200,
        ))
        .await;
    assert_eq!(full["log"], "…");
    run.all(vec![
        scenario(
            "GET",
            execution_t,
            &one_execution(&execution_id),
            wanda.clone(),
            None,
            403,
        ),
        scenario(
            "GET",
            execution_t,
            &one_execution(&execution_id),
            bob.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            execution_t,
            &one_execution(MISSING),
            alice.clone(),
            None,
            404,
        ),
        scenario(
            "GET",
            execution_t,
            &one_execution("not-an-id"),
            alice.clone(),
            None,
            404,
        ),
    ])
    .await;

    // ----------------------------------------------------------------------------------- delete
    run.all(vec![
        scenario("DELETE", script_t, &one(&echo_id), alice.clone(), None, 409),
        scenario("DELETE", script_t, &one(&spare_id), vera.clone(), None, 403),
        scenario("DELETE", script_t, &one(&spare_id), bob.clone(), None, 404),
        scenario(
            "DELETE",
            script_t,
            &one(&spare_id),
            alice.clone(),
            None,
            204,
        ),
        scenario(
            "DELETE",
            script_t,
            &one(&spare_id),
            alice.clone(),
            None,
            404,
        ),
    ])
    .await;

    // Every operation marked as implemented by this feature was exercised.
    let marked: BTreeSet<_> = spec
        .operations_marked("x-implemented", "005")
        .into_iter()
        .collect();
    assert_eq!(marked.len(), 25, "the contract marks the script operations");
    let untested: Vec<_> = marked.difference(&run.covered).collect();
    assert!(
        untested.is_empty(),
        "operations without a conformance scenario: {untested:?}"
    );
}
