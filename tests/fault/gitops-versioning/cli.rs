//! The command line interrupted, repeated and run twice at once, and the tenant boundary seen from
//! it.

use reqwest::Method;
use sagat_cli::Env;
use sagat_domain::identity::Role;
use serde_json::{Value, json};

use crate::support::worker::{platform, single_ir};

const NS: &str = "payments";

struct Dir(std::path::PathBuf);

impl Dir {
    fn new() -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "sagat-fault-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(path.join("workflows")).expect("dirs");
        Self(path)
    }

    fn manifest(&self, names: &[&str]) {
        let mut text = format!(
            "apiVersion: sagat-octopus-orchestrator.io/v1\nkind: Manifest\nnamespace: {NS}\nworkflows:\n"
        );
        for name in names {
            text.push_str(&format!(
                "  - name: {name}\n    file: workflows/{name}.json\n"
            ));
            let doc = single_ir(name, None);
            std::fs::write(
                self.0.join(format!("workflows/{name}.json")),
                serde_json::to_string(&doc).expect("json"),
            )
            .expect("write");
        }
        std::fs::write(self.0.join("sagat.yaml"), text).expect("manifest");
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn run(
    p: &crate::support::worker::Platform,
    dir: &Dir,
    sub: &str,
    tenant: &str,
    args: &[&str],
) -> (u8, String) {
    let env = Env {
        server: Some(p.api.base.clone()),
        token: Some(p.api.idp.token(sub, tenant).build()),
        namespace: None,
        cwd: dir.0.clone(),
    };
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = sagat_cli::run(&args, &env, &mut out, &mut err).await;
    (
        code,
        format!(
            "{}{}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        ),
    )
}

async fn workflows(p: &crate::support::worker::Platform) -> Vec<Value> {
    let (status, page) = p.rest(Method::GET, "/workflows?limit=100", None).await;
    assert_eq!(status, 200, "{page}");
    page["items"].as_array().expect("items").clone()
}

#[tokio::test]
async fn a_publish_that_was_interrupted_half_way_is_completed_by_running_it_again() {
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    let dir = Dir::new();
    dir.manifest(&["checkout", "refund", "invoice"]);

    // A previous run died after it created `checkout` and published `refund` but before the rest.
    let (status, _) = p
        .rest(
            Method::POST,
            "/workflows",
            Some(&json!({"name": "checkout", "ir": single_ir("checkout", None)})),
        )
        .await;
    assert_eq!(status, 201);
    p.publish("refund", &single_ir("refund", None)).await;

    let (code, said) = run(&p, &dir, "dev1", "acme", &["publish"]).await;
    assert_eq!(code, 0, "{said}");
    assert!(
        said.contains("published  checkout v1") && said.contains("created   invoice"),
        "{said}"
    );
    assert!(
        said.contains("3 published")
            || said.contains("2 published, 0 waiting for approval, 1 unchanged"),
        "{said}"
    );
    let all = workflows(&p).await;
    assert_eq!(all.len(), 3);
    assert!(
        all.iter().all(|w| w["activeVersion"]["versionNumber"] == 1),
        "{all:?}"
    );
    let (code, said) = run(&p, &dir, "dev1", "acme", &["diff", "--check"]).await;
    assert_eq!(code, 0, "{said}");
}

#[tokio::test]
async fn two_pipelines_publishing_the_same_manifest_at_once_leave_one_workflow_and_one_version() {
    let p = platform().await;
    p.grant(p.tenant, "dev1", Role::Developer).await;
    p.grant(p.tenant, "dev2", Role::Developer).await;
    let dir = Dir::new();
    dir.manifest(&["checkout", "refund"]);

    let (a, b) = futures_util::future::join(
        run(&p, &dir, "dev1", "acme", &["publish"]),
        run(&p, &dir, "dev2", "acme", &["publish"]),
    )
    .await;
    assert!(
        a.0 == 0 || b.0 == 0,
        "at least one pipeline finished the job:\n{}\n{}",
        a.1,
        b.1
    );
    for (code, said) in [&a, &b] {
        assert!(*code == 0 || *code == 1, "{said}");
        assert!(!said.contains("panicked"), "{said}");
    }
    // Whatever each said, running it once more converges, and nothing was made twice.
    let (code, said) = run(&p, &dir, "dev1", "acme", &["publish"]).await;
    assert_eq!(code, 0, "{said}");
    let all = workflows(&p).await;
    assert_eq!(all.len(), 2, "{all:?}");
    for workflow in &all {
        let id = workflow["id"].as_str().expect("id");
        let (_, versions) = p
            .rest(Method::GET, &format!("/workflows/{id}/versions"), None)
            .await;
        assert_eq!(
            versions["items"].as_array().expect("items").len(),
            1,
            "one version of {}",
            workflow["name"]
        );
    }
}

#[tokio::test]
async fn the_same_publication_sent_again_with_its_key_is_the_same_publication() {
    let p = platform().await;
    let (_, created) = p
        .rest(
            Method::POST,
            "/workflows",
            Some(&json!({"name": "checkout", "ir": single_ir("checkout", None)})),
        )
        .await;
    let id = created["id"].as_str().expect("id").to_owned();
    let send = || async {
        let response = p
            .api
            .as_user(
                Method::POST,
                &format!("/workflows/{id}/publish"),
                "alice",
                "acme",
            )
            .header("x-octopus-namespace", NS)
            .header("idempotency-key", "lost-answer-0123456789")
            .json(&json!({"draftRevision": 1}))
            .send()
            .await
            .expect("send");
        (
            response.status().as_u16(),
            crate::support::api::json_of(response).await,
        )
    };
    let (first, second) = (send().await, send().await);
    assert_eq!(first.0, 201);
    assert_eq!(second.0, 201);
    assert_eq!(
        first.1["versionId"], second.1["versionId"],
        "the answer that was lost is given again"
    );
    let (_, versions) = p
        .rest(Method::GET, &format!("/workflows/{id}/versions"), None)
        .await;
    assert_eq!(versions["items"].as_array().expect("items").len(), 1);
}

#[tokio::test]
async fn from_the_command_line_another_tenants_namespaces_and_workflows_do_not_exist() {
    let p = platform().await;
    let dir = Dir::new();
    dir.manifest(&["checkout"]);
    let (code, said) = run(&p, &dir, "alice", "acme", &["publish"]).await;
    assert_eq!(code, 0, "{said}");

    // Bob administers another tenant. He names acme's namespace and acme's workflow; to him
    // neither exists, and the answer is the same as for names that were never used.
    let (code, real) = run(&p, &dir, "bob", "globex", &["diff"]).await;
    assert_eq!(code, 1, "{real}");
    let never = Dir::new();
    std::fs::write(
        never.0.join("sagat.yaml"),
        "apiVersion: sagat-octopus-orchestrator.io/v1\nkind: Manifest\nnamespace: never-used\nworkflows: []\n",
    )
    .expect("manifest");
    let (code, imaginary) = run(&p, &never, "bob", "globex", &["diff"]).await;
    assert_eq!(code, 1, "{imaginary}");
    // The trace id closes the line and differs between calls: what the person is told is the rest.
    let strip = |text: &str| {
        let line = text.lines().next().unwrap_or("");
        line.rsplit_once(' ')
            .map_or(line, |(told, _trace)| told)
            .to_owned()
    };
    assert_eq!(
        strip(&real),
        strip(&imaginary),
        "an existing namespace of another tenant looks like one that does not exist"
    );
    let (code, said) = run(
        &p,
        &dir,
        "bob",
        "globex",
        &["history", "--workflow", "checkout", "--namespace", NS],
    )
    .await;
    assert_eq!(code, 1, "{said}");
    let (code, said) = run(
        &p,
        &dir,
        "alice",
        "acme",
        &[
            "promote",
            "--workflow",
            "checkout",
            "--to",
            "globex-secret",
            "--reason",
            "into somebody else's namespace",
        ],
    )
    .await;
    assert_eq!(code, 1, "{said}");
    assert!(
        said.contains("the tenant has no namespace 'globex-secret'"),
        "{said}"
    );
}
