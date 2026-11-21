//! The fixture corpus of the 0.3.0 slice (T022-002) through the production codec (T022-006, T022-069): the
//! concurrency and rate-limit declarations, one invalid document per rule of `contracts/errors.md` that names them,
//! and the rules that are not built yet, each with the task that builds it. A row added to `errors.md` fails here until
//! it is covered or listed as pending.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use sagat_application::definitions::{IrCodec, IrError, problems_of};
use sagat_domain::workflow::{DefinitionWarning, RatePeriod};
use sagat_infra_ir::JsonIrCodec;
use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        root()
            .join("tests/contract/flow-control-step-policies/fixtures")
            .join(name),
    )
    .unwrap_or_else(|error| panic!("fixture {name}: {error}"))
}

/// The problem codes and nodes of a refused document.
fn refusal(name: &str) -> Vec<(&'static str, Option<String>)> {
    match JsonIrCodec.parse(&fixture(name)) {
        Ok(_) => panic!("{name} is refused"),
        Err(error) => problems_of(&error)
            .into_iter()
            .map(|problem| (problem.code, problem.node))
            .collect(),
    }
}

/// What each invalid fixture is refused with, and the node it names.
const REFUSED: [(&str, &str, Option<&str>); 7] = [
    (
        "invalid-concurrency-policy.workflow.json",
        "CONCURRENCY_POLICY_INVALID",
        None,
    ),
    (
        "invalid-concurrency-on-a-timer.workflow.json",
        "CONCURRENCY_POLICY_INVALID",
        Some("wait"),
    ),
    (
        "invalid-rate-limit-policy.workflow.json",
        "RATE_LIMIT_POLICY_INVALID",
        None,
    ),
    (
        "invalid-rate-limit-on-a-condition.workflow.json",
        "RATE_LIMIT_POLICY_INVALID",
        Some("check"),
    ),
    (
        "invalid-flow-control-combination.workflow.json",
        "FLOW_CONTROL_COMBINATION",
        None,
    ),
    (
        "invalid-scoped-concurrency.workflow.json",
        "UNSUPPORTED_NODE",
        Some("charge"),
    ),
    (
        "invalid-scoped-rate-limit.workflow.json",
        "UNSUPPORTED_NODE",
        Some("charge"),
    ),
];

#[test]
fn every_invalid_fixture_is_refused_with_its_code_before_a_version_exists() {
    for (name, code, node) in REFUSED {
        assert_eq!(
            refusal(name),
            vec![(code, node.map(str::to_owned))],
            "{name}"
        );
    }
}

#[test]
fn a_body_node_is_refused_with_the_dependency_it_waits_for() {
    for (name, kind, dependency) in [
        (
            "invalid-scoped-concurrency.workflow.json",
            "concurrency",
            "SCOPED_CONCURRENCY",
        ),
        (
            "invalid-scoped-rate-limit.workflow.json",
            "rateLimit",
            "SCOPED_RATE_LIMIT",
        ),
    ] {
        match JsonIrCodec.parse(&fixture(name)) {
            Err(IrError::Pending {
                kind: refused,
                dependency: missing,
                ..
            }) => {
                assert_eq!(refused, kind, "{name}");
                assert_eq!(missing, dependency, "{name}");
            }
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn the_valid_fixtures_parse_to_their_declarations() {
    let parsed = JsonIrCodec
        .parse(&fixture("provider-sync.workflow.json"))
        .expect("provider-sync parses");
    let def = &parsed.definition;
    let run = def.run_flow_control();
    assert_eq!(run.concurrency().map(|policy| policy.limit()), Some(1));
    let rate = run.rate_limit().expect("a run-level rate limit");
    assert_eq!(
        (rate.limit(), rate.per(), rate.queue_timeout().as_millis()),
        (100, RatePeriod::Minute, 900_000)
    );
    let fetch = def.gated_nodes().next().expect("a gated node");
    assert_eq!(fetch.id().as_str(), "fetch");
    let step_rate = fetch
        .flow_control()
        .rate_limit()
        .expect("a step rate limit");
    assert_eq!(
        (step_rate.limit(), step_rate.per()),
        (10, RatePeriod::Second)
    );
    assert_eq!(
        step_rate.key().map(|key| key.as_str()),
        Some("input.providerId")
    );
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);

    let serial = JsonIrCodec
        .parse(&fixture("account-serial.workflow.json"))
        .expect("account-serial parses");
    assert!(serial.definition.declares_gates());
    let notify = serial
        .definition
        .gated_nodes()
        .next()
        .expect("notify is gated");
    assert_eq!(
        notify
            .flow_control()
            .concurrency()
            .and_then(|policy| policy.key())
            .map(|key| key.as_str()),
        Some("steps.price.output.region"),
        "a node-level key reads the run data a step sees"
    );
}

#[test]
fn a_self_starting_child_under_a_run_permit_is_published_with_a_warning() {
    let parsed = JsonIrCodec
        .parse(&fixture("warning-concurrency-self-child.workflow.json"))
        .expect("a warning does not refuse");
    assert!(
        parsed.warnings.iter().any(|warning| matches!(
            warning,
            DefinitionWarning::ConcurrencySelfChild { node } if node.as_str() == "again"
        )),
        "{:?}",
        parsed.warnings
    );
}

#[test]
fn the_digest_is_the_canonical_one_whatever_the_order_of_the_keys() {
    let original = JsonIrCodec
        .parse(&fixture("provider-sync.workflow.json"))
        .expect("parses");
    let mut value: Value =
        serde_json::from_slice(&fixture("provider-sync.workflow.json")).expect("json");
    // the same document with the members of every object written in reverse order
    fn reversed(value: &mut Value) {
        match value {
            Value::Object(map) => {
                let mut entries: Vec<_> = std::mem::take(map).into_iter().collect();
                entries.reverse();
                for (_, member) in &mut entries {
                    reversed(member);
                }
                map.extend(entries);
            }
            Value::Array(items) => items.iter_mut().for_each(reversed),
            _ => {}
        }
    }
    reversed(&mut value);
    let again = JsonIrCodec
        .parse(&serde_json::to_vec(&value).expect("bytes"))
        .expect("parses");
    assert_eq!(again.digest, original.digest);
    assert_eq!(again.canonical, original.canonical);
}

#[test]
fn a_stored_definition_is_read_with_its_gates_even_where_publication_refuses_it() {
    // what a publication refuses across the definition is not applied again on restore (FR-022-059): the one other
    // writer of runs, a continuation successor, checks the definition it is given itself
    let stored = JsonIrCodec
        .parse_stored(&fixture("invalid-flow-control-combination.workflow.json"))
        .expect("restored");
    assert!(stored.definition.run_flow_control().is_gated());
    assert!(stored.definition.continuation_policy().is_some());
}

#[test]
fn the_corpus_that_declares_no_gate_reads_as_before() {
    let mut seen = 0;
    for directory in ["examples", "tests/contract/workflow-primitives/fixtures"] {
        let Ok(entries) = fs::read_dir(root().join(directory)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.to_string_lossy().ends_with(".workflow.json") {
                continue;
            }
            let bytes = fs::read(&path).expect("read");
            if let Ok(parsed) = JsonIrCodec.parse(&bytes) {
                seen += 1;
                assert!(
                    !parsed.definition.declares_gates(),
                    "{} declares no gate",
                    path.display()
                );
                assert!(
                    !parsed.warnings.iter().any(|warning| matches!(
                        warning,
                        DefinitionWarning::ConcurrencySelfChild { .. }
                    )),
                    "{}",
                    path.display()
                );
            }
        }
    }
    assert!(seen >= 1, "the corpus was read");
}

/// The rules of `contracts/errors.md` ("Publication and validation") that name concurrency or rate limits and that
/// the codec does not refuse yet, with the task that builds each and adds its fixture to this corpus. Every other such
/// rule has a fixture in [`REFUSED`], is the warning of
/// [`a_self_starting_child_under_a_run_permit_is_published_with_a_warning`], or is a namespace warning of
/// [`NAMESPACE_WARNINGS`].
const PENDING: [(&str, &str); 4] = [
    ("FLOW_CONTROL_KEY_UNBOUNDED", "T022-103"),
    ("FLOW_CONTROL_KEY_SECRET", "T022-104"),
    ("FLOW_CONTROL_KEY_PII", "T022-104"),
    (
        "ENCODING_READ_BY_CONCURRENCY_KEY",
        "feature 031 registers it (FR-022-036)",
    ),
];

/// The warnings that compare a definition with the namespace's other published definitions (G1187): one document
/// cannot show them, so they are proved against PostgreSQL by the named test of `integration_flow_control_step_policies`.
const NAMESPACE_WARNINGS: [(&str, &str); 2] = [
    (
        "CONCURRENCY_LIMIT_MISMATCH",
        "publication::a_group_declared_elsewhere_with_another_limit_is_warned_and_nothing_else_is",
    ),
    (
        "RATE_LIMIT_MISMATCH",
        "publication::a_group_declared_elsewhere_with_another_limit_is_warned_and_nothing_else_is",
    ),
];

#[test]
fn every_rule_that_names_a_gate_is_covered_or_pending() {
    let errors =
        fs::read_to_string(root().join("specs/022-flow-control-step-policies/contracts/errors.md"))
            .expect("errors.md");
    let table = errors
        .split("## Publication and validation")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("the publication table");
    let named: BTreeSet<String> = table
        .lines()
        .filter(|line| line.starts_with("| `"))
        .filter(|line| {
            let lower = line.to_lowercase();
            lower.contains("concurrency")
                || lower.contains("ratelimit")
                || lower.contains("rate limit")
                || lower.contains("flow_control")
                || lower.contains("rate_limit")
        })
        .filter_map(|line| line.split('`').nth(1).map(str::to_owned))
        .collect();
    let covered: BTreeSet<String> = REFUSED
        .iter()
        .map(|(_, code, _)| (*code).to_owned())
        .chain(["CONCURRENCY_SELF_CHILD".to_owned()])
        .chain(PENDING.iter().map(|(code, _)| (*code).to_owned()))
        .chain(
            NAMESPACE_WARNINGS
                .iter()
                .map(|(code, _)| (*code).to_owned()),
        )
        .collect();
    let missing: Vec<_> = named.difference(&covered).collect();
    assert!(
        missing.is_empty(),
        "rules of errors.md with neither a fixture nor a pending entry: {missing:?}"
    );
    for (code, _) in PENDING.iter().chain(&NAMESPACE_WARNINGS) {
        assert!(named.contains(*code), "{code} is a rule of errors.md");
    }
    let suite = fs::read_to_string(
        root().join("tests/integration/flow-control-step-policies/publication.rs"),
    )
    .expect("the publication suite");
    for (code, test) in NAMESPACE_WARNINGS {
        let name = test.rsplit("::").next().expect("a name");
        let start = suite
            .find(&format!("async fn {name}()"))
            .unwrap_or_else(|| panic!("{test} exists"));
        let body = &suite[start..];
        let body = &body[..body.find("\n#[tokio::test]").unwrap_or(body.len())];
        assert!(
            body.contains(&format!("\"{code}\"")),
            "{test} asserts {code} itself"
        );
    }
}
