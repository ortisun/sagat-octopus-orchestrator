//! The capability lists of the instance-capability registry's checks follow the code (feature 006, decision D006-033 of
//! `specs/006-sdk-multilanguage/research.md`; ADR-0098 decision 8). Two branches redefine the same three checks, each with
//! its own values; a fresh database ends with the newest definition by version, so that one must list exactly
//! `Capability::ALL`, and each down migration must restore the list before its up (decision D006-083). These tests run on
//! every pull request that changes this crate or the application crate, unlike the same rules in
//! `scripts/check-architecture.py`, which CI runs at a release.
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::Path;

use sagat_application::instances::Capability;

const CHECKS: [&str; 3] = [
    "serving_instances_capabilities_known",
    "platform_requirements_capability_known",
    "platform_requirement_history_capability_known",
];

/// The migrations of this crate whose names end with `suffix`, in version order.
fn migrations(suffix: &str) -> Vec<std::path::PathBuf> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut files: Vec<_> = std::fs::read_dir(&folder)
        .expect("the migrations folder")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.to_string_lossy().ends_with(suffix))
        .collect();
    files.sort();
    files
}

fn name_of(file: &Path) -> String {
    file.file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned()
}

/// The quoted values of the last `CONSTRAINT <check> CHECK (...)` of `text` (inline in a `CREATE TABLE` or after
/// `ADD`), if it defines the check.
fn defined(text: &str, check: &str) -> Option<BTreeSet<String>> {
    let needle = format!("CONSTRAINT {check} CHECK (");
    let mut found = None;
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
        found = Some(
            text[start..end]
                .split('\'')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect(),
        );
        from = end;
    }
    found
}

/// The values of the newest definition of `check` among the up migrations, with its file name.
fn newest(check: &str) -> Option<(String, BTreeSet<String>)> {
    let mut found = None;
    for file in migrations(".up.sql") {
        let text = std::fs::read_to_string(&file).expect("a migration");
        if let Some(values) = defined(&text, check) {
            found = Some((name_of(&file), values));
        }
    }
    found
}

/// Decision D006-083: every up migration that redefines `check` has a down that restores exactly the list of the newest
/// up before it (or, for the first definition, defines none), so a rollback never erases another feature's values.
fn down_errors(check: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let mut before: Option<BTreeSet<String>> = None;
    for up in migrations(".up.sql") {
        let text = std::fs::read_to_string(&up).expect("a migration");
        let Some(values) = defined(&text, check) else {
            continue;
        };
        let name = name_of(&up);
        let down = up.with_file_name(name.replace(".up.sql", ".down.sql"));
        let restored = std::fs::read_to_string(&down)
            .ok()
            .and_then(|text| defined(&text, check));
        if restored != before {
            errors.push(format!(
                "{}: restores {restored:?} of {check}, but the list before {name} is {before:?}",
                name_of(&down)
            ));
        }
        before = Some(values);
    }
    errors
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

#[test]
fn each_down_migration_restores_the_capability_list_before_its_up() {
    for check in CHECKS {
        let errors = down_errors(check);
        assert!(
            errors.is_empty(),
            "a rollback must restore the list in force before its migration, never an older or another one (decision \
             D006-083): {errors:#?}"
        );
    }
}
