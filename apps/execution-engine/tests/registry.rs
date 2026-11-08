//! The engine and the instance-capability registry (`T018-080`, `FR-018-052`): the capabilities it announces follow the
//! passes its configuration runs, and every loop it spawns obeys the gate, so that a replica that may not serve runs no
//! pass, reads no connector and keeps no shard. The second is a structural check over `src/lib.rs`: it walks the calls of
//! `tasks.spawn_loop(` with their parentheses balanced, so a loop added later that forgets the gate fails it, and text
//! elsewhere in the function cannot satisfy it.

#![allow(clippy::expect_used, clippy::panic)]

use sagat_application::instances::Capability;
use sagat_control_plane::config::Environment;
use sagat_execution_engine::EngineConfig;
use sagat_execution_engine::capabilities::capabilities;

fn config(extra: &[(&str, &str)]) -> EngineConfig {
    let mut values = vec![
        ("SAGAT_OIDC_ISSUER", "https://idp.example/"),
        ("SAGAT_OIDC_AUDIENCE", "sagat"),
        ("SAGAT_OIDC_JWKS_URI", "https://idp.example/jwks.json"),
    ];
    values.extend_from_slice(extra);
    let lookup = move |name: &str| {
        values
            .iter()
            .rev()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_owned())
    };
    EngineConfig::from_lookup(&lookup, Environment::Dev).expect("a configuration")
}

#[test]
fn the_default_engine_runs_no_schedule_pass_and_lists_no_capability() {
    assert_eq!(capabilities(&config(&[])), Vec::<Capability>::new());
}

#[test]
fn an_engine_lists_schedules_v1_when_and_only_when_its_configuration_runs_the_schedule_pass() {
    for (flag, listed) in [
        ("true", true),
        ("1", true),
        ("yes", true),
        ("false", false),
        ("0", false),
        ("no", false),
        ("", false),
    ] {
        let list = capabilities(&config(&[("SAGAT_SCHEDULES_ENABLED", flag)]));
        assert_eq!(
            list,
            if listed {
                vec![Capability::SchedulesV1]
            } else {
                Vec::new()
            },
            "SAGAT_SCHEDULES_ENABLED={flag:?}"
        );
    }
}

#[test]
fn the_other_passes_do_not_make_an_engine_list_the_capability_of_the_schedule_pass() {
    let list = capabilities(&config(&[
        ("SAGAT_BATCHES_ENABLED", "true"),
        ("SAGAT_RESET_CANCEL_SOURCE_ENABLED", "true"),
    ]));
    assert!(list.is_empty(), "{list:?}");
    let all = capabilities(&config(&[
        ("SAGAT_SCHEDULES_ENABLED", "true"),
        ("SAGAT_BATCHES_ENABLED", "true"),
        ("SAGAT_RESET_CANCEL_SOURCE_ENABLED", "true"),
    ]));
    assert_eq!(all, vec![Capability::SchedulesV1]);
}

#[test]
fn what_an_engine_lists_is_sorted_without_repeats_and_only_values_the_registry_knows() {
    for extra in [
        &[][..],
        &[("SAGAT_SCHEDULES_ENABLED", "true")][..],
        &[
            ("SAGAT_SCHEDULES_ENABLED", "true"),
            ("SAGAT_BATCHES_ENABLED", "true"),
        ][..],
    ] {
        let list = capabilities(&config(extra));
        for capability in &list {
            assert!(Capability::ALL.contains(capability));
        }
        let mut sorted = list.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted, list);
    }
}

// --- the loops --------------------------------------------------------------------------------------------------------------

/// The text between the parentheses of every call of `needle` (which ends with its opening parenthesis), with the
/// parentheses balanced; string literals, characters and comments are skipped over, so a parenthesis in one of them
/// cannot end a call early.
fn calls(source: &str, needle: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = source[from..].find(needle) {
        let start = from + at;
        let open = start + needle.len();
        // a mention in a line comment is not a call
        let line_start = source[..start].rfind('\n').map_or(0, |n| n + 1);
        if source[line_start..start].contains("//") {
            from = open;
            continue;
        }
        let mut depth = 1;
        let mut i = open;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'"' => {
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'"' {
                        i += if bytes[i] == b'\\' { 2 } else { 1 };
                    }
                }
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    i += 2;
                    while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                        i += 1;
                    }
                    i += 1;
                }
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        assert_eq!(depth, 0, "the call at byte {open} is never closed");
        found.push(source[open..i - 1].to_owned());
        from = i;
    }
    found
}

fn first_string(text: &str) -> String {
    text.split('"').nth(1).expect("a name").to_owned()
}

#[test]
fn the_scanner_balances_parentheses_and_ignores_the_ones_in_strings_and_comments() {
    let source = r#"
        tasks.spawn_loop("a (b", f(g(")")), // a ) in a comment
            h()); tasks.spawn_loop("c", x);
        // tasks.spawn_loop("not a call", y)
        other();
    "#;
    let found = calls(source, "tasks.spawn_loop(");
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(first_string(&found[0]), "a (b");
    assert!(found[0].contains("h()"), "{found:?}");
    assert_eq!(first_string(&found[1]), "c");
}

#[test]
fn every_loop_the_engine_spawns_obeys_the_gate_and_the_set_of_loops_is_the_one_listed() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("the engine's source");
    let loops = calls(&source, "tasks.spawn_loop(");
    let names: Vec<String> = loops.iter().map(|call| first_string(call)).collect();
    // a loop that is added is added here, on purpose, and obeys the gate in one of the ways below
    let mut expected = vec![
        "instance heartbeat",
        "shard keeper",
        "scheduler",
        "housekeeping",
        "schedule pass",
        "batch pass",
        "reset pass",
        "event dispatcher",
        "telemetry projector",
        "operational monitor",
        "connector supervisor",
        "delivery worker",
        "outbox dispatcher",
        "blob cleanup",
    ];
    let mut found: Vec<&str> = names.iter().map(String::as_str).collect();
    expected.sort_unstable();
    found.sort_unstable();
    assert_eq!(found, expected, "the loops the engine spawns changed");

    for call in &loops {
        let name = first_string(call);
        match name.as_str() {
            // the registry's own loop: it is what decides the gate
            "instance heartbeat" => {}
            // the keeper gives its shards back and takes none while the gate is closed
            "shard keeper" => assert!(
                call.contains("gate.is_open()"),
                "the keeper does not look at the gate"
            ),
            // the supervisor of the connectors winds its consumers down: it was given the gate where it is built
            "connector supervisor" => {
                let start = source
                    .find("let connectors = ")
                    .expect("the supervisor is built");
                let end = source[start..].find(";\n").expect("a statement") + start;
                assert!(
                    source[start..end].contains(".with_serving_gate("),
                    "the supervisor of the connectors was not given the gate"
                );
            }
            other => assert!(
                call.contains("while_serving("),
                "the loop `{other}` runs its rounds outside the gate: wrap them in `loops::while_serving` (FR-018-052)"
            ),
        }
    }
    // the request surfaces: the worker API obeys the gate in its admission layer
    assert!(
        source.contains("WorkerGrpc::new(state, workers).with_serving_gate("),
        "the worker API was not given the gate"
    );
}
