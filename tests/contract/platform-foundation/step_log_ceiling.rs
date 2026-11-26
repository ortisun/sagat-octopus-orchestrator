//! The closed sets of a step-log stream's `ceiling` and `notStoredReason` are the same in the planned contract and in
//! the domain (design 041, task T041-4; decisions D041-121 and D041-129: the busy loss is `notStoredReason`, never a
//! ceiling), and in the CHECKs of the migration that stores them (task T041-5).

use std::collections::BTreeSet;
use std::path::Path;

use sagat_domain::step_logs::{CeilingReason, NotStoredReason};
use serde_json::Value;

fn contract_enum(property: &str) -> BTreeSet<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../specs/001-platform-foundation/contracts/openapi.yaml");
    let document: Value =
        serde_saphyr::from_str(&std::fs::read_to_string(&path).expect("the contract exists"))
            .expect("the contract is YAML");
    document
        .pointer(&format!(
            "/components/schemas/StepLogStream/properties/{property}/enum"
        ))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("StepLogStream.{property} has an enum"))
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

#[test]
fn the_contracts_not_stored_reasons_are_the_domains() {
    let contract = contract_enum("notStoredReason");
    let domain: BTreeSet<String> = NotStoredReason::ALL
        .iter()
        .map(|reason| reason.as_str().to_owned())
        .collect();
    assert_eq!(contract, domain, "the contract's not-stored reasons");
    for name in &contract {
        assert_eq!(
            NotStoredReason::parse(name).map(NotStoredReason::as_str),
            Some(name.as_str()),
            "{name} parses"
        );
    }
}

#[test]
fn the_contracts_ceiling_reasons_are_the_domains() {
    let contract = contract_enum("ceiling");
    let domain: BTreeSet<String> = CeilingReason::ALL
        .iter()
        .map(|reason| reason.as_str().to_owned())
        .collect();
    assert_eq!(contract, domain, "the contract's ceiling reasons");
    for name in &contract {
        assert_eq!(
            CeilingReason::parse(name).map(CeilingReason::as_str),
            Some(name.as_str()),
            "{name} parses"
        );
    }
}

/// The values a CHECK of the step-log migration allows: the quoted names of `IN (...)` after `CONSTRAINT <name>`.
fn migration_check(name: &str) -> BTreeSet<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/infra-postgres/migrations/20270210000000_step_logs.up.sql");
    let sql = std::fs::read_to_string(&path).expect("the step-log migration exists");
    let start = sql
        .find(&format!("CONSTRAINT {name} CHECK"))
        .unwrap_or_else(|| panic!("the migration has the CHECK {name}"));
    let rest = &sql[start..];
    let open = rest.find(" IN").expect("an IN list") + 3;
    let list = &rest[open..];
    let list = &list[list.find('(').expect("(") + 1..list.find(')').expect(")")];
    list.split(',')
        .map(|value| value.trim().trim_matches('\'').to_owned())
        .collect()
}

#[test]
fn the_migrations_ceiling_and_not_stored_reasons_are_the_domains() {
    let ceilings: BTreeSet<String> = CeilingReason::ALL
        .iter()
        .map(|reason| reason.as_str().to_owned())
        .collect();
    assert_eq!(migration_check("step_log_streams_ceiling_valid"), ceilings);
    let not_stored: BTreeSet<String> = NotStoredReason::ALL
        .iter()
        .map(|reason| reason.as_str().to_owned())
        .collect();
    assert_eq!(
        migration_check("step_log_streams_not_stored_valid"),
        not_stored
    );
    assert!(
        ceilings.is_disjoint(&not_stored),
        "the busy loss is never a ceiling (decision D041-129)"
    );
}
