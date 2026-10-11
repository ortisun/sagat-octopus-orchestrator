//! What a lockfile pins. TypeScript and Python remain framework-only. C# may reference only the
//! exact managed packages already present in its immutable builder image: the standard NuGet lock
//! must match the closed catalog by target, id, version and content hash, with no registry access
//! while a customer build runs.

use sagat_domain::scripts::{Language, Lockfile};
use serde_json::Value;

pub const NEWTONSOFT_JSON_ID: &str = "Newtonsoft.Json";
pub const NEWTONSOFT_JSON_VERSION: &str = "13.0.4";
pub const NEWTONSOFT_JSON_CONTENT_HASH: &str =
    "pdgNNMai3zv51W5aq268sujXUyx7SNdE2bj1wZcWjAQrKMFZV260lbqYop1d2GM67JI1huLRwxo9ZqnfF/lC6A==";

/// A package whose exact bytes are present in the immutable C# builder image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedPackage {
    pub id: String,
    pub version: String,
    pub content_hash: String,
}

fn csharp_dependencies(value: &Value) -> Result<&serde_json::Map<String, Value>, String> {
    let root = value
        .as_object()
        .ok_or_else(|| "a packages.lock.json is a JSON object".to_owned())?;
    if root.len() != 2 || root.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("a C# lockfile has only version 1 and dependencies".to_owned());
    }
    let groups = root
        .get("dependencies")
        .and_then(Value::as_object)
        .ok_or_else(|| "a C# lockfile has dependency groups".to_owned())?;
    if groups.len() != 1 || !groups.contains_key("net10.0") {
        return Err("the C# lockfile must have exactly the net10.0 dependency group".to_owned());
    }
    groups
        .get("net10.0")
        .and_then(Value::as_object)
        .ok_or_else(|| "the net10.0 dependency group is an object".to_owned())
}

fn approved_csharp(lockfile: &Lockfile) -> Result<Vec<ApprovedPackage>, String> {
    let value: Value = serde_json::from_str(lockfile.as_str().trim())
        .map_err(|_| "the lockfile is not valid JSON".to_owned())?;
    let dependencies = csharp_dependencies(&value)?;
    if dependencies.is_empty() {
        return Ok(Vec::new());
    }
    if dependencies.len() != 1 || !dependencies.contains_key(NEWTONSOFT_JSON_ID) {
        return Err("the lockfile names a package outside the immutable C# catalog".to_owned());
    }
    let package = dependencies
        .get(NEWTONSOFT_JSON_ID)
        .and_then(Value::as_object)
        .ok_or_else(|| "a locked package is an object".to_owned())?;
    let expected_keys = ["type", "requested", "resolved", "contentHash"];
    if package.len() != expected_keys.len()
        || expected_keys.iter().any(|key| !package.contains_key(*key))
        || package.get("type").and_then(Value::as_str) != Some("Direct")
        || package.get("requested").and_then(Value::as_str) != Some("[13.0.4, 13.0.4]")
        || package.get("resolved").and_then(Value::as_str) != Some(NEWTONSOFT_JSON_VERSION)
        || package.get("contentHash").and_then(Value::as_str) != Some(NEWTONSOFT_JSON_CONTENT_HASH)
    {
        return Err(
            "Newtonsoft.Json must be a direct exact 13.0.4 package with its approved SHA-512"
                .to_owned(),
        );
    }
    Ok(vec![ApprovedPackage {
        id: NEWTONSOFT_JSON_ID.to_owned(),
        version: NEWTONSOFT_JSON_VERSION.to_owned(),
        content_hash: NEWTONSOFT_JSON_CONTENT_HASH.to_owned(),
    }])
}

/// The packages `lockfile` pins.
///
/// # Errors
/// A description of why the text is not a lockfile of `language`.
pub fn packages(language: Language, lockfile: &Lockfile) -> Result<Vec<String>, String> {
    let text = lockfile.as_str().trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    match language {
        Language::TypeScript => {
            let value: Value = serde_json::from_str(text)
                .map_err(|_| "the lockfile is not valid JSON".to_owned())?;
            let object = value
                .as_object()
                .ok_or_else(|| "a package-lock.json is a JSON object".to_owned())?;
            let mut found: Vec<String> = Vec::new();
            if let Some(packages) = object.get("packages").and_then(Value::as_object) {
                found.extend(packages.keys().filter(|k| !k.is_empty()).cloned());
            }
            if let Some(dependencies) = object.get("dependencies").and_then(Value::as_object) {
                found.extend(dependencies.keys().cloned());
            }
            found.sort();
            found.dedup();
            Ok(found)
        }
        Language::Python => Ok(text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("--"))
            .map(|l| {
                l.split(['=', '<', '>', '~', '!', ' ', ';'])
                    .next()
                    .unwrap_or(l)
                    .to_owned()
            })
            .collect()),
        Language::CSharp => {
            let value: Value = serde_json::from_str(text)
                .map_err(|_| "the lockfile is not valid JSON".to_owned())?;
            Ok(csharp_dependencies(&value)?.keys().cloned().collect())
        }
        _ => Err("no lockfile format is known for this language".to_owned()),
    }
}

/// Whether the build may go on: the lockfile pins nothing.
///
/// # Errors
/// Why not.
pub fn accept(language: Language, lockfile: Option<&Lockfile>) -> Result<(), String> {
    let Some(lockfile) = lockfile else {
        return Ok(());
    };
    if lockfile.as_str().trim().is_empty() {
        return Ok(());
    }
    if language == Language::CSharp {
        return approved_csharp(lockfile).map(drop);
    }
    let pinned = packages(language, lockfile)?;
    if pinned.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the lockfile pins {} package(s) ({}); installing packages from a registry is not available yet, \
             so a script can only use what its language provides",
            pinned.len(),
            pinned
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// Packages the C# builder may reference. Every returned package has already been matched against
/// the closed image catalog; other languages currently accept framework-only builds.
pub fn approved(
    language: Language,
    lockfile: Option<&Lockfile>,
) -> Result<Vec<ApprovedPackage>, String> {
    let Some(lockfile) = lockfile else {
        return Ok(Vec::new());
    };
    if lockfile.as_str().trim().is_empty() {
        return Ok(Vec::new());
    }
    if language == Language::CSharp {
        approved_csharp(lockfile)
    } else {
        accept(language, Some(lockfile)).map(|()| Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock(text: &str) -> Lockfile {
        Lockfile::parse(text).expect("lockfile")
    }

    #[test]
    fn a_lockfile_that_pins_nothing_is_a_lockfile_and_is_accepted() {
        for (language, text) in [
            (Language::TypeScript, ""),
            (
                Language::TypeScript,
                r#"{"lockfileVersion":3,"packages":{"":{"name":"x"}}}"#,
            ),
            (
                Language::Python,
                "# nothing\n\n--index-url https://pypi.org/simple\n",
            ),
        ] {
            assert_eq!(accept(language, Some(&lock(text))), Ok(()), "{text}");
        }
        assert_eq!(accept(Language::Python, None), Ok(()));
    }

    #[test]
    fn a_lockfile_that_pins_a_package_is_refused_with_the_names_and_not_ignored() {
        let npm = lock(
            r#"{"lockfileVersion":3,"packages":{"":{},"node_modules/left-pad":{"version":"1.3.0"}}}"#,
        );
        let reason = accept(Language::TypeScript, Some(&npm)).expect_err("refused");
        assert!(reason.contains("node_modules/left-pad"), "{reason}");
        let pip = lock("requests==2.32.0 --hash=sha256:abc\nflask>=3\n");
        let reason = accept(Language::Python, Some(&pip)).expect_err("refused");
        assert!(
            reason.contains("requests") && reason.contains("flask"),
            "{reason}"
        );
    }

    #[test]
    fn what_is_not_a_lockfile_is_said_so() {
        assert!(accept(Language::TypeScript, Some(&lock("not json"))).is_err());
        assert!(accept(Language::TypeScript, Some(&lock("[1]"))).is_err());
        assert!(accept(Language::Go, Some(&lock("x"))).is_err());
    }

    #[test]
    fn an_exact_allowlisted_csharp_package_is_accepted_offline() {
        let packages = lock(
            r#"{
              "version": 1,
              "dependencies": {
                "net10.0": {
                  "Newtonsoft.Json": {
                    "type": "Direct",
                    "requested": "[13.0.4, 13.0.4]",
                    "resolved": "13.0.4",
                    "contentHash": "pdgNNMai3zv51W5aq268sujXUyx7SNdE2bj1wZcWjAQrKMFZV260lbqYop1d2GM67JI1huLRwxo9ZqnfF/lC6A=="
                  }
                }
              }
            }"#,
        );

        assert_eq!(
            approved(Language::CSharp, Some(&packages)),
            Ok(vec![ApprovedPackage {
                id: NEWTONSOFT_JSON_ID.to_owned(),
                version: NEWTONSOFT_JSON_VERSION.to_owned(),
                content_hash: NEWTONSOFT_JSON_CONTENT_HASH.to_owned(),
            }])
        );
        assert_eq!(accept(Language::CSharp, Some(&packages)), Ok(()));
        assert_eq!(accept(Language::CSharp, None), Ok(()));
        assert_eq!(accept(Language::CSharp, Some(&lock(""))), Ok(()));
    }

    #[test]
    fn a_csharp_lockfile_cannot_float_substitute_or_hide_a_package() {
        let base = serde_json::json!({
            "version": 1,
            "dependencies": {"net10.0": {
                "Newtonsoft.Json": {
                    "type": "Direct",
                    "requested": "[13.0.4, 13.0.4]",
                    "resolved": "13.0.4",
                    "contentHash": NEWTONSOFT_JSON_CONTENT_HASH
                }
            }}
        });
        let mut cases: Vec<(&str, Value)> = Vec::new();
        for (what, pointer, replacement) in [
            (
                "a floating request",
                "/dependencies/net10.0/Newtonsoft.Json/requested",
                serde_json::json!("[13.0.4, )"),
            ),
            (
                "another version",
                "/dependencies/net10.0/Newtonsoft.Json/resolved",
                serde_json::json!("13.0.3"),
            ),
            (
                "another archive",
                "/dependencies/net10.0/Newtonsoft.Json/contentHash",
                serde_json::json!("sha512-no"),
            ),
            (
                "a transitive disguise",
                "/dependencies/net10.0/Newtonsoft.Json/type",
                serde_json::json!("Transitive"),
            ),
        ] {
            let mut value = base.clone();
            *value.pointer_mut(pointer).expect("pointer") = replacement;
            cases.push((what, value));
        }
        let mut unknown = base.clone();
        unknown["dependencies"]["net10.0"]["Other.Package"] = serde_json::json!({
            "type": "Direct", "requested": "[1.0.0]", "resolved": "1.0.0",
            "contentHash": "sha512-no"
        });
        cases.push(("an unknown package", unknown));
        let mut target = base.clone();
        let dependencies = target["dependencies"].as_object_mut().expect("groups");
        let group = dependencies.remove("net10.0").expect("net10 group");
        dependencies.insert("net9.0".to_owned(), group);
        cases.push(("another target framework", target));

        for (what, value) in cases {
            let lockfile = lock(&serde_json::to_string(&value).expect("json"));
            assert!(
                approved(Language::CSharp, Some(&lockfile)).is_err(),
                "accepted {what}"
            );
        }
    }
}
