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

// ---- The same rule, written out of the worker paths (feature 035, D035-150, slice S0).
//
// A managed step is a SQL statement **or** an Iceberg write (and, with D035-150, an Iceberg probe). Six places stated that by hand,
// `database_operation().is_some() || iceberg_write().is_some()`: a third kind would have been created with no connection binding and
// claimed for ever, with nothing failing at compile time. They ask `managed_kind()` now, and this check keeps the by-hand form out of
// the three crates that decide admission, storage and publication. A question about **one** kind (the SQL-only ad hoc path, the
// write-only egress check) is a different question and is not flagged: only the two joined are.

/// Every `.rs` file under `crates/<name>/src`, in a stable order.
fn sources_of(name: &str) -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(name)
        .join("src");
    let mut files = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("a source directory") {
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

/// Whether the SQL question and the write question are asked within a few characters of each other, in either order and
/// however the source is laid out: the managed predicate stated by hand.
fn restates_the_managed_predicate(text: &str) -> bool {
    const SQL: [&str; 2] = [
        "database_operation().is_some()",
        "database_operation().is_none()",
    ];
    const LAKE: [&str; 2] = ["iceberg_write().is_some()", "iceberg_write().is_none()"];
    const NEAR: usize = 60;
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = compact.as_bytes();
    SQL.iter().any(|sql| {
        compact.match_indices(sql).any(|(at, _)| {
            let window = String::from_utf8_lossy(
                &bytes[at.saturating_sub(NEAR)..(at + sql.len() + NEAR).min(bytes.len())],
            )
            .into_owned();
            LAKE.iter().any(|lake| window.contains(lake))
        })
    })
}

#[test]
fn no_production_code_states_the_managed_predicate_by_hand() {
    let mut offenders = Vec::new();
    for name in ["application", "infra-postgres", "infra-ir"] {
        for path in sources_of(name) {
            let text = fs::read_to_string(&path).expect("a source");
            if restates_the_managed_predicate(production(&text)) {
                offenders.push(path.display().to_string());
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "ask `managed_kind().is_some()`, not the two payload questions joined: {offenders:#?}"
    );
}

#[test]
fn the_check_for_the_hand_written_predicate_sees_it_however_it_is_laid_out_and_nothing_else() {
    assert!(restates_the_managed_predicate(
        "n.database_operation().is_some() || n.iceberg_write().is_some()"
    ));
    assert!(restates_the_managed_predicate(
        "node\n  .iceberg_write()\n  .is_some()\n  || node\n  .database_operation()\n  .is_some()"
    ));
    assert!(restates_the_managed_predicate(
        "a.database_operation().is_none() && a.iceberg_write().is_none()"
    ));
    assert!(
        !restates_the_managed_predicate("definition.nodes().any(|n| n.iceberg_write().is_some())"),
        "a question about the write alone is the write's question"
    );
    assert!(
        !restates_the_managed_predicate("nodes.iter().any(|n| n.database_operation().is_some())"),
        "a question about SQL alone is the SQL path's question"
    );
    let far = "x".repeat(200);
    assert!(
        !restates_the_managed_predicate(&format!(
            "n.database_operation().is_some(){far}n.iceberg_write().is_some()"
        )),
        "two unrelated questions far apart are not the predicate"
    );
}
