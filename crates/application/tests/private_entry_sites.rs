//! Who may use a private entry of the codec (ADR-0080, D035-150 slice S2a).
//!
//! A node that only the platform may write is parsed by a private twin of the codec's entry points, and nothing public may reach one:
//! not publication, not the API, not a worker, not a scheduler. The rule is a fixed list of the files that mention one: the codec that
//! defines them, the cache that offers them, the start path that builds the platform's own probe document, and the run store that restores
//! a private version because its workflow is internal. A file that is not on the list and mentions one fails this test, and adding it
//! to the list is a decision a reviewer sees. This source-text check is the fitness function of that rule (it is not a call-graph proof).
#![allow(clippy::expect_used, clippy::panic)]

use std::{
    fs,
    path::{Path, PathBuf},
};

/// The names of the private entries, on the codec, the cache and the port.
const ENTRIES: [&str; 5] = [
    "parse_private",
    "parse_stored_private",
    "parse_private_document",
    "get_or_parse_private",
    "declared_state_publication_private",
];

/// The production files that may mention one, by their path from `crates/` or `apps/`.
const ALLOWED: [&str; 5] = [
    // the port that defines them and the cache that offers them
    "application/src/definitions.rs",
    "application/src/execution.rs",
    // the start path that builds the platform's own probe document and parses it
    "application/src/runs/ad_hoc.rs",
    // the run store: a private version is restored by its workflow's `internal` flag
    "infra-postgres/src/execution.rs",
    // the codec that implements them
    "infra-ir/src/lib.rs",
];

fn sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for top in ["crates", "apps"] {
        let mut stack = vec![root.join(top)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries {
                let path = entry.expect("a directory entry").path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if path.is_dir() {
                    // build output, dependencies and tests are not production code
                    if !matches!(
                        name,
                        "target" | "node_modules" | "tests" | "vendor" | ".git"
                    ) {
                        stack.push(path);
                    }
                } else if path.extension().is_some_and(|e| e == "rs")
                    && path
                        .components()
                        .any(|component| component.as_os_str() == "src")
                {
                    files.push(path);
                }
            }
        }
    }
    files.sort();
    files
}

/// The file without its `mod tests`, which may call anything it likes.
fn production(text: &str) -> &str {
    text.split("#[cfg(test)]\nmod tests")
        .next()
        .unwrap_or_default()
}

fn mentions_a_private_entry(text: &str) -> bool {
    ENTRIES.iter().any(|entry| text.contains(entry))
}

fn relative(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    ["/crates/", "/apps/"]
        .iter()
        .find_map(|marker| {
            text.rfind(marker)
                .map(|at| text[at + marker.len()..].to_owned())
        })
        .unwrap_or(text)
}

#[test]
fn only_the_listed_files_mention_a_private_entry() {
    let mut strangers = Vec::new();
    let mut seen = Vec::new();
    for path in sources() {
        let text = fs::read_to_string(&path).expect("a source");
        if mentions_a_private_entry(production(&text)) {
            let name = relative(&path);
            if !ALLOWED.contains(&name.as_str()) {
                strangers.push(name.clone());
            }
            seen.push(name);
        }
    }
    assert!(
        strangers.is_empty(),
        "a private entry of the codec is reached from {strangers:#?}: nothing public may reach one (ADR-0080); if this is the platform's own \
         path, add it to ALLOWED with the reason"
    );
    // The list is not stale: every file on it still says what it is on the list for.
    for allowed in ALLOWED {
        assert!(
            seen.iter().any(|name| name == allowed),
            "{allowed} is on the list and no longer mentions a private entry: take it off"
        );
    }
}

#[test]
fn the_check_sees_an_entry_by_any_of_its_names_and_nothing_else() {
    assert!(mentions_a_private_entry("codec.parse_private(doc)"));
    assert!(mentions_a_private_entry("cache.get_or_parse_private(v, d)"));
    assert!(mentions_a_private_entry(
        "x.declared_state_publication_private(d)"
    ));
    assert!(!mentions_a_private_entry(
        "codec.parse(doc); cache.get_or_parse(v, d)"
    ));
    assert!(
        !mentions_a_private_entry("let private = true; // a private workflow"),
        "the word is not the entry"
    );
    assert!(
        !production("fn a() {}\n#[cfg(test)]\nmod tests { fn t() { x.parse_private(d); } }")
            .contains("parse_private")
    );
}
