//! A step an isolated runner executes is *managed*, and the worker paths ask for that through one predicate,
//! `Node::managed_kind` (feature 035, D035-130 A, D035-134). Asking `database_operation().is_some()` instead would
//! name only the SQL kind: a lake step would be leased to a worker as an ordinary activity, carrying the whole run
//! input, and a lake runner's own heartbeat, completion and blob read would be refused. This source-text check is the
//! fitness function of that rule (it is not a call-graph proof): it fails when a worker path adds the narrow question.
#![allow(clippy::expect_used, clippy::panic)]

use std::{
    fs,
    path::{Path, PathBuf},
};

/// `worker.rs` and every file of `worker/`: the claim, lease, report, scoped and heartbeat paths.
fn worker_paths() -> Vec<PathBuf> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![src.join("worker.rs")];
    let mut stack = vec![src.join("worker")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("the worker directory") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// The narrow question: whether a node has a SQL operation, asked only to learn whether it is managed.
fn asks_only_about_sql(text: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains("database_operation().is_some()")
        || compact.contains("database_operation().is_none()")
}

/// The file without its `mod tests`, which may build SQL nodes on purpose. Other `#[cfg(test)]` items (a helper for the
/// tests) do not end the production code: `worker.rs` has one long before its managed paths.
fn production(text: &str) -> &str {
    text.split("#[cfg(test)]\nmod tests")
        .next()
        .unwrap_or_default()
}

#[test]
fn the_worker_paths_ask_whether_a_step_is_managed_through_its_kind_not_through_its_sql_operation() {
    let mut offenders = Vec::new();
    for path in worker_paths() {
        let text = fs::read_to_string(&path).expect("a worker source");
        if asks_only_about_sql(production(&text)) {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "ask `managed_kind().is_some()` (or match the kind), not `database_operation().is_some()`: {offenders:#?}"
    );
}

#[test]
fn the_check_reads_all_the_production_code_of_a_file_up_to_its_test_module() {
    let text = "fn a() {}\n#[cfg(test)]\nfn helper() {}\nfn b() { n.database_operation().is_some(); }\n#[cfg(test)]\nmod tests { fn t() { n.database_operation().is_some(); } }";
    assert!(
        asks_only_about_sql(production(text)),
        "a test helper must not hide what follows it"
    );
    assert!(
        !asks_only_about_sql(production(
            "fn a() {}\n#[cfg(test)]\nmod tests { fn t() { n.database_operation().is_some(); } }"
        )),
        "the test module may build SQL nodes"
    );
}

#[test]
fn the_check_sees_the_narrow_question_however_it_is_laid_out() {
    assert!(asks_only_about_sql("node.database_operation().is_some()"));
    assert!(asks_only_about_sql(
        "node\n    .database_operation()\n    .is_none()"
    ));
    assert!(!asks_only_about_sql("node.managed_kind().is_some()"));
    assert!(
        !asks_only_about_sql(
            "n.and_then(Node::database_operation).is_some_and(|op| op.kind == Execute)"
        ),
        "reading the SQL operation for what only SQL has (a write) is not the question"
    );
}
