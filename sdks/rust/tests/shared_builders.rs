#![allow(clippy::expect_used)]
#[path = "../conformance/src/builders.rs"]
mod builders;
use std::path::Path;
#[test]
fn public_builders_match_existing_shared_programs_and_reviewed_goldens() {
    let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/contract/sdk-conformance");
    let mut scenarios = std::fs::read_dir(suite.join("scenarios"))
        .expect("scenario directory")
        .map(|entry| entry.expect("scenario entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    scenarios.sort();
    assert!(!scenarios.is_empty(), "shared scenarios must be present");
    for path in scenarios {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("scenario name");
        let scenario = serde_json::from_slice(
            &std::fs::read(suite.join("scenarios").join(format!("{name}.json"))).expect("scenario"),
        )
        .expect("scenario JSON");
        let expected: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                suite
                    .join("expected")
                    .join(format!("{name}.workflows.json")),
            )
            .expect("reviewed golden"),
        )
        .expect("golden JSON");
        let actual = builders::build(&scenario).expect(name);
        compare(&actual, &expected, name);
    }
}
fn compare(actual: &serde_json::Value, expected: &serde_json::Value, path: &str) {
    match (actual, expected) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            assert_eq!(
                a.keys().collect::<Vec<_>>(),
                b.keys().collect::<Vec<_>>(),
                "{path} keys"
            );
            for (key, value) in a {
                compare(value, &b[key], &format!("{path}.{key}"));
            }
        }
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{path} length");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                compare(a, b, &format!("{path}[{i}]"));
            }
        }
        (serde_json::Value::Number(a), serde_json::Value::Number(b)) => {
            assert_eq!(a.as_f64(), b.as_f64(), "{path}")
        }
        _ => assert_eq!(actual, expected, "{path}"),
    }
}
