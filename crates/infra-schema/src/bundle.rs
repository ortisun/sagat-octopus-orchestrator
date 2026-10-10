//! The artifact bundle, as a runner reports it and as a runner loads it: a canonical JSON document of
//! the code, the policy it was built under and the schemas of its input and result.
//!
//! ```json
//! { "v": 1, "language": "typescript", "policy": "v1;memory=…", "inputSchema": null,
//!   "outputSchema": null, "entry": "main.mjs", "files": { "main.mjs": "…" } }
//! ```

use sagat_application::scripts::{BundleChecker, Expected};
use sagat_domain::scripts::Language;
use serde_json::Value;

/// Files a bundle may hold.
pub const MAX_FILES: usize = 32;

/// The files of a bundle, if it is one for `language`: version 1, at most [`MAX_FILES`] files, each
/// with a plain name (letters, digits, `.`, `_`, `-`; up to 64; not starting with a dot).
#[must_use]
pub fn files(bytes: &[u8], language: Language) -> Option<Vec<(String, String)>> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    if value.get("v")?.as_u64()? != 1 || value.get("language")?.as_str()? != language.as_str() {
        return None;
    }
    let files = value.get("files")?.as_object()?;
    if files.is_empty() || files.len() > MAX_FILES {
        return None;
    }
    files
        .iter()
        .map(|(name, content)| {
            let safe = !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
                && !name.starts_with('.');
            safe.then(|| {
                (
                    name.clone(),
                    content.as_str().unwrap_or_default().to_owned(),
                )
            })
        })
        .collect()
}

/// Checks that a bundle says what the version it was built from says.
#[derive(Debug, Clone, Copy, Default)]
pub struct Bundles;

impl BundleChecker for Bundles {
    fn check(&self, bundle: &[u8], expected: &Expected<'_>) -> Result<(), &'static str> {
        let value: Value =
            serde_json::from_slice(bundle).map_err(|_| "the artifact is not a bundle")?;
        let listed = files(bundle, expected.language)
            .ok_or("the artifact is not a bundle of this language")?;
        let entry = value
            .get("entry")
            .and_then(Value::as_str)
            .ok_or("the artifact names no entry point")?;
        if !listed.iter().any(|(name, _)| name == entry) {
            return Err("the entry point of the artifact is not one of its files");
        }
        if value.get("policy").and_then(Value::as_str) != Some(expected.policy.canonical().as_str())
        {
            return Err("the artifact was built under another policy than the version's");
        }
        let schema = |bytes: Option<&[u8]>| -> Result<Option<Value>, &'static str> {
            bytes
                .map(|b| serde_json::from_slice(b).map_err(|_| "the version's schema is not JSON"))
                .transpose()
        };
        for (key, wanted) in [
            ("inputSchema", expected.input_schema),
            ("outputSchema", expected.output_schema),
        ] {
            let carried = value.get(key).filter(|v| !v.is_null());
            if carried != schema(wanted)?.as_ref() {
                return Err("the artifact carries other schemas than the version's");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sagat_domain::scripts::Policy;
    use serde_json::json;

    fn bundle(policy: &Policy, extra: Value) -> Vec<u8> {
        let mut value = json!({
            "v": 1, "language": "typescript", "policy": policy.canonical(),
            "inputSchema": null, "outputSchema": null, "entry": "main.mjs",
            "files": {"main.mjs": "export default () => 1"}
        });
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            value[k] = v;
        }
        serde_json::to_vec(&value).expect("json")
    }

    fn expected<'a>(policy: &'a Policy, input: Option<&'a [u8]>) -> Expected<'a> {
        Expected {
            language: Language::TypeScript,
            policy,
            input_schema: input,
            output_schema: None,
        }
    }

    #[test]
    fn a_bundle_that_says_what_its_version_says_is_accepted() {
        let policy = Policy::default();
        assert_eq!(
            Bundles.check(&bundle(&policy, json!({})), &expected(&policy, None)),
            Ok(())
        );
        let schema = br#"{"type":"object"}"#;
        assert_eq!(
            Bundles.check(
                &bundle(&policy, json!({"inputSchema": {"type": "object"}})),
                &expected(&policy, Some(schema))
            ),
            Ok(())
        );
    }

    #[test]
    fn a_bundle_that_says_anything_else_is_refused_before_it_is_signed() {
        let policy = Policy::default();
        let wider = Policy {
            egress: sagat_domain::scripts::Egress::Namespace,
            ..Policy::default()
        };
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("not json", b"nope".to_vec()),
            ("another policy", bundle(&wider, json!({}))),
            (
                "another language",
                bundle(&policy, json!({"language": "python"})),
            ),
            ("another version", bundle(&policy, json!({"v": 2}))),
            (
                "schemas the version has not",
                bundle(&policy, json!({"inputSchema": {"type": "string"}})),
            ),
            ("no entry", bundle(&policy, json!({"entry": "other.mjs"}))),
            (
                "a path as a file name",
                bundle(&policy, json!({"files": {"../x": "1"}, "entry": "../x"})),
            ),
            ("no files", bundle(&policy, json!({"files": {}}))),
        ];
        for (what, bytes) in cases {
            assert!(
                Bundles.check(&bytes, &expected(&policy, None)).is_err(),
                "{what}"
            );
        }
        // A version with a schema is not built into a bundle without it.
        assert!(
            Bundles
                .check(
                    &bundle(&policy, json!({})),
                    &expected(&policy, Some(br#"{"type":"object"}"#))
                )
                .is_err()
        );
    }
}
