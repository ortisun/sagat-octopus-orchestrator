//! Feature 035 contract invariants: the canonical workflow schema, receipts, OpenAPI, protobuf and error
//! catalog (T035-007). They are written before the fragments are merged into the canonical contracts
//! (T035-008), so the cases that need the merge fail first; the cases that read the fragments themselves
//! (receipts, the fragment checks of the IR cases) pass from the start.
#![allow(clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

mod attempt_input_builder;
mod errors;
mod ir;
mod openapi;
mod proto;
mod receipts;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("contract/iceberg-writes/fixtures")
}

/// The contract fragments of the feature (`specs/035-iceberg-write-steps/contracts`), which stay live after the merge.
fn fragments() -> PathBuf {
    repository_root().join("specs/035-iceberg-write-steps/contracts")
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&read_text(path))
        .unwrap_or_else(|error| panic!("{} is JSON: {error}", path.display()))
}

/// Every `*.json` of a fixture directory, by file name without the extension, sorted.
fn json_files(directory: &Path) -> Vec<(String, Value)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .expect("a file name")
                .to_string_lossy()
                .into_owned();
            (name, read_json(&path))
        })
        .collect()
}

/// One change to a JSON document, by JSON pointer. A refusal case is a sample that is accepted plus one edit, so
/// the edit alone explains the refusal.
enum Edit {
    Set(&'static str, Value),
    Remove(&'static str),
}

fn apply(document: &mut Value, edit: &Edit) {
    let (pointer, replacement) = match edit {
        Edit::Set(pointer, value) => (*pointer, Some(value)),
        Edit::Remove(pointer) => (*pointer, None),
    };
    let (parent, key) = pointer.rsplit_once('/').expect("a pointer with a parent");
    let target = document
        .pointer_mut(parent)
        .unwrap_or_else(|| panic!("{parent} is not in the sample"));
    match (target, replacement) {
        (Value::Object(map), Some(value)) => {
            map.insert(key.to_string(), value.clone());
        }
        (Value::Object(map), None) => {
            map.remove(key)
                .unwrap_or_else(|| panic!("{pointer} is not in the sample"));
        }
        (Value::Array(items), Some(value)) => {
            let index: usize = key.parse().expect("an array index");
            items[index] = value.clone();
        }
        (Value::Array(items), None) => {
            items.remove(key.parse::<usize>().expect("an array index"));
        }
        (other, _) => panic!("{parent} is {other}, not a container"),
    }
}

/// The first reasons a validator gives for refusing `instance`, or `None` if it accepts it.
fn refusal(validator: &jsonschema::Validator, instance: &Value) -> Option<String> {
    let reasons: Vec<String> = validator
        .iter_errors(instance)
        .take(3)
        .map(|error| {
            // The message starts with the whole instance and ends with the reason: keep the path and the end.
            let message = error.to_string();
            let characters: Vec<char> = message.chars().collect();
            let reason: String = if characters.len() > 110 {
                format!(
                    "…{}",
                    characters[characters.len() - 110..]
                        .iter()
                        .collect::<String>()
                )
            } else {
                message
            };
            format!("{}: {reason}", error.instance_path())
        })
        .collect();
    (!reasons.is_empty()).then(|| reasons.join("; "))
}
