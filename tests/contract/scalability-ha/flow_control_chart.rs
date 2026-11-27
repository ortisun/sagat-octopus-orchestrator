//! The chart's flow-control switches (feature 022, gap G022-036): each gate has its own value, both the control plane
//! and the engine get the same switches, and a switch on without the digest key Secret fails the render, as the
//! processes would refuse to start (D022-057).

use super::*;
use serde_json::json;

const CONTROL: &str = "sagat-octopus-orchestrator-control-plane";
const ENGINE: &str = "sagat-octopus-orchestrator-engine";
const RATE: &str = "SAGAT_FLOW_CONTROL_RATE_LIMITS_ENABLED";
const CONCURRENCY: &str = "SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED";
const KEYS_FILE: &str = "SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE";

fn flow_control(values: &Value) -> Value {
    with(&json!({ "flowControl": values }))
}

#[test]
fn the_rate_switch_turns_the_rate_gate_on_in_both_processes_with_the_keys() {
    let docs = render(&flow_control(
        &json!({"rateLimits": true, "digestKeys": {"existingSecret": "flow-keys"}}),
    ));
    for name in [CONTROL, ENGINE] {
        let env = env_of(named(&docs, "Deployment", name));
        assert_eq!(env.get(RATE).map(String::as_str), Some("true"), "{name}");
        assert!(
            !env.contains_key(CONCURRENCY),
            "{name}: the rate switch alone turns on the rate gate only"
        );
        assert_eq!(
            env.get(KEYS_FILE).map(String::as_str),
            Some("/etc/sagat/flow-control/flow-control.keys"),
            "{name}"
        );
    }
}

#[test]
fn the_two_switches_are_independent_and_both_off_by_default() {
    let docs = render(&base());
    for name in [CONTROL, ENGINE] {
        let env = env_of(named(&docs, "Deployment", name));
        assert!(
            !env.contains_key(RATE) && !env.contains_key(CONCURRENCY),
            "{name}: off by default"
        );
    }
    let docs = render(&flow_control(&json!({
        "concurrencyKeys": true, "rateLimits": true, "digestKeys": {"existingSecret": "flow-keys"}
    })));
    for name in [CONTROL, ENGINE] {
        let env = env_of(named(&docs, "Deployment", name));
        assert_eq!(env.get(RATE).map(String::as_str), Some("true"), "{name}");
        assert_eq!(
            env.get(CONCURRENCY).map(String::as_str),
            Some("true"),
            "{name}"
        );
    }
}

#[test]
fn a_switch_without_the_digest_keys_fails_the_render() {
    for switch in ["concurrencyKeys", "rateLimits"] {
        let why = helm(&flow_control(&json!({ switch: true }))).expect_err(switch);
        assert!(
            why.contains(&format!(
                "flowControl.{switch} needs flowControl.digestKeys.existingSecret"
            )),
            "{switch}: {why}"
        );
    }
}

/// Feature 022's switches beside feature 018's automatic handover (ADR-0091), which the base values turn on: both
/// processes carry the two gates and the engine the handover flag, and the handover's Job, which runs no gate, is given
/// neither a flow-control switch nor the digest key Secret.
#[test]
fn the_flow_control_switches_and_the_automatic_handover_render_together() {
    let docs = render(&flow_control(&json!({
        "concurrencyKeys": true, "rateLimits": true, "digestKeys": {"existingSecret": "flow-keys"}
    })));
    for name in [CONTROL, ENGINE] {
        let env = env_of(named(&docs, "Deployment", name));
        assert_eq!(env.get(RATE).map(String::as_str), Some("true"), "{name}");
        assert_eq!(
            env.get(CONCURRENCY).map(String::as_str),
            Some("true"),
            "{name}"
        );
    }
    let engine = env_of(named(&docs, "Deployment", ENGINE));
    assert_eq!(
        engine
            .get("SAGAT_SCHEDULES_AUTO_HANDOVER")
            .map(String::as_str),
        Some("true")
    );
    let jobs = of_kind(&docs, "Job");
    assert_eq!(
        jobs.len(),
        1,
        "the handover's Job is rendered beside the gates"
    );
    let job = serde_json::to_string(jobs[0]).expect("json");
    assert!(
        !job.contains("SAGAT_FLOW_CONTROL")
            && !job.contains("flow-keys")
            && !job.contains("flow-control"),
        "the Job runs no gate: {job}"
    );
}
