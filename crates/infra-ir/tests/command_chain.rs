//! The public RFC 8785 function and the command chain against the golden corpus of design 044 (Task 044-5,
//! `specs/044-workflow-as-code/contracts/goldens/`). The corpus is written by `generate.mjs` on Node, never by this
//! crate, so these tests compare the server's serializer and chain with an independent one.
#![allow(clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use sagat_domain::execution::code::{ChainedCommand, CommandChain, CommandDigester, CommandKind};
use sagat_infra_ir::Sha256CommandDigester;
use sagat_infra_ir::canonical::rfc8785;
use serde_json::Value;

fn goldens() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../specs/044-workflow-as-code/contracts/goldens")
}

fn read(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} is a string in {value}"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256CommandDigester.sha256(bytes)
}

fn unhex32(text: &str) -> [u8; 32] {
    assert_eq!(text.len(), 64, "{text} is 32 bytes in hexadecimal");
    let mut out = [0_u8; 32];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&text[2 * index..2 * index + 2], 16).expect("hexadecimal");
    }
    out
}

#[test]
fn the_rfc8785_vectors_of_the_corpus_hold() {
    let vectors = read(&goldens().join("vectors/rfc8785.json"));
    let cases = vectors["cases"].as_array().expect("cases");
    assert!(cases.len() >= 24, "the corpus has its 24 vectors or more");
    for case in cases {
        let name = text(case, "name");
        let input: Value = serde_json::from_str(text(case, "input"))
            .unwrap_or_else(|error| panic!("{name}: the input is JSON: {error}"));
        let bytes = rfc8785(&input);
        assert_eq!(
            String::from_utf8_lossy(&bytes),
            text(case, "canonical"),
            "{name}: the canonical text"
        );
        assert_eq!(
            hex(&bytes),
            text(case, "canonicalHex"),
            "{name}: the canonical bytes"
        );
    }
}

/// A language's default float printing is wrong on a tie: 1424953923781206.25 is exactly halfway between the two
/// shortest forms that round-trip, and ECMA-262 takes the even digit (Number::toString, step 5), where Rust's `{}`
/// prints the odd one.
#[test]
fn rfc8785_prints_a_tie_to_the_even_digit() {
    // Built exactly: an integer below 2^53 plus a quarter, both doubles (a literal would trip a precision lint).
    let whole = 1_424_953_923_781_206.0_f64;
    let tie = whole + 0.25;
    // Rust's `{}` prints this tie as ...206.3, the odd digit.
    assert_eq!(rfc8785(&Value::from(tie)), b"1424953923781206.2");
    assert_eq!(
        rfc8785(&Value::from(whole + 0.75)),
        b"1424953923781206.8",
        "a tie whose even digit is the upper one"
    );
    assert_eq!(rfc8785(&Value::from(-tie)), b"-1424953923781206.2");
    assert_eq!(
        rfc8785(&serde_json::json!([0.1, 1e21, 1e-7, -0.0])),
        b"[0.1,1e+21,1e-7,0]"
    );
}

/// Seventeen-digit decimals that a parser without correct rounding reads one unit in the last place off (serde_json's
/// own `float_roundtrip` cases): their RFC 8785 text is the text they were written with, so an honest worker's
/// canonical body prints back as it came.
#[test]
fn rfc8785_reprints_numbers_that_need_correct_rounding() {
    for text in [
        "51.248178375505404",
        "2.0030397744267762e-253",
        "1424953923781206.2",
    ] {
        let parsed: Value = serde_json::from_str(text).expect("a JSON number");
        let expected = text.as_bytes().to_vec();
        assert_eq!(rfc8785(&parsed), expected, "{text}");
    }
}

/// The chain vectors and every scenario run of the corpus: each body's digest is the SHA-256 of its RFC 8785 bytes,
/// each `d(k)` is the digester's, and the domain's fold gives each `H(k)`.
#[test]
fn the_chain_of_the_golden_commands_is_the_golden_hash() {
    let vectors = read(&goldens().join("vectors/chain.json"));
    let lists = vectors["lists"].as_array().expect("lists");
    assert!(!lists.is_empty(), "the chain vectors have lists");
    let mut checked = 0_usize;
    let mut described = 0_usize;
    for list in lists {
        let name = text(list, "name");
        let mut chain = CommandChain::EMPTY;
        for command in list["commands"].as_array().expect("commands") {
            let seq = u32::try_from(command["seq"].as_u64().expect("seq")).expect("small");
            let at = format!("{name}, command {seq}");
            let body_sha256 = sha256(&rfc8785(&command["body"]));
            assert_eq!(
                hex(&body_sha256),
                text(command, "bodySha256"),
                "{at}: the body's digest"
            );
            if let Some((field, value)) = carried_value(command) {
                assert_eq!(
                    command["body"][field],
                    descriptor(value),
                    "{at}: the descriptor of the {field}"
                );
                described += 1;
            }
            let kind = CommandKind::from_chain_name(text(command, "kind"))
                .unwrap_or_else(|| panic!("{at}: a known kind"));
            let target = text(command, "target");
            assert_eq!(
                hex(&Sha256CommandDigester.command_digest_input(
                    seq,
                    kind.chain_name(),
                    target,
                    body_sha256
                )),
                text(command, "dHex"),
                "{at}: d(k)"
            );
            chain = chain
                .extend(
                    &Sha256CommandDigester,
                    &ChainedCommand {
                        seq,
                        kind,
                        target,
                        body_sha256,
                    },
                )
                .unwrap_or_else(|error| panic!("{at}: {error}"));
            assert_eq!(hex(&chain.hash()), text(command, "chainHex"), "{at}: H(k)");
            checked += 1;
        }
    }

    for run in scenario_runs() {
        let at = run.display().to_string();
        let commands = read(&run.join("commands.json"));
        let golden = read(&run.join("chain.json"));
        let expected = golden["commands"].as_array().expect("chain.json commands");
        let mut chain = CommandChain::EMPTY;
        for activation in commands["activations"].as_array().expect("activations") {
            assert_eq!(
                hex(&chain.hash()),
                activation["replayedChainHex"]
                    .as_str()
                    .unwrap_or(&hex(&chain.hash())),
                "{at}: the chain a completion replays"
            );
            for command in activation["commands"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let seq = u32::try_from(command["commandNumber"].as_u64().expect("a number"))
                    .expect("small");
                let body_sha256 = sha256(&rfc8785(&command["body"]));
                assert_eq!(
                    hex(&body_sha256),
                    text(command, "bodySha256"),
                    "{at}: body of {seq}"
                );
                if let Some((field, value)) = carried_value(command) {
                    assert_eq!(
                        command["body"][field],
                        descriptor(value),
                        "{at}: the descriptor of command {seq}'s {field}"
                    );
                    described += 1;
                }
                let kind =
                    CommandKind::from_chain_name(text(command, "kind")).expect("a known kind");
                chain = chain
                    .extend(
                        &Sha256CommandDigester,
                        &ChainedCommand {
                            seq,
                            kind,
                            target: text(command, "target"),
                            body_sha256,
                        },
                    )
                    .unwrap_or_else(|error| panic!("{at}: {error}"));
                let golden_hash = expected
                    .get(usize::try_from(seq - 1).expect("small"))
                    .map(|entry| text(entry, "chainHex"))
                    .unwrap_or_else(|| panic!("{at}: chain.json has command {seq}"));
                assert_eq!(hex(&chain.hash()), golden_hash, "{at}: H({seq})");
                checked += 1;
            }
            if let Some(deferred) = activation.get("deferred") {
                assert_eq!(
                    hex(&sha256(&rfc8785(&deferred["body"]))),
                    text(deferred, "bodySha256"),
                    "{at}: the deferral's body digest"
                );
            }
            if let Some(after) = activation["chainAfterHex"].as_str() {
                assert_eq!(
                    hex(&chain.hash()),
                    after,
                    "{at}: the chain after the activation"
                );
                let count_after = u32::try_from(
                    activation["commandCountAfter"]
                        .as_u64()
                        .expect("the count after an activation"),
                )
                .expect("small");
                assert_eq!(
                    CommandChain::restore(unhex32(after), count_after),
                    Ok(chain),
                    "{at}: the stored chain and count after the activation"
                );
                let entry = golden["activations"]
                    .as_array()
                    .expect("chain.json activations")
                    .iter()
                    .find(|entry| entry["k"] == activation["k"])
                    .unwrap_or_else(|| {
                        panic!("{at}: chain.json has activation {}", activation["k"])
                    });
                assert_eq!(
                    text(entry, "chainHex"),
                    after,
                    "{at}: chain.json after the activation"
                );
                assert_eq!(
                    entry["commandCountAfter"].as_u64(),
                    Some(u64::from(chain.count())),
                    "{at}: chain.json's count after the activation"
                );
            }
        }
    }
    assert!(
        described >= 100,
        "the corpus's value descriptors are checked ({described})"
    );
    assert!(
        checked >= 400,
        "the corpus chains its four hundred commands or more ({checked})"
    );
}

/// The value a command carries as a descriptor, with the body's field that holds it (the body table of the corpus
/// README); none for a value carried as a blob, whose descriptor the generator states, or for a kind without one.
fn carried_value(command: &Value) -> Option<(&'static str, &Value)> {
    if command.get("valueBlob").is_some() {
        return None;
    }
    let typed = &command["typed"];
    match text(command, "kind") {
        "activity" | "script" | "child" => command.get("value").map(|value| ("input", value)),
        "approval" => command.get("value").map(|value| ("subject", value)),
        "complete" => typed.get("complete").map(|value| ("output", value)),
        "update_complete" => typed["updateResult"]
            .get("result")
            .map(|value| ("result", value)),
        "marker_side_effect" => typed["marker"]
            .get("sideEffectValue")
            .map(|value| ("value", value)),
        "fail" => typed["fail"]
            .get("safeMessage")
            .map(|value| ("message", value)),
        _ => None,
    }
}

/// `D(v)` of the corpus README: the SHA-256 and the size of the value's RFC 8785 bytes.
fn descriptor(value: &Value) -> Value {
    let bytes = rfc8785(value);
    serde_json::json!({"sha256": hex(&sha256(&bytes)), "size": bytes.len()})
}

/// Every scenario directory and run subdirectory that holds a `commands.json`.
fn scenario_runs() -> Vec<PathBuf> {
    let mut runs = Vec::new();
    let mut scenarios: Vec<PathBuf> = std::fs::read_dir(goldens().join("scenarios"))
        .expect("the scenarios")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    scenarios.sort();
    for scenario in scenarios {
        if scenario.join("commands.json").is_file() {
            runs.push(scenario.clone());
        }
        let mut subdirectories: Vec<PathBuf> = std::fs::read_dir(&scenario)
            .expect("a scenario")
            .map(|entry| entry.expect("an entry").path())
            .filter(|path| path.join("commands.json").is_file())
            .collect();
        subdirectories.sort();
        runs.extend(subdirectories);
    }
    assert!(runs.len() >= 25, "every run of the corpus ({})", runs.len());
    runs
}
