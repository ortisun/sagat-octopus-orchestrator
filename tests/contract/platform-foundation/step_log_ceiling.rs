//! The closed set of a step-log stream's `ceiling` reason is the same in the planned contract and in the domain
//! (design 041, decision D041-121; task T041-5 extends it to the migration's `ceiling_reason` CHECK).

use std::collections::BTreeSet;
use std::path::Path;

use sagat_domain::step_logs::CeilingReason;
use serde_json::Value;

#[test]
fn the_contracts_ceiling_reasons_are_the_domains() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../specs/001-platform-foundation/contracts/openapi.yaml");
    let document: Value =
        serde_saphyr::from_str(&std::fs::read_to_string(&path).expect("the contract exists"))
            .expect("the contract is YAML");
    let contract: BTreeSet<String> = document
        .pointer("/components/schemas/StepLogStream/properties/ceiling/enum")
        .and_then(Value::as_array)
        .expect("StepLogStream.ceiling has an enum")
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
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
