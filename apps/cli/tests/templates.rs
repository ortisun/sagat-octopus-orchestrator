//! The templates `sagat new` writes: each is a document the platform's codec accepts, with the
//! answers to serve it and a README, and nothing in them is a secret or somebody's address.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use sagat_cli::diagnostics::of_document;
use sagat_cli::mocks::{Mocks, activities_of};
use sagat_cli::new::{TEMPLATES, find};
use serde_json::Value;

fn dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "sagat-templates-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

fn run(args: &[&str], cwd: &std::path::Path) -> (u8, String, String) {
    let env = sagat_cli::Env {
        cwd: cwd.to_path_buf(),
        ..Default::default()
    };
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let code = runtime.block_on(sagat_cli::run(&args, &env, &mut out, &mut err));
    (
        code,
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

#[test]
fn every_template_is_a_document_the_platform_accepts() {
    assert_eq!(
        TEMPLATES.iter().map(|t| t.name).collect::<Vec<_>>(),
        [
            "hello-world",
            "event-trigger",
            "cron",
            "approval",
            "compensation",
            "inline-script"
        ]
    );
    for template in TEMPLATES {
        let workflow = template
            .files
            .iter()
            .find(|f| f.name == "workflow.json")
            .expect("a workflow");
        let found = of_document(workflow.text.as_bytes());
        assert!(
            found.iter().all(|d| d.warning),
            "{}: {found:?}",
            template.name
        );
        let value: Value = serde_json::from_str(workflow.text).expect("json");
        assert_eq!(
            value["metadata"]["name"],
            *template
                .name
                .replace("compensation", "payment-saga")
                .replace("event-trigger", "on-order-created")
                .replace("cron", "nightly-report")
                .replace("approval", "change-approval")
                .replace("inline-script", "enrich-order")
                .as_str(),
            "{}",
            template.name
        );
    }
}

#[test]
fn every_template_has_answers_that_serve_every_activity_of_its_workflow() {
    for template in TEMPLATES {
        let file = |name: &str| {
            template
                .files
                .iter()
                .find(|f| f.name == name)
                .map(|f| f.text)
        };
        let workflow: Value =
            serde_json::from_str(file("workflow.json").expect("workflow")).expect("json");
        let mocks = Mocks::parse(
            file("mocks.yaml").unwrap_or_else(|| panic!("{} has no mocks.yaml", template.name)),
            true,
        )
        .unwrap_or_else(|p| panic!("{}: {p:?}", template.name));
        let served = mocks.names();
        for activity in activities_of(&workflow) {
            assert!(
                served.contains(&activity),
                "{}: no answer for {activity}",
                template.name
            );
        }
        assert!(
            file("README.md").is_some_and(|r| r.starts_with("# ")),
            "{} has no README",
            template.name
        );
    }
}

#[test]
fn no_template_holds_a_secret_a_token_or_an_address_that_is_somebody_s() {
    for template in TEMPLATES {
        for file in template.files {
            let text = file.text.to_lowercase();
            for needle in [
                "bearer ey",
                "password",
                "secret:",
                "api_key",
                "ghp_",
                "sk-live",
                "@gmail",
                "amazonaws",
            ] {
                assert!(
                    !text.contains(needle),
                    "{}/{} holds '{needle}'",
                    template.name,
                    file.name
                );
            }
        }
    }
}

#[test]
fn new_writes_the_files_and_will_not_write_over_them_without_being_told() {
    let cwd = dir();
    let (code, out, _) = run(&["new", "hello-world"], &cwd);
    assert_eq!(code, 0, "{out}");
    for name in ["workflow.json", "mocks.yaml", "README.md"] {
        assert!(cwd.join("hello-world").join(name).is_file(), "{name}");
    }
    assert!(out.contains("next: read"), "{out}");
    // A second time nothing is written, and nothing is half written.
    std::fs::write(cwd.join("hello-world/mocks.yaml"), "mine").expect("write");
    let (code, _, err) = run(&["new", "hello-world"], &cwd);
    assert_eq!(code, 1);
    assert!(err.contains("exists (--force writes over it)"), "{err}");
    assert_eq!(
        std::fs::read_to_string(cwd.join("hello-world/mocks.yaml")).expect("read"),
        "mine"
    );
    // With --force it does.
    let (code, _, _) = run(&["new", "hello-world", "--force"], &cwd);
    assert_eq!(code, 0);
    assert!(
        std::fs::read_to_string(cwd.join("hello-world/mocks.yaml"))
            .expect("read")
            .contains("greeting.compose")
    );
    // Into a named directory.
    let (code, _, _) = run(&["new", "cron", "reports/nightly"], &cwd);
    assert_eq!(code, 0);
    assert!(cwd.join("reports/nightly/workflow.json").is_file());
}

#[cfg(unix)]
#[test]
fn a_link_in_the_way_is_never_followed_even_with_force() {
    let cwd = dir();
    let victim = cwd.join("victim.txt");
    std::fs::write(&victim, "precious").expect("write");
    std::fs::create_dir_all(cwd.join("hello-world")).expect("dir");
    std::os::unix::fs::symlink(&victim, cwd.join("hello-world/workflow.json")).expect("link");
    for args in [
        &["new", "hello-world"][..],
        &["new", "hello-world", "--force"][..],
    ] {
        let (code, _, err) = run(args, &cwd);
        assert_eq!(code, 1, "{err}");
        assert_eq!(std::fs::read_to_string(&victim).expect("read"), "precious");
    }
}

#[test]
fn an_unknown_template_names_the_ones_there_are_and_the_list_shows_them_all() {
    let cwd = dir();
    let (code, _, err) = run(&["new", "nonsense"], &cwd);
    assert_eq!(code, 1);
    assert!(
        err.contains("'nonsense' is not a template") && err.contains("compensation"),
        "{err}"
    );
    let (code, out, _) = run(&["new", "--list"], &cwd);
    assert_eq!(code, 0);
    for template in TEMPLATES {
        assert!(out.contains(template.name), "{out}");
    }
    assert!(find("approval").is_some() && find("nonsense").is_none());
}
