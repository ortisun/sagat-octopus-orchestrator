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
fn the_default_engine_runs_no_schedule_pass_and_lists_only_run_input_validation() {
    // design 043: `enforce` is the default, and an engine in `enforce` promises `run-input-validation-v1`
    assert_eq!(
        capabilities(&config(&[])),
        vec![Capability::RunInputValidationV1]
    );
}

/// A file of digest keys for the configuration tests, owner-readable only.
fn key_file(text: &str) -> tempfile::NamedTempFile {
    use std::io::Write as _;
    let mut file = tempfile::NamedTempFile::new().expect("a file");
    file.write_all(text.as_bytes()).expect("written");
    file
}

const KEY: &str = "3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c";

/// D022-057, D022-062, T022-072: an engine lists both gates, `flow-control-concurrency-v1` and `flow-control-rate-v1`,
/// when it holds the deployment's digest keys, its switches on or off (a draining engine runs the gates for what was
/// admitted), and nothing without the keys.
#[test]
fn an_engine_lists_both_gates_with_its_keys_whatever_its_switches() {
    let keys = key_file(&format!("1:{KEY}\n"));
    let path = keys.path().to_str().expect("a path").to_owned();
    let listed = capabilities(&config(&[
        ("SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED", "true"),
        ("SAGAT_FLOW_CONTROL_RATE_LIMITS_ENABLED", "true"),
        ("SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE", &path),
    ]));
    assert_eq!(
        listed,
        vec![
            Capability::RunInputValidationV1,
            Capability::FlowControlConcurrencyV1,
            Capability::FlowControlRateV1
        ]
    );
    // keys without the switches: the gates are run for what was admitted (a draining engine, D022-062), so listed
    assert_eq!(
        capabilities(&config(&[("SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE", &path)])),
        vec![
            Capability::RunInputValidationV1,
            Capability::FlowControlConcurrencyV1,
            Capability::FlowControlRateV1
        ]
    );
    // no keys and no switch: nothing listed (a switch without the keys does not start)
    assert_eq!(
        capabilities(&config(&[])),
        vec![Capability::RunInputValidationV1]
    );
    let engine = config(&[
        ("SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED", "true"),
        ("SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE", &path),
    ]);
    // what a configuration dump would print names the versions, never the key
    let printed = format!("{:?}", engine.flow_control_keys);
    assert!(
        printed.contains("versions") && !printed.contains(KEY) && !printed.contains("3c3c"),
        "{printed}"
    );
}

/// D022-057: a switch on without the digest keys, or a file that is not a ring of keys, stops the engine naming the
/// variable and never the file's content.
#[test]
fn a_switch_without_usable_digest_keys_stops_the_engine_naming_the_variable_only() {
    let refuse = |values: &[(&str, &str)]| {
        let mut all = vec![
            ("SAGAT_OIDC_ISSUER", "https://idp.example/"),
            ("SAGAT_OIDC_AUDIENCE", "sagat"),
            ("SAGAT_OIDC_JWKS_URI", "https://idp.example/jwks.json"),
        ];
        all.extend_from_slice(values);
        let lookup = move |name: &str| {
            all.iter()
                .rev()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        };
        match EngineConfig::from_lookup(&lookup, Environment::Dev) {
            Ok(_) => panic!("refused"),
            Err(error) => error.to_string(),
        }
    };
    for switch in [
        "SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED",
        "SAGAT_FLOW_CONTROL_RATE_LIMITS_ENABLED",
    ] {
        let why = refuse(&[(switch, "true")]);
        assert!(why.contains("SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE"), "{why}");
    }
    let secret_looking = format!("1:{}", &KEY[..62]);
    let short = key_file(&secret_looking);
    let twice = key_file(&format!("1:{KEY},1:{KEY}"));
    let empty = key_file(" \n");
    let large = key_file(&format!("1:{KEY}{}", " ".repeat(16 * 1024)));
    for (file, what) in [
        (&short, "a short key"),
        (&twice, "a version twice"),
        (&empty, "no key"),
        (&large, "too large"),
    ] {
        let path = file.path().to_str().expect("a path").to_owned();
        let why = refuse(&[
            ("SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED", "true"),
            ("SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE", &path),
        ]);
        assert!(
            why.contains("SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE"),
            "{what}: {why}"
        );
        assert!(
            !why.contains("3c3c"),
            "{what}: the error names no key: {why}"
        );
    }
    let why = refuse(&[
        ("SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED", "true"),
        (
            "SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE",
            "/nonexistent/flow-control.keys",
        ),
    ]);
    assert!(why.contains("readable file"), "{why}");
}

/// FR-022-031 (re-review of 34b8be41, Major 2; re-review of 39157608, Major 1): an engine with both switches off that is
/// given the digest keys holds them, so that what was admitted before the switch went off drains (its waiters granted,
/// its gated steps decided), and lists the gate so that a fleet whose activation requires it keeps it; it admits no new
/// gated start (`CAPABILITY_DISABLED`). The file is read as strictly as with a switch on; without a file it holds nothing.
#[test]
fn an_engine_with_its_switches_off_holds_a_given_key_file_to_drain_and_lists_the_gate() {
    let keys = key_file(&format!("1:{KEY}\n"));
    let path = keys.path().to_str().expect("a path").to_owned();
    let draining = config(&[("SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE", &path)]);
    assert!(
        draining.flow_control_keys.is_some(),
        "the ring is held with every switch off"
    );
    assert!(!draining.flow_control.concurrency_keys && !draining.flow_control.rate_limits);
    assert_eq!(
        capabilities(&draining),
        vec![
            Capability::RunInputValidationV1,
            Capability::FlowControlConcurrencyV1,
            Capability::FlowControlRateV1
        ]
    );
    assert!(config(&[]).flow_control_keys.is_none());
    let empty = key_file(" \n");
    let lookup = {
        let path = empty.path().to_str().expect("a path").to_owned();
        move |name: &str| match name {
            "SAGAT_OIDC_ISSUER" => Some("https://idp.example/".to_owned()),
            "SAGAT_OIDC_AUDIENCE" => Some("sagat".to_owned()),
            "SAGAT_OIDC_JWKS_URI" => Some("https://idp.example/jwks.json".to_owned()),
            "SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE" => Some(path.clone()),
            _ => None,
        }
    };
    let why = match EngineConfig::from_lookup(&lookup, Environment::Dev) {
        Ok(_) => panic!("a file that is not a ring is refused with the switches off too"),
        Err(error) => error.to_string(),
    };
    assert!(why.contains("SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE"), "{why}");
}

/// FR-043-013: the capability is listed only in `enforce`, so a replica in `report` is refused once it is required.
#[test]
fn an_engine_in_report_mode_does_not_list_run_input_validation() {
    for (mode, listed) in [("enforce", true), ("", true), ("report", false)] {
        let list = capabilities(&config(&[
            ("SAGAT_SCHEDULES_ENABLED", "true"),
            ("SAGAT_RUN_INPUT_VALIDATION", mode),
        ]));
        let mut expected = vec![Capability::SchedulesV1];
        if listed {
            expected.push(Capability::RunInputValidationV1);
        }
        assert_eq!(list, expected, "SAGAT_RUN_INPUT_VALIDATION={mode:?}");
    }
}

/// The variable is parsed strictly: anything but `enforce`, `report` or empty stops the engine naming it.
#[test]
fn the_run_input_validation_mode_of_the_engine_is_read_strictly() {
    for wrong in ["Report", "ENFORCE", " report", "off", "true"] {
        let values = [
            ("SAGAT_OIDC_ISSUER", "https://idp.example/"),
            ("SAGAT_OIDC_AUDIENCE", "sagat"),
            ("SAGAT_OIDC_JWKS_URI", "https://idp.example/jwks.json"),
            ("SAGAT_RUN_INPUT_VALIDATION", wrong),
        ];
        let lookup = move |name: &str| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        };
        let Err(error) = EngineConfig::from_lookup(&lookup, Environment::Dev) else {
            panic!("{wrong:?} was accepted");
        };
        assert!(
            error.to_string().contains("SAGAT_RUN_INPUT_VALIDATION"),
            "{wrong:?}: {error}"
        );
    }
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
                vec![Capability::SchedulesV1, Capability::RunInputValidationV1]
            } else {
                vec![Capability::RunInputValidationV1]
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
    assert_eq!(list, vec![Capability::RunInputValidationV1], "{list:?}");
    let all = capabilities(&config(&[
        ("SAGAT_SCHEDULES_ENABLED", "true"),
        ("SAGAT_BATCHES_ENABLED", "true"),
        ("SAGAT_RESET_CANCEL_SOURCE_ENABLED", "true"),
    ]));
    assert_eq!(
        all,
        vec![Capability::SchedulesV1, Capability::RunInputValidationV1]
    );
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
        "trigger schedules",
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
