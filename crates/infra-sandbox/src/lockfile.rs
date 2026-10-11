//! What a lockfile pins. The platform accepts a lockfile (it is part of the identity of a build)
//! but does not yet install anything from a registry: a lockfile that pins a package is refused,
//! with the reason, rather than ignored. That is a pending decision (see the gaps of feature 005),
//! not a promise.

use sagat_domain::scripts::{Language, Lockfile};
use serde_json::Value;

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
}
