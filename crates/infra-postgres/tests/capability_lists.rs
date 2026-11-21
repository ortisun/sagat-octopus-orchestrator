//! The capability lists of the instance-capability registry's checks follow the code (feature 006, decision D006-033 of
//! `specs/006-sdk-multilanguage/research.md`; ADR-0098 decision 8). Two branches redefine the same three checks, each with
//! its own values; a fresh database ends with the newest definition by version, so that one must list exactly
//! `Capability::ALL`. This test runs on every pull request that changes this crate or the application crate, unlike the
//! same rule in `scripts/check-architecture.py`, which CI runs at a release.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::Path;

use sagat_application::instances::Capability;

const CHECKS: [&str; 3] = [
    "serving_instances_capabilities_known",
    "platform_requirements_capability_known",
    "platform_requirement_history_capability_known",
];

/// The quoted values of the newest `ADD CONSTRAINT <check> CHECK (...)` among the up migrations, with its file name.
fn newest(check: &str) -> Option<(String, BTreeSet<String>)> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut files: Vec<_> = std::fs::read_dir(&folder)
        .expect("the migrations folder")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.to_string_lossy().ends_with(".up.sql"))
        .collect();
    files.sort();
    let needle = format!("ADD CONSTRAINT {check} CHECK (");
    let mut found = None;
    for file in files {
        let text = std::fs::read_to_string(&file).expect("a migration");
        let mut from = 0;
        while let Some(at) = text[from..].find(&needle) {
            let start = from + at + needle.len();
            let mut depth = 1usize;
            let mut end = text.len();
            for (offset, c) in text[start..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                if depth == 0 {
                    end = start + offset;
                    break;
                }
            }
            let values = text[start..end]
                .split('\'')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect();
            let name = file
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .into_owned();
            found = Some((name, values));
            from = end;
        }
    }
    found
}

#[test]
fn capability_lists_follow_the_code() {
    let code: BTreeSet<String> = Capability::ALL
        .iter()
        .map(|capability| capability.as_str().to_owned())
        .collect();
    for check in CHECKS {
        let (file, listed) = newest(check).unwrap_or_else(|| {
            panic!("no up migration defines {check}: the rule would check nothing")
        });
        assert_eq!(
            listed, code,
            "{check}, last defined by {file}, must list exactly `Capability::ALL`: the branch that merges second takes \
             the union of both lists in its own, newest migration (decision D006-033)"
        );
    }
}
