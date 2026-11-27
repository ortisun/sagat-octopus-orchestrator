//! The golden corpus of design 044 (Task 044-30, `specs/044-workflow-as-code/contracts/goldens/`). `generate.mjs`
//! writes it; this module recomputes it without that script: a test-local RFC 8785 serializer over `serde_json` (with
//! ECMAScript's number printing), SHA-256 from `sha2`, a test-local xoshiro256**, and the body of each command kind
//! as `README.md` defines it. Every history line must be a `HistoryEvent` of worker.proto in the proto3 JSON mapping,
//! and `node generate.mjs --check` must reproduce the checked-in files (`AGENTS.md` rule 18: a golden changes only
//! after its diff has been read).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::num::FpCategory;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::{read_json, repository_root, worker_proto};

const GOLDENS: &str = "specs/044-workflow-as-code/contracts/goldens";

/// The conformance scenarios of design 044's Verification Plan, one directory each.
const SCENARIOS: [&str; 22] = [
    "code-run-lifecycle",
    "code-deterministic-primitives",
    "code-signals",
    "code-signal-at-return",
    "code-updates",
    "code-update-with-start",
    "code-declared-state",
    "code-worker-queries",
    "code-graph-mixed",
    "code-continue-as-new",
    "code-saga",
    "code-saga-uncertain",
    "code-gated-activity",
    "code-cancellation",
    "code-failures",
    "code-nondeterminism",
    "code-patch-and-move",
    "code-cache-eviction",
    "code-activation-deadline",
    "code-history-paging",
    "code-command-batch-limit",
    "code-blob-values",
];

/// The member of `HistoryEvent.body` each kind carries.
const BODY_OF_KIND: [(&str, &str); 14] = [
    ("HISTORY_RUN_STARTED", "runStarted"),
    ("HISTORY_WORKFLOW_TASK_STARTED", "workflowTaskStarted"),
    ("HISTORY_WORKFLOW_TASK_COMPLETED", "workflowTaskCompleted"),
    ("HISTORY_COMMAND_RECORDED", "commandRecord"),
    ("HISTORY_STEP_OUTCOME", "stepOutcome"),
    ("HISTORY_TIMER_FIRED", "timerFired"),
    ("HISTORY_WAIT_OUTCOME", "waitOutcome"),
    ("HISTORY_CHILD_FINISHED", "childFinished"),
    ("HISTORY_APPROVAL_DECIDED", "approvalDecided"),
    ("HISTORY_SAGA_OUTCOME", "sagaOutcome"),
    ("HISTORY_SIGNAL_BUFFERED", "signalBuffered"),
    ("HISTORY_UPDATE_ADMITTED", "updateAdmitted"),
    ("HISTORY_CANCELLATION_REQUESTED", "cancellationRequested"),
    ("HISTORY_MARKER_RECORDED", "markerRecorded"),
];

/// The code-profile definition of `workflow.schema.json` for each work command's fragment.
const PROFILES: [(&str, &str); 6] = [
    ("activity", "codeProfileActivity"),
    ("script", "codeProfileScript"),
    ("timer_start", "codeProfileTimer"),
    ("wait_event", "codeProfileWaitEvent"),
    ("child", "codeProfileChild"),
    ("approval", "codeProfileApproval"),
];

type Enums = BTreeMap<String, BTreeSet<String>>;
type Profiles = BTreeMap<&'static str, jsonschema::Validator>;

#[test]
fn the_goldens_parse_and_their_chains_recompute() {
    let root = repository_root().join(GOLDENS);
    numbers_parse_exactly();
    rfc8785_vectors(&root);
    let primitives = primitives_vectors(&root);
    let profiles = profiles();
    chain_vectors(&root, &profiles);
    let enums = proto_enums();
    // The command kinds whose steps the corpus shows cancelled through a step outcome (Decision 044-245).
    let mut cancelled: BTreeSet<String> = BTreeSet::new();

    let mut found: Vec<PathBuf> = std::fs::read_dir(root.join("scenarios"))
        .expect("the scenarios directory")
        .map(|entry| entry.expect("a scenario entry").path())
        .filter(|path| path.is_dir())
        .collect();
    found.sort();
    let names: BTreeSet<String> = found.iter().map(|path| file_name(path)).collect();
    let expected: BTreeSet<String> = SCENARIOS.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(
        names, expected,
        "one directory per conformance scenario of the Verification Plan"
    );

    for dir in &found {
        assert!(
            dir.join("function.md").is_file(),
            "{} has its function.md",
            dir.display()
        );
        if dir.join("PENDING").is_file() {
            assert_eq!(
                file_name(dir),
                "code-worker-queries",
                "only the worker queries wait for their task"
            );
            continue;
        }
        let mut runs = vec![dir.clone()];
        let mut subdirs: Vec<PathBuf> = std::fs::read_dir(dir)
            .expect("a scenario directory")
            .map(|entry| entry.expect("an entry").path())
            .filter(|path| path.join("commands.json").is_file())
            .collect();
        subdirs.sort();
        runs.extend(subdirs);
        for run in &runs {
            check_run(run, &enums, &profiles, &mut cancelled);
        }
        if file_name(dir) == "code-deterministic-primitives" {
            the_primitives_scenario_draws_the_vector_stream(dir, &primitives);
        }
    }

    for kind in ["activity", "wait_event", "approval"] {
        assert!(
            cancelled.contains(kind),
            "the corpus shows a cancelled {kind} as a STEP_OUTCOME_CANCELLED (Decision 044-245)"
        );
    }

    the_generator_reproduces_the_corpus();
}

// ---------------------------------------------------------------------------------------------------------------
// RFC 8785

fn canonical(value: &Value) -> Vec<u8> {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out.into_bytes()
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => {
            out.push_str(&es_number(
                number.as_f64().expect("a JSON number is a double"),
            ));
        }
        Value::String(text) => quote(text, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            out.push('{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                quote(key, out);
                out.push(':');
                write_canonical(item, out);
            }
            out.push('}');
        }
    }
}

fn quote(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                write!(out, "\\u{:04x}", u32::from(c)).expect("a String accepts writes");
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The digits and decimal exponent of `{:e}` (or `{:.N$e}`) output.
fn split_scientific(scientific: &str) -> (Vec<u8>, i32) {
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("scientific notation has an exponent");
    let digits = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|digit| digit - b'0')
        .collect();
    (digits, exponent.parse().expect("a decimal exponent"))
}

/// The `count`-digit decimal closest to `magnitude` whose last digit has the place of the shortest form's last digit
/// (`10^(exponent - count + 1)`), a tie going to the even digit, as ECMA-262 asks when several shortest forms
/// round-trip. Rust's shortest digits do not always make that choice (1424953923781206.25 prints ...206.3, ECMAScript
/// ...206.2), so the closest one is taken from the exact expansion (a double has at most 767 significant digits).
fn closest_digits(magnitude: f64, count: usize, exponent: i32) -> Option<(Vec<u8>, i32)> {
    let (exact, exact_exponent) = split_scientific(&format!("{magnitude:.800e}"));
    let keep = usize::try_from(exact_exponent - exponent + i32::try_from(count).ok()?).ok()?;
    let (head, rest) = exact.split_at(keep.min(exact.len()));
    let mut digits = head.to_vec();
    let mut leading = exact_exponent;
    let first = rest.first().copied().unwrap_or(0);
    let exact_half = first == 5 && rest.iter().skip(1).all(|digit| *digit == 0);
    let odd = digits.last().is_some_and(|digit| digit & 1 == 1);
    if first > 5 || (first == 5 && !exact_half) || (exact_half && odd) {
        let mut carry = true;
        for digit in digits.iter_mut().rev() {
            if *digit == 9 {
                *digit = 0;
            } else {
                *digit += 1;
                carry = false;
                break;
            }
        }
        if carry {
            digits.insert(0, 1);
            leading += 1;
        }
    }
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }
    let (first_digit, others) = digits.split_first()?;
    let others: String = others
        .iter()
        .map(|digit| char::from(b'0' + digit))
        .collect();
    let round_trips = format!("{first_digit}.{others}e{leading}")
        .parse::<f64>()
        .ok()?
        .to_bits()
        == magnitude.to_bits();
    round_trips.then_some((digits, leading))
}

/// ECMAScript's Number-to-String for a finite double: the shortest digits that round-trip, the closest of them,
/// placed by the exponent rules of ECMA-262 (Number::toString), so -0 prints 0 and 1e21 prints 1e+21.
fn es_number(x: f64) -> String {
    assert!(x.is_finite(), "RFC 8785 has no form for {x}");
    if x.classify() == FpCategory::Zero {
        return "0".to_owned();
    }
    let sign = if x.is_sign_negative() { "-" } else { "" };
    let magnitude = x.abs();
    let (shortest, shortest_exponent) = split_scientific(&format!("{magnitude:e}"));
    let (digits, exponent) = closest_digits(magnitude, shortest.len(), shortest_exponent)
        .unwrap_or((shortest, shortest_exponent));
    let digits: String = digits
        .iter()
        .map(|digit| char::from(b'0' + digit))
        .collect();
    let k = i32::try_from(digits.len()).expect("at most 17 digits");
    let n = exponent + 1;
    let body = if (k..=21).contains(&n) {
        let zeros = usize::try_from(n - k).expect("n is at least k");
        format!("{digits}{}", "0".repeat(zeros))
    } else if (1..=21).contains(&n) {
        let (whole, fraction) = digits.split_at(usize::try_from(n).expect("n is positive"));
        format!("{whole}.{fraction}")
    } else if (-5..=0).contains(&n) {
        let zeros = usize::try_from(-n).expect("n is at most 0");
        format!("0.{}{digits}", "0".repeat(zeros))
    } else {
        let e = n - 1;
        let exponent_sign = if e < 0 { '-' } else { '+' };
        let magnitude = e.abs();
        let (first, rest) = digits.split_at(1);
        if rest.is_empty() {
            format!("{first}e{exponent_sign}{magnitude}")
        } else {
            format!("{first}.{rest}e{exponent_sign}{magnitude}")
        }
    };
    format!("{sign}{body}")
}

/// The vectors' numbers reach this serializer as the doubles their text denotes only if `serde_json` parses them
/// correctly rounded: its `float_roundtrip` feature, which the test crate's graph enables (through `jsonschema`); it is
/// built without `arbitrary_precision`, whose numbers `as_f64` would parse with the standard library anyway.
fn numbers_parse_exactly() {
    for text in [
        "5e-324",
        "1.7976931348623157e308",
        "0.30000000000000004",
        "2.2250738585072014e-308",
        "9.999999999999997e-7",
        "123456789012345678901",
        "1424953923781206.2",
    ] {
        let parsed: Value = serde_json::from_str(text).expect("a JSON number");
        let expected: f64 = text.parse().expect("a decimal");
        assert_eq!(
            parsed.as_f64().map(f64::to_bits),
            Some(expected.to_bits()),
            "serde_json parses {text} to the nearest double"
        );
    }
}

fn rfc8785_vectors(root: &Path) {
    let vectors = read(&root.join("vectors/rfc8785.json"));
    let cases = array(&vectors["cases"]);
    assert!(cases.len() >= 24, "at least 24 RFC 8785 vectors");
    for case in cases {
        let name = text(case, "name");
        let input: Value = serde_json::from_str(text(case, "input"))
            .unwrap_or_else(|error| panic!("{name}: the input is JSON: {error}"));
        let bytes = canonical(&input);
        assert_eq!(
            hex(&bytes),
            text(case, "canonicalHex"),
            "{name}: the canonical bytes"
        );
        assert_eq!(
            String::from_utf8(bytes).expect("UTF-8"),
            text(case, "canonical"),
            "{name}: the canonical text"
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// SHA-256, descriptors and the chain

fn sha(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).iter().copied().collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2), "{text} has an even length");
    (0..text.len())
        .step_by(2)
        .map(|at| {
            let pair = text.get(at..at + 2).expect("two hexadecimal digits");
            u8::from_str_radix(pair, 16)
                .unwrap_or_else(|error| panic!("{pair} is hexadecimal: {error}"))
        })
        .collect()
}

fn descriptor(value: &Value) -> Value {
    let bytes = canonical(value);
    json!({"sha256": hex(&sha(&bytes)), "size": bytes.len()})
}

fn d_of(seq: u64, kind: &str, target: &str, body: &Value) -> Vec<u8> {
    canonical(
        &json!({"seq": seq, "kind": kind, "target": target, "body": hex(&sha(&canonical(body)))}),
    )
}

fn extend(chain: &[u8], d: &[u8]) -> Vec<u8> {
    let length = u32::try_from(d.len()).expect("d(k) is short");
    let mut buffer = chain.to_vec();
    buffer.extend_from_slice(&length.to_be_bytes());
    buffer.extend_from_slice(d);
    sha(&buffer)
}

/// A value that may travel as a blob enters the body as its descriptor; a blob-carried value of the corpus is rebuilt
/// from its generator, `{"rows": [{"i": i, "label": label + i}, ...]}`.
fn carried(command: &Value, value: Option<&Value>) -> Value {
    if let Some(blob) = command.get("valueBlob") {
        let count = blob["generator"]["rows"].as_u64().expect("a row count");
        let label = text(&blob["generator"], "label");
        let rows: Vec<Value> = (0..count)
            .map(|i| json!({"i": i, "label": format!("{label}{i}")}))
            .collect();
        let rebuilt = descriptor(&json!({ "rows": rows }));
        assert_eq!(
            rebuilt, blob["descriptor"],
            "the blob's descriptor is that of its rebuilt value"
        );
        return rebuilt;
    }
    descriptor(value.unwrap_or_else(|| panic!("{command} carries a value")))
}

fn with_value(command: &Value, key: &str) -> Value {
    let mut body = command["fragment"]
        .as_object()
        .unwrap_or_else(|| panic!("{command} has a fragment"))
        .clone();
    body.insert(key.to_owned(), carried(command, command.get("value")));
    Value::Object(body)
}

fn seq_number(value: &Value) -> u64 {
    match value {
        Value::String(decimal) => decimal
            .parse()
            .unwrap_or_else(|error| panic!("{decimal}: {error}")),
        other => other
            .as_u64()
            .unwrap_or_else(|| panic!("{other} is a sequence")),
    }
}

fn utf16_order(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// B(k) of `README.md`, from the fields of the `Command` message the entry shows.
fn body_of(command: &Value) -> Value {
    let typed = &command["typed"];
    match text(command, "kind") {
        "activity" | "script" => {
            let mut body = with_value(command, "input");
            if let Some(scope) = command.get("sagaScope") {
                let fields = body
                    .as_object_mut()
                    .expect("a work command's body is an object");
                fields.insert("scope".to_owned(), scope.clone());
            }
            body
        }
        "child" => with_value(command, "input"),
        "approval" => with_value(command, "subject"),
        "timer_start" | "wait_event" => command["fragment"].clone(),
        "timer_cancel" | "step_cancel" => json!({"target": typed["target"]["target"]}),
        "saga_open" => {
            let open = &typed["sagaOpen"];
            let parent = match open.get("parentScope") {
                Some(Value::String(scope)) if !scope.is_empty() => json!(scope),
                _ => Value::Null,
            };
            json!({
                "scope": open["scope"],
                "parentScope": parent,
                "onCompensationFailure": open["onCompensationFailure"],
                "cancelPolicy": open["cancelPolicy"],
            })
        }
        "saga_close" => json!({"scope": typed["sagaClose"]["target"]}),
        "saga_abort" => json!({"scope": typed["sagaAbort"]["target"]}),
        "marker_patch" => json!({"markerId": typed["marker"]["markerId"]}),
        "marker_side_effect" => json!({
            "markerId": typed["marker"]["markerId"],
            "value": descriptor(&typed["marker"]["sideEffectValue"]),
        }),
        "set_state" => json!({"state": typed["setState"]}),
        "update_accept" => {
            json!({"admittedSeq": seq_number(&typed["updateDecision"]["admittedSeq"]), "accepted": true})
        }
        "update_reject" => json!({
            "admittedSeq": seq_number(&typed["updateDecision"]["admittedSeq"]),
            "accepted": false,
            "reason": typed["updateDecision"]["rejectionReason"],
        }),
        "update_complete" => {
            let result = &typed["updateResult"];
            let admitted = seq_number(&result["admittedSeq"]);
            match (result.get("result"), result.get("failureCode")) {
                (Some(value), None) => {
                    json!({"admittedSeq": admitted, "result": descriptor(value)})
                }
                (None, Some(code)) => json!({"admittedSeq": admitted, "failureCode": code}),
                _ => panic!(
                    "{command}: exactly one of a result and a failure code (else COMMAND_INVALID)"
                ),
            }
        }
        "consume_signals" => {
            let listed = array(&typed["consumeSignals"]["signals"]);
            let names: Vec<&str> = listed
                .iter()
                .map(|signal| text(signal, "signalName"))
                .collect();
            assert!(
                names
                    .windows(2)
                    .all(|pair| utf16_order(pair[0], pair[1]).is_lt()),
                "{command}: signal names in ascending UTF-16 order"
            );
            let signals: Vec<Value> = listed
                .iter()
                .map(|signal| {
                    let seqs: Vec<u64> = array(&signal["seqs"]).iter().map(seq_number).collect();
                    assert!(
                        seqs.windows(2).all(|pair| pair[0] < pair[1]),
                        "{command}: ascending sequences"
                    );
                    json!({"name": signal["signalName"], "seqs": seqs})
                })
                .collect();
            json!({ "signals": signals })
        }
        "complete" => json!({"output": carried(command, typed.get("complete"))}),
        "fail" => {
            json!({"code": typed["fail"]["code"], "message": descriptor(&typed["fail"]["safeMessage"])})
        }
        "continue_as_new" => {
            let next = &typed["continueAsNew"];
            let version = if next.get("activeVersion") == Some(&Value::Bool(true)) {
                "active"
            } else {
                "same"
            };
            json!({
                "input": next["input"],
                "state": next.get("state").cloned().unwrap_or(Value::Null),
                "version": version,
            })
        }
        "acknowledge_cancellation" => json!({}),
        other => panic!("{other} is not a kind of the README's body table"),
    }
}

/// Checks the body, its digest and the fragment of one command entry, and returns the body.
fn check_command(command: &Value, profiles: &Profiles) -> Value {
    i_json_integers(command, "a command");
    let body = body_of(command);
    assert_eq!(&body, &command["body"], "{command}: the body");
    assert_eq!(
        hex(&sha(&canonical(&body))),
        text(command, "bodySha256"),
        "{command}: the body's digest"
    );
    let kind = text(command, "kind");
    match (profiles.get(kind), command.get("fragment")) {
        (Some(validator), Some(fragment)) => {
            let errors: Vec<String> = validator
                .iter_errors(fragment)
                .map(|error| format!("{error} (at {})", error.instance_path()))
                .collect();
            assert!(
                errors.is_empty(),
                "{fragment} is outside the code profile of {kind}: {}",
                errors.join("; ")
            );
        }
        (Some(_), None) => panic!("{command}: a work command carries a fragment"),
        (None, Some(_)) => panic!("{command}: only a work command carries a fragment"),
        (None, None) => {}
    }
    body
}

fn profiles() -> Profiles {
    let schema = read_json("specs/001-platform-foundation/contracts/workflow.schema.json");
    PROFILES
        .iter()
        .map(|(kind, definition)| {
            let root = json!({
                "$schema": schema["$schema"],
                "$defs": schema["$defs"],
                "$ref": format!("#/$defs/{definition}"),
            });
            let validator = jsonschema::options()
                .build(&root)
                .unwrap_or_else(|error| panic!("{definition} compiles: {error}"));
            (*kind, validator)
        })
        .collect()
}

fn chain_vectors(root: &Path, profiles: &Profiles) {
    let vectors = read(&root.join("vectors/chain.json"));
    let mut kinds = BTreeSet::new();
    let mut chains: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for list in array(&vectors["lists"]) {
        let name = text(list, "name");
        let mut chain = vec![0_u8; 32];
        let mut hexes = Vec::new();
        for (index, command) in array(&list["commands"]).iter().enumerate() {
            let seq = command["seq"].as_u64().expect("a sequence");
            assert_eq!(
                usize::try_from(seq).expect("small"),
                index + 1,
                "{name}: commands numbered 1, 2, 3..."
            );
            let body = check_command(command, profiles);
            let kind = text(command, "kind");
            kinds.insert(kind.to_owned());
            let d = d_of(seq, kind, text(command, "target"), &body);
            assert_eq!(hex(&d), text(command, "dHex"), "{name}: d({seq})");
            chain = extend(&chain, &d);
            assert_eq!(hex(&chain), text(command, "chainHex"), "{name}: H({seq})");
            hexes.push(hex(&chain));
        }
        chains.insert(name.to_owned(), hexes);
    }
    let every_kind: BTreeSet<String> = proto_enums()["CommandKind"]
        .iter()
        .filter(|name| name.as_str() != "COMMAND_KIND_UNSPECIFIED")
        .map(|name| name.trim_start_matches("COMMAND_").to_lowercase())
        .collect();
    assert_eq!(
        kinds, every_kind,
        "the chain vectors cover every kind of command"
    );

    let base = &chains["sagas-state-signals"];
    for (variant, first_difference) in [
        ("sagas-state-signals-changed-input", 2),
        ("sagas-state-signals-changed-state", 7),
    ] {
        let other = &chains[variant];
        assert_eq!(other.len(), base.len(), "{variant} has the same commands");
        for (index, (a, b)) in base.iter().zip(other).enumerate() {
            if index + 1 < first_difference {
                assert_eq!(a, b, "{variant}: H({}) is shared", index + 1);
            } else {
                assert_ne!(
                    a,
                    b,
                    "{variant}: H({}) differs from command {first_difference} on",
                    index + 1
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The deterministic primitives

struct Xoshiro256 {
    s: [u64; 4],
    calls: u64,
}

impl Xoshiro256 {
    fn new(seed: &[u8]) -> Self {
        assert_eq!(seed.len(), 32, "a seed has 32 bytes");
        let mut s = [0_u64; 4];
        let (words, _) = seed.as_chunks::<8>();
        for (word, chunk) in s.iter_mut().zip(words) {
            *word = u64::from_le_bytes(*chunk);
        }
        if s == [0; 4] {
            // The all-zero seed is replaced by the first four outputs of SplitMix64 started at 0.
            let mut x = 0_u64;
            for word in &mut s {
                x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = x;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                *word = z ^ (z >> 31);
            }
        }
        Self { s, calls: 0 }
    }

    fn next_u64(&mut self) -> u64 {
        let [s0, s1, s2, s3] = self.s;
        let result = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s1 << 17;
        let mut n2 = s2 ^ s0;
        let mut n3 = s3 ^ s1;
        let n1 = s1 ^ n2;
        let n0 = s0 ^ n3;
        n2 ^= t;
        n3 = n3.rotate_left(45);
        self.s = [n0, n1, n2, n3];
        self.calls += 1;
        result
    }

    /// `(next() >> 11) × 2^-53`, exact: 53 bits fit a double's significand.
    fn random(&mut self) -> f64 {
        let bits = self.next_u64() >> 11;
        let high = u32::try_from(bits >> 32).expect("21 bits");
        let low = u32::try_from(bits & 0xffff_ffff).expect("32 bits");
        (f64::from(high) * 4_294_967_296.0 + f64::from(low)) * 2.0_f64.powi(-53)
    }

    /// The value and the number of `next()` calls it took.
    fn random_int(&mut self, n: u64) -> (u64, u64) {
        assert!((1..=1 << 32).contains(&n), "0 < n <= 2^32");
        let threshold = n.wrapping_neg() % n; // (2^64 - n) mod n
        let before = self.calls;
        loop {
            let x = self.next_u64();
            if x >= threshold {
                return (x % n, self.calls - before);
            }
        }
    }

    fn uuid(&mut self) -> String {
        let mut bytes = [0_u8; 16];
        let (first, second) = bytes.split_at_mut(8);
        first.copy_from_slice(&self.next_u64().to_le_bytes());
        second.copy_from_slice(&self.next_u64().to_le_bytes());
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let h = hex(&bytes);
        format!(
            "{}-{}-{}-{}-{}",
            &h[0..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..32]
        )
    }
}

fn same_double(actual: f64, expected: &Value) -> bool {
    expected.as_f64().map(f64::to_bits) == Some(actual.to_bits())
}

/// Recomputes every seed's stream and returns the vectors file.
fn primitives_vectors(root: &Path) -> Value {
    let vectors = read(&root.join("vectors/primitives.json"));
    let seeds = array(&vectors["seeds"]);
    assert!(
        seeds.len() >= 5,
        "the all-zero, sequential, all-ff and two more seeds"
    );
    for seed in seeds {
        let seed_hex = text(seed, "seedHex");
        let bytes = unhex(seed_hex);
        let fresh = Xoshiro256::new(&bytes);
        let words: Vec<String> = fresh.s.iter().map(|word| format!("{word:016x}")).collect();
        assert_eq!(
            json!(words),
            seed["stateWords"],
            "{seed_hex}: the state words"
        );

        let mut fresh = Xoshiro256::new(&bytes);
        let next: Vec<String> = (0..8)
            .map(|_| format!("{:016x}", fresh.next_u64()))
            .collect();
        assert_eq!(json!(next), seed["next"], "{seed_hex}: the first 8 next()");

        let mut stream = Xoshiro256::new(&bytes);
        let random = array(&seed["random"]);
        assert_eq!(random.len(), 64, "{seed_hex}: 64 random()");
        for (index, expected) in random.iter().enumerate() {
            let actual = stream.random();
            assert!(
                same_double(actual, expected),
                "{seed_hex}: random() {index} is {actual}, not {expected}"
            );
        }
        for group in array(&seed["randomInt"]) {
            let n = group["n"].as_u64().expect("a bound");
            for (value, calls) in array(&group["values"])
                .iter()
                .zip(array(&group["nextCalls"]))
            {
                let (actual, used) = stream.random_int(n);
                assert_eq!(Some(actual), value.as_u64(), "{seed_hex}: randomInt({n})");
                assert_eq!(
                    Some(used),
                    calls.as_u64(),
                    "{seed_hex}: the next() calls of randomInt({n})"
                );
            }
        }
        let uuids: Vec<String> = (0..8).map(|_| stream.uuid()).collect();
        assert_eq!(json!(uuids), seed["uuid"], "{seed_hex}: uuid()");
        assert_eq!(
            Some(stream.calls),
            seed["streamNextCalls"].as_u64(),
            "{seed_hex}: the stream's length"
        );
    }
    for case in array(&vectors["rejection"]) {
        let bytes = unhex(text(case, "seedHex"));
        let n = case["n"].as_u64().expect("a bound");
        let mut draws = Xoshiro256::new(&bytes);
        let first: Vec<String> = (0..2)
            .map(|_| format!("{:016x}", draws.next_u64()))
            .collect();
        assert_eq!(
            json!(first),
            case["draws"],
            "the rejection seed's first draws"
        );
        let (value, calls) = Xoshiro256::new(&bytes).random_int(n);
        assert_eq!(
            Some(value),
            case["value"].as_u64(),
            "randomInt({n}) after a rejection"
        );
        assert_eq!(
            Some(calls),
            case["nextCalls"].as_u64(),
            "randomInt({n}) draws again after 0"
        );
        assert!(calls > 1, "the rejection vector rejects a draw");
    }
    vectors
}

/// The scenario's output is the stream of the sequential seed in the primitives vectors.
fn the_primitives_scenario_draws_the_vector_stream(dir: &Path, primitives: &Value) {
    let commands = read(&dir.join("commands.json"));
    let vector = array(&primitives["seeds"])
        .iter()
        .find(|seed| seed["seedHex"] == commands["seedHex"])
        .expect("the scenario's seed is a primitives vector");
    let activations = array(&commands["activations"]);
    let last = activations.last().expect("an activation");
    let output = &array(&last["commands"]).last().expect("the completion")["typed"]["complete"];
    assert_eq!(
        output["random"], vector["random"],
        "the scenario's random()"
    );
    assert_eq!(output["uuid"], vector["uuid"], "the scenario's uuid()");
    for (drawn, expected) in array(&output["randomInt"])
        .iter()
        .zip(array(&vector["randomInt"]))
    {
        assert_eq!(drawn["n"], expected["n"], "the scenario's bounds");
        assert_eq!(
            drawn["values"], expected["values"],
            "the scenario's randomInt()"
        );
    }
    let times: Vec<u64> = activations
        .iter()
        .map(|activation| activation["nowUs"].as_u64().expect("a time") / 1000)
        .collect();
    assert_eq!(
        json!(times),
        output["nowMs"],
        "now() is the time of S(k), in milliseconds"
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Scenarios

fn proto_enums() -> Enums {
    let set = worker_proto();
    let file = set
        .file
        .iter()
        .find(|f| f.name() == "worker.proto")
        .expect("worker.proto");
    file.enum_type
        .iter()
        .map(|enumeration| {
            let values = enumeration
                .value
                .iter()
                .map(|v| v.name().to_owned())
                .collect();
            (enumeration.name().to_owned(), values)
        })
        .collect()
}

fn in_enum(enums: &Enums, enumeration: &str, value: &Value, at: &str) {
    let name = value
        .as_str()
        .unwrap_or_else(|| panic!("{at}: {value} is an enum value by name"));
    assert!(
        enums[enumeration].contains(name),
        "{at}: {name} is a value of {enumeration}"
    );
}

/// Every `json` member of a HistoryPayload is the base64 of the RFC 8785 bytes of its value.
fn check_payloads(value: &Value, at: &str) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(encoded)) = map.get("json") {
                let bytes = STANDARD
                    .decode(encoded)
                    .unwrap_or_else(|error| panic!("{at}: a payload is standard base64: {error}"));
                let parsed: Value = serde_json::from_slice(&bytes)
                    .unwrap_or_else(|error| panic!("{at}: a payload is JSON: {error}"));
                assert_eq!(
                    canonical(&parsed),
                    bytes,
                    "{at}: a payload's json is RFC 8785"
                );
                i_json_integers(&parsed, at);
            }
            for item in map.values() {
                check_payloads(item, at);
            }
        }
        Value::Array(items) => {
            for item in items {
                check_payloads(item, at);
            }
        }
        _ => {}
    }
}

/// I-JSON (RFC 7493), which RFC 8785 assumes: a command refuses an integer outside ±(2^53 − 1) anywhere
/// (COMMAND_INVALID), so no command or scenario of the corpus holds one; every double of magnitude 2^53 or more is
/// an integer. Only the serializer vectors of `rfc8785.json` print such numbers.
fn i_json_integers(value: &Value, at: &str) {
    const SAFE: f64 = 9_007_199_254_740_991.0;
    match value {
        Value::Number(number) => {
            let x = number.as_f64().expect("a JSON number is a double");
            assert!(
                x.fract().classify() != FpCategory::Zero || x.abs() <= SAFE,
                "{at}: {number} is an integer outside ±(2^53 − 1)"
            );
        }
        Value::Array(items) => {
            for item in items {
                i_json_integers(item, at);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                i_json_integers(item, at);
            }
        }
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
}

fn payload_value(payload: &Value) -> Value {
    let bytes = STANDARD
        .decode(text(payload, "json"))
        .expect("standard base64");
    serde_json::from_slice(&bytes).expect("a JSON payload")
}

/// One `HistoryEvent` in the proto3 JSON mapping; returns its sequence.
fn check_event(event: &Value, enums: &Enums, at: &str) -> u64 {
    let object = event
        .as_object()
        .unwrap_or_else(|| panic!("{at}: a HistoryEvent is an object"));
    let kind = text(event, "kind");
    in_enum(enums, "HistoryEventKind", &event["kind"], at);
    in_enum(enums, "HistoryEventOrigin", &event["origin"], at);
    let body = BODY_OF_KIND
        .iter()
        .find(|(name, _)| *name == kind)
        .map(|(_, field)| *field)
        .unwrap_or_else(|| panic!("{at}: {kind} has a body"));
    for key in object.keys() {
        assert!(
            ["seq", "kind", "origin", "commandNumber", "atUs", body].contains(&key.as_str()),
            "{at}: {key} is not a field of a {kind} HistoryEvent"
        );
    }
    assert!(object.contains_key(body), "{at}: {kind} carries {body}");
    int64(&event["atUs"], at);
    let numbered = matches!(kind, "HISTORY_COMMAND_RECORDED" | "HISTORY_MARKER_RECORDED");
    assert_eq!(
        object.contains_key("commandNumber"),
        numbered,
        "{at}: only a command record and a marker carry a command number"
    );
    let fields = &event[body];
    match kind {
        "HISTORY_COMMAND_RECORDED" => in_enum(enums, "CommandKind", &fields["kind"], at),
        "HISTORY_STEP_OUTCOME" => in_enum(enums, "StepOutcome", &fields["outcome"], at),
        "HISTORY_CANCELLATION_REQUESTED" => {
            in_enum(enums, "CancelOrigin", &fields["cancelOrigin"], at)
        }
        "HISTORY_WORKFLOW_TASK_COMPLETED" if fields.get("deferred").is_some() => {
            in_enum(enums, "CommandKind", &fields["deferredKind"], at);
            in_enum(
                enums,
                "WorkflowTaskFailureCause",
                &fields["deferralCause"],
                at,
            );
        }
        _ => {}
    }
    check_payloads(fields, at);
    int64(&event["seq"], at)
}

/// The kind, the target and the saga scope a history record states for a command number (the scope is empty outside
/// a scope and for every kind but an activity or a script, Decision 044-244).
fn recorded(event: &Value) -> (String, String, String) {
    if text(event, "kind") == "HISTORY_MARKER_RECORDED" {
        let marker = &event["markerRecorded"];
        let kind = if marker.get("sideEffect") == Some(&Value::Bool(true)) {
            "COMMAND_MARKER_SIDE_EFFECT"
        } else {
            "COMMAND_MARKER_PATCH"
        };
        return (
            kind.to_owned(),
            text(marker, "markerId").to_owned(),
            String::new(),
        );
    }
    let record = &event["commandRecord"];
    let optional = |key: &str| {
        record
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    (
        text(record, "kind").to_owned(),
        optional("target"),
        optional("sagaScope"),
    )
}

/// Each step ends at most once (Decision 044-245): with its outcome event, or for a timer with the record of the
/// function's `timer_cancel`. An outcome of the origin `EFFECT` follows, in the same completion, the `step_cancel`
/// record of its step; a cancelled outcome is `EFFECT` or `ENGINE`; a timer's cancelled step outcome comes from the
/// stop drain, after which no workflow task starts.
fn one_end_per_step_with_its_origin(
    events: &[Value],
    kind_of_node: &BTreeMap<&str, &str>,
    leased: &BTreeMap<String, u64>,
    at: &str,
) {
    let mut ends: BTreeMap<String, u64> = BTreeMap::new();
    let mut cancelled_in_this_completion: BTreeSet<String> = BTreeSet::new();
    for (index, event) in events.iter().enumerate() {
        let kind = text(event, "kind");
        let seq = int64(&event["seq"], at);
        // A completion's own rows (its records and their immediate effects) are contiguous: any other row ends them.
        if kind == "HISTORY_WORKFLOW_TASK_COMPLETED"
            || !matches!(
                text(event, "origin"),
                "HISTORY_ORIGIN_COMPLETION" | "HISTORY_ORIGIN_EFFECT"
            )
        {
            cancelled_in_this_completion.clear();
        }
        if kind == "HISTORY_WORKFLOW_TASK_COMPLETED" || kind == "HISTORY_WORKFLOW_TASK_STARTED" {
            continue;
        }
        let end = match kind {
            "HISTORY_STEP_OUTCOME" => Some(text(&event["stepOutcome"], "node").to_owned()),
            "HISTORY_TIMER_FIRED" => Some(text(&event["timerFired"], "node").to_owned()),
            "HISTORY_WAIT_OUTCOME" => Some(text(&event["waitOutcome"], "node").to_owned()),
            "HISTORY_CHILD_FINISHED" => Some(text(&event["childFinished"], "node").to_owned()),
            "HISTORY_APPROVAL_DECIDED" => Some(text(&event["approvalDecided"], "node").to_owned()),
            "HISTORY_COMMAND_RECORDED" => {
                let record = &event["commandRecord"];
                let noop = record.get("noop") == Some(&Value::Bool(true));
                let target = record
                    .get("target")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match text(record, "kind") {
                    "COMMAND_STEP_CANCEL" if !noop => {
                        cancelled_in_this_completion.insert(target.to_owned());
                        None
                    }
                    "COMMAND_TIMER_CANCEL" if !noop => Some(target.to_owned()),
                    _ => None,
                }
            }
            _ => None,
        };
        let Some(node) = end else { continue };
        if let Some(first) = ends.insert(node.clone(), seq) {
            panic!("{at}: {node} ends twice, at sequences {first} and {seq}");
        }
        let origin = text(event, "origin");
        if origin == "HISTORY_ORIGIN_EFFECT" {
            // A step whose attempt began is never cancelled by the command alone (Decision 044-245).
            assert!(
                leased.get(&node).is_none_or(|lease| *lease > seq),
                "{at}: {node} was leased, so its outcome at {seq} is not the command's effect"
            );
            assert!(
                cancelled_in_this_completion.contains(&node),
                "{at}: the EFFECT outcome of {node} at {seq} follows its step_cancel in the same completion"
            );
        }
        if kind == "HISTORY_STEP_OUTCOME"
            && text(&event["stepOutcome"], "outcome") == "STEP_OUTCOME_CANCELLED"
        {
            assert!(
                matches!(origin, "HISTORY_ORIGIN_EFFECT" | "HISTORY_ORIGIN_ENGINE"),
                "{at}: a cancelled step's outcome at {seq} is EFFECT or ENGINE"
            );
            if kind_of_node.get(node.as_str()) == Some(&"timer_start") {
                assert!(
                    events[index + 1..]
                        .iter()
                        .all(|later| text(later, "kind") != "HISTORY_WORKFLOW_TASK_STARTED"),
                    "{at}: a timer's cancelled outcome at {seq} comes from the stop drain"
                );
            }
        }
    }
}

/// One run of a scenario: its history, its activations and windows, and its chain recomputed from its commands.
fn check_run(dir: &Path, enums: &Enums, profiles: &Profiles, cancelled: &mut BTreeSet<String>) {
    let at = dir.display().to_string();
    let commands = read(&dir.join("commands.json"));
    i_json_integers(&commands, &at);
    let chain_file = read(&dir.join("chain.json"));
    let history = std::fs::read_to_string(dir.join("history.jsonl"))
        .unwrap_or_else(|error| panic!("{at}: history.jsonl: {error}"));
    assert!(
        history.ends_with('\n'),
        "{at}: history.jsonl ends with a newline"
    );

    let mut events = Vec::new();
    let mut previous = 0;
    for (index, line) in history.lines().enumerate() {
        let where_ = format!("{at}/history.jsonl:{}", index + 1);
        let event: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("{where_}: not JSON: {error}"));
        let seq = check_event(&event, enums, &where_);
        assert!(
            seq > previous,
            "{where_}: journal sequences increase strictly"
        );
        previous = seq;
        events.push(event);
    }
    let first = events
        .first()
        .unwrap_or_else(|| panic!("{at}: an empty history"));
    assert_eq!(
        int64(&first["seq"], &at),
        1,
        "{at}: the history starts at journal sequence 1"
    );
    assert_eq!(
        text(first, "kind"),
        "HISTORY_RUN_STARTED",
        "{at}: run.started first"
    );
    assert_eq!(
        payload_value(&first["runStarted"]["input"]),
        commands["input"],
        "{at}: run.started carries the input"
    );

    let mut records: BTreeMap<u64, (String, String, String)> = BTreeMap::new();
    let mut started: BTreeMap<u64, u64> = BTreeMap::new();
    for event in &events {
        let seq = int64(&event["seq"], &at);
        if let Some(number) = event.get("commandNumber") {
            let previous = records.insert(int64(number, &at), recorded(event));
            assert!(previous.is_none(), "{at}: one record per command number");
        }
        if text(event, "kind") == "HISTORY_WORKFLOW_TASK_STARTED" {
            started.insert(seq, int64(&event["atUs"], &at));
        }
    }

    let chain_commands = array(&chain_file["commands"]);
    let chain_activations = array(&chain_file["activations"]);
    let mut chain = vec![0_u8; 32];
    let mut count = 0_u64;
    let mut after = 0_u64;
    let activations = array(&commands["activations"]);
    for (index, activation) in activations.iter().enumerate() {
        let k = activation["k"].as_u64().expect("k");
        assert_eq!(
            usize::try_from(k).expect("small"),
            index + 1,
            "{at}: activations numbered from 1"
        );
        let window = &activation["window"];
        assert_eq!(
            window["afterSeq"].as_u64(),
            Some(after),
            "{at}: activation {k} starts at S(k - 1)"
        );
        for attempt in activation
            .get("failedAttempts")
            .map(array)
            .unwrap_or_default()
        {
            let seq = attempt["startedSeq"].as_u64().expect("a started sequence");
            assert!(
                seq > after && started.contains_key(&seq),
                "{at}: a failed attempt has its own start"
            );
        }
        let Some(through) = window["throughSeq"].as_u64() else {
            assert!(
                activation.get("review").is_some(),
                "{at}: an activation without S(k) is a review"
            );
            assert_eq!(
                index + 1,
                activations.len(),
                "{at}: the review ends the golden"
            );
            continue;
        };
        let time = started
            .get(&through)
            .unwrap_or_else(|| panic!("{at}: S({k}) = {through} is a workflow_task.started"));
        assert_eq!(
            activation["nowUs"].as_u64(),
            Some(*time),
            "{at}: now() is the time of S({k})"
        );
        assert_eq!(
            activation["appliedThroughSeq"].as_u64(),
            Some(through),
            "{at}: applied through S({k})"
        );
        assert_eq!(
            text(activation, "replayedChainHex"),
            hex(&chain),
            "{at}: the replayed chain of {k}"
        );
        for command in array(&activation["commands"]) {
            count += 1;
            assert_eq!(
                command["commandNumber"].as_u64(),
                Some(count),
                "{at}: commands numbered 1, 2, 3..."
            );
            let body = check_command(command, profiles);
            let kind = text(command, "kind");
            let target = text(command, "target");
            chain = extend(&chain, &d_of(count, kind, target, &body));
            let entry = chain_commands
                .get(usize::try_from(count - 1).expect("small"))
                .unwrap_or_else(|| panic!("{at}: chain.json has command {count}"));
            assert_eq!(
                entry["commandNumber"].as_u64(),
                Some(count),
                "{at}: chain.json is in command order"
            );
            assert_eq!(text(entry, "chainHex"), hex(&chain), "{at}: H({count})");
            let scope = command
                .get("sagaScope")
                .and_then(Value::as_str)
                .unwrap_or_default();
            assert!(
                scope.is_empty() || matches!(kind, "activity" | "script"),
                "{at}: only an activity or a script is scheduled in a scope"
            );
            let expected = (
                format!("COMMAND_{}", kind.to_uppercase()),
                target.to_owned(),
                scope.to_owned(),
            );
            assert_eq!(
                records.get(&count),
                Some(&expected),
                "{at}: the history records command {count}"
            );
        }
        if let Some(deferred) = activation.get("deferred") {
            assert!(
                deferred.get("commandNumber").is_none(),
                "{at}: a deferred command takes no number"
            );
            check_command(deferred, profiles);
            let entry = chain_activations
                .iter()
                .find(|entry| entry["k"] == activation["k"])
                .expect("the deferral's activation");
            assert_eq!(
                entry["deferredBodySha256"], deferred["bodySha256"],
                "{at}: the deferral's digest"
            );
        }
        assert_eq!(
            text(activation, "chainAfterHex"),
            hex(&chain),
            "{at}: the chain after {k}"
        );
        assert_eq!(
            activation["commandCountAfter"].as_u64(),
            Some(count),
            "{at}: the count after {k}"
        );
        let entry = chain_activations
            .iter()
            .find(|entry| entry["k"] == activation["k"])
            .unwrap_or_else(|| panic!("{at}: chain.json has activation {k}"));
        assert_eq!(
            text(entry, "chainHex"),
            hex(&chain),
            "{at}: chain.json after {k}"
        );
        after = through;
    }
    assert_eq!(
        u64::try_from(records.len()).expect("small"),
        count,
        "{at}: every recorded command is a command of an activation"
    );

    // A step outcome ends an activity or a script, a cancelled wait or approval, and a timer that the engine cancelled
    // (the stop drain); a child ends with its child.finished, a timer the function cancelled with the record of its
    // timer_cancel (Decision 044-245).
    let kind_of_node: BTreeMap<&str, &str> = activations
        .iter()
        .flat_map(|activation| activation.get("commands").map(array).unwrap_or_default())
        .filter(|command| {
            matches!(
                text(command, "kind"),
                "activity" | "script" | "timer_start" | "wait_event" | "child" | "approval"
            )
        })
        .filter_map(|command| Some((command.get("node")?.as_str()?, text(command, "kind"))))
        .collect();
    for event in &events {
        if text(event, "kind") != "HISTORY_STEP_OUTCOME" {
            continue;
        }
        let outcome = &event["stepOutcome"];
        let node = text(outcome, "node");
        let kind = kind_of_node.get(node).unwrap_or_else(|| {
            panic!("{at}: the step outcome of {node} names a step a command created")
        });
        let cancelled_step = text(outcome, "outcome") == "STEP_OUTCOME_CANCELLED";
        assert!(
            matches!(*kind, "activity" | "script")
                || (cancelled_step && matches!(*kind, "wait_event" | "approval" | "timer_start")),
            "{at}: a {kind} step ends with its own event, not this step outcome"
        );
        if cancelled_step {
            assert_eq!(
                outcome.get("errorCode").and_then(Value::as_str),
                Some("CANCELLED"),
                "{at}: a cancelled step carries the error code CANCELLED"
            );
            cancelled.insert((*kind).to_owned());
        }
    }
    let leased: BTreeMap<String, u64> = array(&commands["invisibleJournal"])
        .iter()
        .filter(|row| row["journalKind"] == "step.leased")
        .filter_map(|row| Some((row.get("node")?.as_str()?.to_owned(), row["seq"].as_u64()?)))
        .collect();
    one_end_per_step_with_its_origin(&events, &kind_of_node, &leased, &at);
    assert_eq!(
        u64::try_from(chain_commands.len()).expect("small"),
        count,
        "{at}: chain.json lists every command"
    );
}

fn the_generator_reproduces_the_corpus() {
    let script = repository_root().join(GOLDENS).join("generate.mjs");
    let version = Command::new("node").arg("--version").output().unwrap_or_else(|error| {
        panic!(
            "the golden corpus is checked by running {} with Node 20 or later, and `node` cannot run here: {error}",
            script.display()
        )
    });
    let printed = String::from_utf8_lossy(&version.stdout);
    let major: u32 = printed
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|major| major.parse().ok())
        .unwrap_or(0);
    assert!(
        major >= 20,
        "the golden corpus needs Node 20 or later to run {}; `node --version` printed {printed}",
        script.display()
    );
    let output = Command::new("node")
        .arg(&script)
        .arg("--check")
        .current_dir(repository_root())
        .output()
        .expect("node runs generate.mjs");
    assert!(
        output.status.success(),
        "generate.mjs --check: the checked-in corpus is not what the generator produces (read the diff, then run \
         `node {GOLDENS}/generate.mjs`):\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// ---------------------------------------------------------------------------------------------------------------
// Reading

fn read(path: &Path) -> Value {
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&source)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()))
}

fn array(value: &Value) -> &[Value] {
    value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| panic!("{value} is an array"))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{value} has the string {key}"))
}

/// A proto3 JSON int64: a decimal string.
fn int64(value: &Value, at: &str) -> u64 {
    value
        .as_str()
        .and_then(|decimal| decimal.parse().ok())
        .unwrap_or_else(|| panic!("{at}: {value} is an int64 as a decimal string"))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .expect("a UTF-8 name")
        .to_owned()
}
