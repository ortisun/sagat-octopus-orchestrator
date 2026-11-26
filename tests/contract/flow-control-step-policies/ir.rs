//! The fixture corpus of the 0.3.0 slice (T022-002) through the production codec (T022-006, T022-069): the
//! concurrency and rate-limit declarations, one invalid document per rule of `contracts/errors.md` that names them,
//! and the rules that are not built yet, each with the task that builds it. A row added to `errors.md` fails here until
//! it is covered or listed as pending.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use sagat_application::definitions::{IrCodec, IrError, problems_of};
use sagat_domain::classification::Level;
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
const REFUSED: [(&str, &str, Option<&str>); 9] = [
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
    (
        "invalid-flow-control-key-unbounded.workflow.json",
        "FLOW_CONTROL_KEY_UNBOUNDED",
        Some("call"),
    ),
    (
        "invalid-flow-control-key-secret.workflow.json",
        "FLOW_CONTROL_KEY_SECRET",
        Some("call"),
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

/// FR-022-056 (task T022-103, decision D022-081): a step key that reads the run's identity, alone or beside other data,
/// at either gate, is refused `FLOW_CONTROL_KEY_UNBOUNDED` at its node; a key over the run's data that does not read it
/// is accepted, and a top-level key cannot read it at all (it reads only `input`).
#[test]
fn a_key_that_reads_the_runs_identity_is_refused_as_unbounded() {
    let document = |gate: &str, key: &str| {
        let mut policy = serde_json::json!({"key": key, "limit": 1});
        if gate == "rateLimit" {
            policy["per"] = serde_json::json!("second");
        }
        let mut call =
            serde_json::json!({"id": "call", "type": "activity", "activity": "provider.call"});
        call[gate] = policy;
        serde_json::to_vec(&serde_json::json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1",
            "kind": "Workflow",
            "metadata": {"name": "keys"},
            "entrypoint": "call",
            "nodes": [call]
        }))
        .expect("json")
    };
    for gate in ["concurrency", "rateLimit"] {
        for key in ["run.id", "input.accountId + run.id", "run"] {
            let codes: Vec<_> = match JsonIrCodec.parse(&document(gate, key)) {
                Ok(_) => panic!("{gate} {key} is refused"),
                Err(error) => problems_of(&error)
                    .into_iter()
                    .map(|problem| (problem.code, problem.node))
                    .collect(),
            };
            assert_eq!(
                codes,
                vec![("FLOW_CONTROL_KEY_UNBOUNDED", Some("call".to_owned()))],
                "{gate} {key}"
            ); // a stored version is restored as it was published (an earlier build may have published it): the rule is
            // publication's only (review of 2026-11-25, correctness minor 1, security nit 1)
            JsonIrCodec
                .parse_stored(&document(gate, key))
                .unwrap_or_else(|error| panic!("a stored {gate} {key} is restored: {error:?}"));
        }
        for key in ["input.accountId", "input.accountId + '-' + input.region"] {
            JsonIrCodec
                .parse(&document(gate, key))
                .unwrap_or_else(|error| panic!("{gate} {key}: {error:?}"));
        }
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
    // no schema classifies its input, so each key reads `pii` (016: an unannotated location is `pii`): published, warned
    // at each declaration (FR-022-057, decision D022-085)
    assert_eq!(
        parsed
            .warnings
            .iter()
            .map(|warning| sagat_application::definitions::warning_problem(warning).code)
            .collect::<Vec<_>>(),
        [
            "FLOW_CONTROL_KEY_PII",
            "FLOW_CONTROL_KEY_PII",
            "FLOW_CONTROL_KEY_PII"
        ],
        "{:?}",
        parsed.warnings
    );

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

/// FR-022-057 (task T022-104, decision D022-085): a key over a `pii` location is published with a warning at its
/// declaration, and one over `internal` locations only is not.
#[test]
fn a_key_over_personal_data_is_published_with_a_warning_and_one_over_internal_data_is_not() {
    let parsed = JsonIrCodec
        .parse(&fixture("warning-flow-control-key-pii.workflow.json"))
        .expect("a warning does not refuse");
    let problems: Vec<_> = parsed
        .warnings
        .iter()
        .map(sagat_application::definitions::warning_problem)
        .map(|problem| (problem.code, problem.node))
        .collect();
    assert_eq!(
        problems,
        [("FLOW_CONTROL_KEY_PII", None)],
        "{:?}",
        parsed.warnings
    );
    let def = &parsed.definition;
    assert_eq!(
        def.run_flow_control()
            .concurrency()
            .map(sagat_domain::workflow::ConcurrencyPolicy::key_level),
        Some(Level::Pii)
    );
    assert_eq!(
        def.gated_nodes()
            .next()
            .and_then(|node| node.flow_control().rate_limit())
            .map(sagat_domain::workflow::RateLimitPolicy::key_level),
        Some(Level::Internal),
        "`region` inherits the input's `internal`"
    );
}

/// A document whose input, `price` output and gated `call` node are given; `key` is the `call` node's concurrency key.
fn classified(key: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": "classified-keys"},
        "entrypoint": "price",
        "inputSchema": {
            "type": "object",
            "x-sagat-classification": "public",
            "$defs": {"token": {"type": "string", "x-sagat-classification": "secret"}},
            "properties": {
                "tier": {"type": "string"},
                "team": {"type": "string", "x-sagat-classification": "internal"},
                "email": {"type": "string", "x-sagat-classification": "pii"},
                "token": {"$ref": "#/$defs/token"},
                "either": {"anyOf": [
                    {"type": "string"},
                    {"type": "string", "x-sagat-classification": "secret"}
                ]},
                "account": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "key": {"type": "string", "x-sagat-classification": "secret"}
                    }
                },
                "tags": {"type": "array", "items": {"type": "string", "x-sagat-classification": "internal"}},
                "extra": {"type": "object", "additionalProperties": {"type": "string", "x-sagat-classification": "secret"}}
            }
        },
        "nodes": [
            {
                "id": "price", "type": "activity", "activity": "catalog.price", "next": "call",
                "outputSchema": {
                    "type": "object",
                    "x-sagat-classification": "internal",
                    "properties": {
                        "region": {"type": "string"},
                        "card": {"type": "string", "x-sagat-classification": "secret"}
                    }
                }
            },
            {"id": "call", "type": "activity", "activity": "provider.call", "concurrency": {"key": key, "limit": 1}}
        ]
    }))
    .expect("json")
}

/// FR-022-057 over the carriers of feature 016 (FR-016-002, FR-016-003; task T022-104, decision D022-085): the input,
/// a root node's output, a `$ref`, the strictest branch of an `anyOf`, a structure read whole, a computed index, an
/// `additionalProperties`, the declared state (no carrier) and a location no schema declares. A key over a `secret`
/// location is refused at its node; any other has the strictest level it reads, `pii` warned; a stored version is
/// restored with its level as published, whose label nobody may see.
#[test]
fn a_key_has_the_strictest_level_of_the_locations_it_reads_over_the_016_carriers() {
    for (key, expected) in [
        ("input.tier", Some(Level::Public)),
        ("input.team + '/' + input.tier", Some(Level::Internal)),
        ("input.tags[0]", Some(Level::Internal)),
        ("steps.price.output.region", Some(Level::Internal)),
        ("input.account.id", Some(Level::Public)),
        ("input.email", Some(Level::Pii)),
        ("input.undeclared", Some(Level::Pii)),
        // no carrier of its own: the strictest level of every carrier of the document (batch 4 reviews, security
        // Major 2), here `secret`
        ("state.counter", None),
        ("steps.missing.output.region", None),
        ("steps", None),
        ("'one-key'", Some(Level::Public)),
        // a structure read whole holds a secret below it; so does a list read at a computed position
        ("string(size(input.account))", None),
        ("input.token", None),
        ("input.either", None),
        ("input.extra.anything", None),
        ("steps.price.output.card", None),
        ("steps.price.output[input.tier]", None),
        ("input.team + steps.price.output.card", None),
    ] {
        let document = classified(key);
        match (JsonIrCodec.parse(&document), expected) {
            (Ok(parsed), Some(level)) => {
                let read = parsed
                    .definition
                    .gated_nodes()
                    .next()
                    .and_then(|node| node.flow_control().concurrency())
                    .map(sagat_domain::workflow::ConcurrencyPolicy::key_level);
                assert_eq!(read, Some(level), "{key}");
                let warned = parsed.warnings.iter().any(|warning| {
                    sagat_application::definitions::warning_problem(warning).code
                        == "FLOW_CONTROL_KEY_PII"
                });
                assert_eq!(warned, level == Level::Pii, "{key}: {:?}", parsed.warnings);
            }
            (Err(error), None) => {
                let codes: Vec<_> = problems_of(&error)
                    .into_iter()
                    .map(|problem| (problem.code, problem.node))
                    .collect();
                assert_eq!(
                    codes,
                    [("FLOW_CONTROL_KEY_SECRET", Some("call".to_owned()))],
                    "{key}"
                );
                let stored = JsonIrCodec
                    .parse_stored(&document)
                    .unwrap_or_else(|error| panic!("a stored {key} is restored: {error:?}"));
                assert_eq!(
                    stored
                        .definition
                        .gated_nodes()
                        .next()
                        .and_then(|node| node.flow_control().concurrency())
                        .map(sagat_domain::workflow::ConcurrencyPolicy::key_level),
                    Some(Level::Secret),
                    "{key}: restored with the level it reads"
                );
            }
            (outcome, expected) => panic!("{key}: {outcome:?}, expected {expected:?}"),
        }
    }
    // a top-level key reads only `input`; a secret there is refused at the workflow
    let mut top: Value = serde_json::from_slice(&classified("input.tier")).expect("json");
    top["rateLimit"] = serde_json::json!({"key": "input.token", "limit": 1, "per": "second"});
    match JsonIrCodec.parse(&serde_json::to_vec(&top).expect("bytes")) {
        Err(error) => assert_eq!(
            problems_of(&error)
                .into_iter()
                .map(|problem| (problem.code, problem.node))
                .collect::<Vec<_>>(),
            [("FLOW_CONTROL_KEY_SECRET", None)]
        ),
        Ok(_) => panic!("a top-level key over a secret is refused"),
    }
}

/// Batch 4 reviews of 2026-11-25, security Major 2: a value that reaches a location without a carrier of its own takes
/// the strictest level of every carrier of the document, never less than `pii`. A `database` node's output has no
/// carrier (its rows are whatever the database holds): beside an input with a `secret` field, a key over that output is
/// refused; beside carriers that hold nothing above `internal`, it is `pii`, warned. (A `forEach` that returns a body's
/// output is the reviewer's case; a gate beside a `forEach` is not published by this build, `UNSUPPORTED_NODE`, and the
/// same rule reads it.)
#[test]
fn a_key_over_a_node_without_its_own_carrier_takes_the_strictest_level_of_the_document() {
    let document = |level: &str, key: &str| {
        serde_json::to_vec(&serde_json::json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
            "metadata": {"name": "carrierless"}, "entrypoint": "read",
            "inputSchema": {"type": "object", "x-sagat-classification": "public", "properties": {
                "token": {"type": "string", "x-sagat-classification": level}}},
            "nodes": [
                {"id": "read", "type": "database", "connectionRef": {"name": "orders-db"}, "operation": "query",
                 "statement": "SELECT id FROM orders", "next": "call"},
                {"id": "call", "type": "activity", "activity": "provider.call",
                 "concurrency": {"key": key, "limit": 1}}
            ]
        }))
        .expect("json")
    };
    for key in [
        "steps.read.output.rows[0].id",
        "string(steps.read)",
        "string(steps)",
    ] {
        match JsonIrCodec.parse(&document("secret", key)) {
            Err(error) => assert_eq!(
                problems_of(&error)
                    .into_iter()
                    .map(|problem| (problem.code, problem.node))
                    .collect::<Vec<_>>(),
                [("FLOW_CONTROL_KEY_SECRET", Some("call".to_owned()))],
                "{key}"
            ),
            Ok(parsed) => panic!(
                "{key}: what reaches a node without a carrier may be the input's secret: {:?}",
                parsed.warnings
            ),
        }
        let parsed = JsonIrCodec
            .parse(&document("internal", key))
            .unwrap_or_else(|error| panic!("{key}: {error:?}"));
        let level = parsed
            .definition
            .gated_nodes()
            .next()
            .and_then(|node| node.flow_control().concurrency())
            .map(sagat_domain::workflow::ConcurrencyPolicy::key_level);
        assert_eq!(level, Some(Level::Pii), "{key}: never less than pii");
    }
}

/// Batch 4 reviews of 2026-11-25, the security suspicion: a carrier whose classification walk did not complete (an
/// annotation reached through a keyword the walk does not follow, here a `$ref` to a `secret` field under `not`) cannot
/// be proved not to hold a secret. Publication refuses such a schema already (`CLASSIFICATION_UNSUPPORTED_KEYWORD`); a
/// stored version restored with it gives a key over any location of that carrier the level `secret`, whose label nobody
/// may see, even one declared `public`.
#[test]
fn a_key_over_a_carrier_whose_walk_did_not_complete_counts_as_secret() {
    let mut document: Value = serde_json::from_slice(&classified("input.tier")).expect("json");
    document["inputSchema"]["properties"]["opaque"] =
        serde_json::json!({"type": "string", "not": {"$ref": "#/$defs/token"}});
    let bytes = serde_json::to_vec(&document).expect("bytes");
    assert!(
        JsonIrCodec.parse(&bytes).is_err(),
        "publication refuses the schema"
    );
    let stored = JsonIrCodec
        .parse_stored(&bytes)
        .unwrap_or_else(|error| panic!("a stored version is restored: {error:?}"));
    assert_eq!(
        stored
            .definition
            .gated_nodes()
            .next()
            .and_then(|node| node.flow_control().concurrency())
            .map(sagat_domain::workflow::ConcurrencyPolicy::key_level),
        Some(Level::Secret)
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
const PENDING: [(&str, &str); 1] = [(
    "ENCODING_READ_BY_CONCURRENCY_KEY",
    "feature 031 registers it (FR-022-036)",
)];

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
        .chain([
            "CONCURRENCY_SELF_CHILD".to_owned(),
            "FLOW_CONTROL_KEY_PII".to_owned(),
        ])
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
