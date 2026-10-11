//! The manifest: a file kept in Git that says which workflows a namespace has and where their
//! documents are (FR-011-002).
//!
//! The manifest is declarative and small: the namespace, and for each workflow its name, the file
//! with its document and, optionally, the release notes. It is read from JSON or YAML; the
//! documents are JSON or YAML as well and are turned into JSON before anything is sent. Nothing in
//! a manifest is trusted: a file is read only from below the directory of the manifest, a name must
//! be what the platform accepts, and a document larger than the platform takes is refused here and
//! not after an upload.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::cell::Cell;

/// The `apiVersion` of the manifest.
pub const API_VERSION: &str = "sagat-octopus-orchestrator.io/v1";
/// The `kind` of the manifest.
pub const KIND: &str = "Manifest";
/// Most workflows one manifest lists.
pub const MAX_WORKFLOWS: usize = 500;
/// The largest manifest or document read: what the platform takes of a workflow document.
pub const MAX_BYTES: u64 = 1024 * 1024;
/// Longest release note the platform keeps.
pub const MAX_RELEASE_NOTES: usize = 2048;

/// One workflow of the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    pub name: String,
    pub file: String,
    #[serde(default)]
    pub release_notes: Option<String>,
}

/// A manifest as written.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub api_version: String,
    pub kind: String,
    pub namespace: String,
    pub workflows: Vec<Entry>,
}

/// What is wrong with a manifest, each with where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub at: String,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.at, self.message)
    }
}

fn problem(at: impl Into<String>, message: impl Into<String>) -> Problem {
    Problem {
        at: at.into(),
        message: message.into(),
    }
}

/// A slug as the platform takes it for namespaces and workflows: lower case letters, digits and
/// hyphens, starting with a letter, 3 to 63 characters, no two hyphens in a row, not ending in one.
#[must_use]
pub fn is_slug(text: &str) -> bool {
    let bytes = text.as_bytes();
    (3..=63).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[bytes.len() - 1] != b'-'
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && !text.contains("--")
}

/// Whether `file` stays below the directory of the manifest: relative, no `..`, no root.
#[must_use]
pub fn stays_below(file: &str) -> bool {
    let path = Path::new(file);
    !file.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// What a document may expand to once YAML aliases are followed, counted as 16 bytes a node and the
/// bytes of every string. A document of [`MAX_BYTES`] without aliases cannot come near it; a "billion
/// laughs" document is stopped long before it fills the memory of a pipeline.
const MAX_EXPANDED: usize = 32 * 1024 * 1024;

/// Reads YAML into a JSON value, refusing one that expands beyond [`MAX_EXPANDED`].
fn yaml_value(text: &str) -> Result<Value, String> {
    let spent = Cell::new(0usize);
    let deserializer = serde_yaml_ng::Deserializer::from_str(text);
    let mut documents = deserializer.into_iter();
    let value = match documents.next() {
        Some(document) => Bounded(&spent)
            .deserialize(document)
            .map_err(|e| e.to_string())?,
        None => Value::Null,
    };
    if documents.next().is_some() {
        return Err("more than one YAML document in the file".to_owned());
    }
    Ok(value)
}

struct Bounded<'a>(&'a Cell<usize>);

impl<'de> DeserializeSeed<'de> for Bounded<'_> {
    type Value = Value;

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl Bounded<'_> {
    fn spend<E: de::Error>(&self, bytes: usize) -> Result<(), E> {
        let used = self.0.get().saturating_add(16).saturating_add(bytes);
        if used > MAX_EXPANDED {
            return Err(E::custom(
                "the document expands to more than the platform could take (YAML aliases that repeat themselves?)",
            ));
        }
        self.0.set(used);
        Ok(())
    }
}

impl<'de> Visitor<'de> for Bounded<'_> {
    type Value = Value;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("any value")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        self.spend(0)?;
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        self.spend(0)?;
        Ok(Value::from(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        self.spend(0)?;
        Ok(Value::from(v))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        self.spend(0)?;
        Ok(Number::from_f64(v).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        self.spend(v.len())?;
        Ok(Value::String(v.to_owned()))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        self.spend(0)?;
        Ok(Value::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        self.visit_unit()
    }

    fn visit_some<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        self.spend(0)?;
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(Bounded(self.0))? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        self.spend(0)?;
        let mut entries = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            self.spend(key.len())?;
            entries.insert(key, map.next_value_seed(Bounded(self.0))?);
        }
        Ok(Value::Object(entries))
    }
}

impl Manifest {
    /// Reads a manifest from `text`, as JSON or as YAML.
    ///
    /// # Errors
    /// A [`Problem`] naming what could not be read.
    pub fn parse(text: &str, yaml: bool) -> Result<Self, Problem> {
        let value = if yaml {
            yaml_value(text).map_err(|e| problem("manifest", e))?
        } else {
            serde_json::from_str::<Value>(text).map_err(|e| problem("manifest", e.to_string()))?
        };
        serde_json::from_value(value).map_err(|e| problem("manifest", e.to_string()))
    }

    /// Everything wrong with the manifest as written, found in one pass.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        let mut found = Vec::new();
        if self.api_version != API_VERSION {
            found.push(problem(
                "apiVersion",
                format!("this tool reads {API_VERSION}"),
            ));
        }
        if self.kind != KIND {
            found.push(problem("kind", format!("must be {KIND}")));
        }
        if !is_slug(&self.namespace) {
            found.push(problem(
                "namespace",
                "a namespace slug: lower case letters, digits and hyphens, 3 to 63 characters",
            ));
        }
        if self.workflows.len() > MAX_WORKFLOWS {
            found.push(problem(
                "workflows",
                format!("at most {MAX_WORKFLOWS} workflows in one manifest"),
            ));
        }
        let mut names = BTreeSet::new();
        let mut files = BTreeSet::new();
        for (i, entry) in self.workflows.iter().enumerate() {
            let at = format!("workflows[{i}]");
            if !is_slug(&entry.name) {
                found.push(problem(
                    format!("{at}.name"),
                    "a workflow name: lower case letters, digits and hyphens, 3 to 63 characters",
                ));
            } else if !names.insert(entry.name.clone()) {
                found.push(problem(format!("{at}.name"), "this name is listed twice"));
            }
            if !stays_below(&entry.file) {
                found.push(problem(
                    format!("{at}.file"),
                    "a relative path below the directory of the manifest",
                ));
            } else if !files.insert(entry.file.clone()) {
                found.push(problem(
                    format!("{at}.file"),
                    "two workflows cannot be the same file",
                ));
            }
            if entry
                .release_notes
                .as_ref()
                .is_some_and(|n| n.len() > MAX_RELEASE_NOTES)
            {
                found.push(problem(
                    format!("{at}.releaseNotes"),
                    format!("at most {MAX_RELEASE_NOTES} bytes"),
                ));
            }
        }
        found
    }
}

/// Whether a path names a YAML file.
#[must_use]
pub fn is_yaml(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("yaml" | "yml")
    )
}

/// Reads a file of at most [`MAX_BYTES`].
///
/// # Errors
/// A [`Problem`] when the file cannot be read or is larger than the platform takes.
pub fn read_limited(path: &Path, at: &str) -> Result<String, Problem> {
    let meta =
        std::fs::metadata(path).map_err(|e| problem(at, format!("{}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(problem(at, format!("{} is not a file", path.display())));
    }
    if meta.len() > MAX_BYTES {
        return Err(problem(
            at,
            format!("{} is larger than {MAX_BYTES} bytes", path.display()),
        ));
    }
    std::fs::read_to_string(path).map_err(|e| problem(at, format!("{}: {e}", path.display())))
}

/// The file as it really is on disk, after following links, if it is still below the directory of
/// the manifest: a link in a repository must not make the tool read (and send) another file.
fn confined(base: &Path, file: &str) -> Result<PathBuf, String> {
    let joined = base.join(file);
    let real = std::fs::canonicalize(&joined).map_err(|e| format!("{}: {e}", joined.display()))?;
    let root = std::fs::canonicalize(base).map_err(|e| format!("{}: {e}", base.display()))?;
    if real.starts_with(&root) {
        Ok(real)
    } else {
        Err(format!(
            "{} leads outside the directory of the manifest",
            joined.display()
        ))
    }
}

/// A workflow of the manifest with its document read.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub entry: Entry,
    /// The document as JSON text.
    pub document: Vec<u8>,
    pub value: Value,
}

/// The manifest read from `path`, validated, with the document of every workflow loaded.
///
/// # Errors
/// Every problem found, so a person fixes them in one go.
pub fn load(path: &Path) -> Result<(Manifest, Vec<Loaded>), Vec<Problem>> {
    let text = read_limited(path, "manifest").map_err(|p| vec![p])?;
    let manifest = Manifest::parse(&text, is_yaml(path)).map_err(|p| vec![p])?;
    let mut found = manifest.problems();
    if !found.is_empty() {
        return Err(found);
    }
    let base: PathBuf = path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let mut loaded = Vec::new();
    for (i, entry) in manifest.workflows.iter().enumerate() {
        let at = format!("workflows[{i}].file");
        let file = match confined(&base, &entry.file) {
            Ok(file) => file,
            Err(why) => {
                found.push(problem(&at, why));
                continue;
            }
        };
        let text = match read_limited(&file, &at) {
            Ok(text) => text,
            Err(p) => {
                found.push(p);
                continue;
            }
        };
        let value: Result<Value, String> = if is_yaml(&file) {
            yaml_value(&text)
        } else {
            serde_json::from_str(&text).map_err(|e| e.to_string())
        };
        match value {
            Ok(value) if value.is_object() => {
                if value["metadata"]["name"].as_str() != Some(entry.name.as_str()) {
                    found.push(problem(
                        &at,
                        format!(
                            "the document is named '{}' and the manifest lists it as '{}'",
                            value["metadata"]["name"].as_str().unwrap_or(""),
                            entry.name
                        ),
                    ));
                    continue;
                }
                let document = serde_json::to_vec(&value).map_err(|e| problem(&at, e.to_string()));
                match document {
                    Ok(document) => loaded.push(Loaded {
                        entry: entry.clone(),
                        document,
                        value,
                    }),
                    Err(p) => found.push(p),
                }
            }
            Ok(_) => found.push(problem(&at, "a workflow document is an object")),
            Err(e) => found.push(problem(&at, e)),
        }
    }
    if found.is_empty() {
        Ok((manifest, loaded))
    } else {
        Err(found)
    }
}
