//! Every event kind design 044 adds has a reset rule in feature 018's registry (Architecture decision 0050 decision 8;
//! design 044, Technical Plan 5.1 and 6.3), with the rule for a code run.

use std::collections::BTreeMap;

use crate::repository_root;

/// The new kinds of design 044, Technical Plan 5.1.
const NEW_KINDS: [&str; 14] = [
    "workflow_task.started",
    "workflow_task.completed",
    "workflow_task.failed",
    "workflow_task.timed_out",
    "command.noop",
    "signal.consumed",
    "run.cancel_requested",
    "run.continue_requested",
    "update.admitted",
    "update.accepted",
    "update.rejected",
    "update.completed",
    "marker.recorded",
    "run.code_version_moved",
];

const RULES: [&str; 4] = ["COPY", "REMATERIALIZE", "EXCLUDE", "REFUSE"];

/// The rows of the section that design 044 owns: each event type named in the first column, with its code-run rule.
fn code_run_rules() -> BTreeMap<String, String> {
    let path = repository_root()
        .join("specs/018-schedules-bulk-operations/contracts/journal-and-events.md");
    let text = std::fs::read_to_string(&path).expect("the registry exists");
    let section = text
        .split("### Event types added by design 044")
        .nth(1)
        .expect("the registry has design 044's section")
        .split("\n### ")
        .next()
        .expect("a section")
        .split("\nDrafts ")
        .next()
        .expect("the table");
    let mut rules = BTreeMap::new();
    for row in section.lines().filter(|line| line.starts_with("| `")) {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        // ["", event types, graph run, code run, notes, ""]
        assert!(cells.len() >= 5, "a row of four columns: {row}");
        let code_rule = cells[3];
        assert!(
            RULES.iter().any(|rule| code_rule.contains(rule)),
            "the code-run rule of `{}` is one of {RULES:?}: {code_rule}",
            cells[1]
        );
        for kind in cells[1].split('`').skip(1).step_by(2) {
            rules.insert(kind.to_owned(), code_rule.to_owned());
        }
    }
    rules
}

#[test]
fn every_new_event_kind_has_a_reset_rule() {
    let rules = code_run_rules();
    for kind in NEW_KINDS {
        assert!(
            rules.contains_key(kind),
            "`{kind}` has no reset rule for a code run"
        );
    }
    // The rules the design states (Technical Plan 6.3).
    assert_eq!(rules["run.continue_requested"], "`EXCLUDE`");
    assert_eq!(rules["timer.started"], "`REMATERIALIZE`");
    assert_eq!(rules["wait.started"], "`REMATERIALIZE`");
    for kind in [
        "workflow_task.started",
        "signal.consumed",
        "run.code_version_moved",
        "update.completed",
    ] {
        assert_eq!(rules[kind], "`COPY`", "{kind}");
    }
}
