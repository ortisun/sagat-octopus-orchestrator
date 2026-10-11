//! The manifest: what is accepted, what is refused and where it is said.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use sagat_cli::manifest::{self, Manifest, is_slug, stays_below};

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "sagat-cli-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("dirs");
        }
        std::fs::write(&path, text).expect("write");
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const DOC: &str = r#"{"metadata":{"name":"checkout"},"kind":"workflow"}"#;

fn manifest_text(entries: &str) -> String {
    format!(
        "apiVersion: sagat-octopus-orchestrator.io/v1\nkind: Manifest\nnamespace: billing-dev\nworkflows:\n{entries}"
    )
}

fn problems_of(m: &Dir, name: &str) -> Vec<String> {
    manifest::load(&m.0.join(name))
        .expect_err("refused")
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn slugs_are_what_the_platform_accepts() {
    for ok in ["abc", "checkout", "billing-dev", "a1b", "a-b-c"] {
        assert!(is_slug(ok), "{ok}");
    }
    for bad in [
        "",
        "ab",
        "Checkout",
        "1abc",
        "a--b",
        "abc-",
        "-abc",
        "a_b",
        "a b",
        "ç-abc",
        &"a".repeat(64),
    ] {
        assert!(!is_slug(bad), "{bad:?}");
    }
}

#[test]
fn a_file_stays_below_the_manifest() {
    for ok in ["a.json", "workflows/a.json", "./a.json", "a/b/c.yaml"] {
        assert!(stays_below(ok), "{ok}");
    }
    for bad in [
        "",
        "../a.json",
        "a/../../b.json",
        "/etc/passwd",
        "workflows/../../x",
    ] {
        assert!(!stays_below(bad), "{bad:?}");
    }
}

#[test]
fn a_manifest_is_read_with_the_documents_it_lists_as_json_or_yaml() {
    let dir = Dir::new();
    dir.write("workflows/checkout.json", DOC);
    dir.write(
        "workflows/refund.yaml",
        "metadata:\n  name: refund\nkind: workflow\n",
    );
    dir.write(
        "sagat.yaml",
        &manifest_text(
            "  - name: checkout\n    file: workflows/checkout.json\n    releaseNotes: first\n  - name: refund\n    file: workflows/refund.yaml\n",
        ),
    );
    let (manifest, loaded) = manifest::load(&dir.0.join("sagat.yaml")).expect("loads");
    assert_eq!(manifest.namespace, "billing-dev");
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].entry.release_notes.as_deref(), Some("first"));
    assert_eq!(loaded[1].value["metadata"]["name"], "refund");
    // A YAML document is sent as JSON.
    assert!(serde_json::from_slice::<serde_json::Value>(&loaded[1].document).is_ok());

    let json = Manifest::parse(
        r#"{"apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Manifest","namespace":"billing-dev","workflows":[]}"#,
        false,
    )
    .expect("json manifest");
    assert!(json.problems().is_empty());
}

#[test]
fn every_problem_of_a_manifest_is_reported_at_once() {
    let dir = Dir::new();
    dir.write(
        "bad.yaml",
        "apiVersion: example.com/v9\nkind: Other\nnamespace: X\nworkflows:\n  - name: Checkout\n    file: ../outside.json\n  - name: dup-name\n    file: a.json\n  - name: dup-name\n    file: a.json\n",
    );
    let problems = problems_of(&dir, "bad.yaml").join("\n");
    for expect in [
        "apiVersion: this tool reads",
        "kind: must be Manifest",
        "namespace: a namespace slug",
        "workflows[0].name",
        "workflows[0].file: a relative path below",
        "workflows[2].name: this name is listed twice",
        "workflows[2].file: two workflows cannot be the same file",
    ] {
        assert!(problems.contains(expect), "{expect} in\n{problems}");
    }
}

#[test]
fn unknown_fields_are_refused_rather_than_ignored() {
    let dir = Dir::new();
    dir.write(
        "m.yaml",
        &manifest_text("  - name: checkout\n    file: a.json\n    secrets: [x]\n"),
    );
    let problems = problems_of(&dir, "m.yaml").join("\n");
    assert!(problems.contains("unknown field `secrets`"), "{problems}");
}

#[test]
fn a_document_must_be_the_workflow_the_manifest_names() {
    let dir = Dir::new();
    dir.write("a.json", r#"{"metadata":{"name":"refund"}}"#);
    dir.write(
        "m.yaml",
        &manifest_text("  - name: checkout\n    file: a.json\n"),
    );
    let problems = problems_of(&dir, "m.yaml").join("\n");
    assert!(
        problems.contains("the document is named 'refund' and the manifest lists it as 'checkout'"),
        "{problems}"
    );
    dir.write("array.json", "[1,2]");
    dir.write(
        "m2.yaml",
        &manifest_text("  - name: checkout\n    file: array.json\n"),
    );
    assert!(
        problems_of(&dir, "m2.yaml")
            .join("\n")
            .contains("is an object")
    );
    dir.write("garbage.json", "{not json");
    dir.write(
        "m3.yaml",
        &manifest_text("  - name: checkout\n    file: garbage.json\n"),
    );
    assert!(!problems_of(&dir, "m3.yaml").is_empty());
}

#[test]
fn a_file_larger_than_the_platform_takes_is_refused_before_it_is_read() {
    let dir = Dir::new();
    let big = format!(
        "{{\"metadata\":{{\"name\":\"checkout\"}},\"pad\":\"{}\"}}",
        "x".repeat(usize::try_from(manifest::MAX_BYTES).expect("fits"))
    );
    dir.write("big.json", &big);
    dir.write(
        "m.yaml",
        &manifest_text("  - name: checkout\n    file: big.json\n"),
    );
    assert!(
        problems_of(&dir, "m.yaml")
            .join("\n")
            .contains("is larger than")
    );
}

#[cfg(unix)]
#[test]
fn a_link_cannot_make_the_tool_read_a_file_outside_the_manifest_directory() {
    let outside = Dir::new();
    outside.write("secret.json", DOC);
    let dir = Dir::new();
    std::fs::create_dir_all(dir.0.join("workflows")).expect("dir");
    std::os::unix::fs::symlink(
        outside.0.join("secret.json"),
        dir.0.join("workflows/checkout.json"),
    )
    .expect("link");
    dir.write(
        "m.yaml",
        &manifest_text("  - name: checkout\n    file: workflows/checkout.json\n"),
    );
    let problems = problems_of(&dir, "m.yaml").join("\n");
    assert!(
        problems.contains("leads outside the directory of the manifest"),
        "{problems}"
    );
    assert!(Path::new(&outside.0).join("secret.json").exists());
}

#[test]
fn a_manifest_that_does_not_exist_is_said_so() {
    let dir = Dir::new();
    let problems = problems_of(&dir, "nowhere.yaml").join("\n");
    assert!(problems.contains("nowhere.yaml"), "{problems}");
}
