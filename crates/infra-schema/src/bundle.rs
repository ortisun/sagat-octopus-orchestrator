//! The artifact bundle, as a runner reports it and as a runner loads it: a canonical JSON document of
//! the code, the policy it was built under and the schemas of its input and result.
//!
//! ```json
//! { "v": 1, "language": "typescript", "policy": "v1;memory=…", "inputSchema": null,
//!   "outputSchema": null, "entry": "main.mjs", "files": { "main.mjs": "…" } }
//! ```

use base64::{Engine as _, engine::general_purpose::STANDARD};
use sagat_application::scripts::{BundleChecker, Expected};
use sagat_domain::scripts::Language;
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;

/// Files a bundle may hold.
pub const MAX_FILES: usize = 32;
/// Largest canonical artifact accepted by the control plane.
pub const MAX_BUNDLE_BYTES: usize = 2 * 1024 * 1024;
/// A v2 bundle is bounded before and after base64 decoding. The control plane separately applies
/// its 2 MiB artifact limit; this bound leaves room for JSON framing and the base64 expansion.
pub const MAX_DECODED_BYTES: usize = 1400 * 1024;

/// One file after the bundle's wire encoding has been checked and removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !name.starts_with('.')
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn rva_offset(bytes: &[u8], sections: usize, count: usize, rva: u32, size: u32) -> Option<usize> {
    for index in 0..count {
        let section = sections.checked_add(index.checked_mul(40)?)?;
        let virtual_address = u32_at(bytes, section + 12)?;
        let raw_size = u32_at(bytes, section + 16)?;
        let raw = u32_at(bytes, section + 20)?;
        let Some(relative) = rva.checked_sub(virtual_address) else {
            continue;
        };
        if relative <= raw_size && size <= raw_size.checked_sub(relative)? {
            let offset = raw.checked_add(relative)?;
            let end = offset.checked_add(size)?;
            let offset = usize::try_from(offset).ok()?;
            usize::try_from(end)
                .ok()
                .filter(|end| *end <= bytes.len())?;
            return Some(offset);
        }
    }
    None
}

/// Checks the PE/CLI headers without loading the assembly. The CLR header and metadata must map to
/// real section bytes, the metadata must have its signature, and `ILONLY` must exclude mixed-mode
/// assemblies. The runtime host remains the final loader and rejects invalid metadata.
fn managed_pe(bytes: &[u8]) -> bool {
    if bytes.get(..2) != Some(b"MZ") {
        return false;
    }
    let Some(pe) = u32_at(bytes, 0x3c).and_then(|v| usize::try_from(v).ok()) else {
        return false;
    };
    if bytes.get(pe..pe + 4) != Some(b"PE\0\0") {
        return false;
    }
    let section_count = u16_at(bytes, pe + 6).map(usize::from).unwrap_or(0);
    let optional_size = u16_at(bytes, pe + 20).map(usize::from).unwrap_or(0);
    let characteristics = u16_at(bytes, pe + 22).unwrap_or_default();
    let optional = pe + 24;
    let (directories, count_at, minimum) = match u16_at(bytes, optional) {
        Some(0x10b) => (optional + 96, optional + 92, 224),
        Some(0x20b) => (optional + 112, optional + 108, 240),
        _ => return false,
    };
    if section_count == 0
        || characteristics & 0x2000 == 0
        || optional_size < minimum
        || u32_at(bytes, count_at).is_none_or(|count| count <= 14)
    {
        return false;
    }
    let cli = directories + 14 * 8;
    let Some(cli_rva) = u32_at(bytes, cli).filter(|rva| *rva != 0) else {
        return false;
    };
    let Some(cli_size) = u32_at(bytes, cli + 4).filter(|size| *size >= 72) else {
        return false;
    };
    let sections = optional + optional_size;
    let Some(cli_at) = rva_offset(bytes, sections, section_count, cli_rva, cli_size) else {
        return false;
    };
    if u32_at(bytes, cli_at).is_none_or(|size| size < 72) {
        return false;
    }
    let Some(metadata_rva) = u32_at(bytes, cli_at + 8).filter(|rva| *rva != 0) else {
        return false;
    };
    let Some(metadata_size) = u32_at(bytes, cli_at + 12).filter(|size| *size >= 4) else {
        return false;
    };
    let flags = u32_at(bytes, cli_at + 16).unwrap_or_default();
    let Some(metadata) = rva_offset(bytes, sections, section_count, metadata_rva, metadata_size)
    else {
        return false;
    };
    flags & 1 == 1 && flags & 0x10 == 0 && bytes.get(metadata..metadata + 4) == Some(b"BSJB")
}

struct UniqueJson;

impl<'de> DeserializeSeed<'de> for UniqueJson {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueVisitor)
    }
}

struct UniqueVisitor;

impl<'de> Visitor<'de> for UniqueVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without duplicate object members")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(A::Error::custom("duplicate object member"));
            }
            map.next_value_seed(UniqueJson)?;
        }
        Ok(())
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element_seed(UniqueJson)?.is_some() {}
        Ok(())
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }
}

fn unique_json(bytes: &[u8]) -> bool {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    UniqueJson.deserialize(&mut deserializer).is_ok() && deserializer.end().is_ok()
}

/// The files of a bundle, if it is one for `language`: version 1, at most [`MAX_FILES`] files, each
/// with a plain name (letters, digits, `.`, `_`, `-`; up to 64; not starting with a dot).
#[must_use]
pub fn files(bytes: &[u8], language: Language) -> Option<Vec<(String, String)>> {
    decoded_files(bytes, language)?
        .into_iter()
        .map(|file| {
            String::from_utf8(file.bytes)
                .ok()
                .map(|text| (file.name, text))
        })
        .collect()
}

/// The checked, decoded files of either a v1 text bundle or a v2 C# binary bundle.
#[must_use]
pub fn decoded_files(bytes: &[u8], language: Language) -> Option<Vec<BundleFile>> {
    if bytes.len() > MAX_BUNDLE_BYTES {
        return None;
    }
    if !unique_json(bytes) {
        return None;
    }
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let version = value.get("v")?.as_u64()?;
    if value.get("language")?.as_str()? != language.as_str() {
        return None;
    }
    let files = value.get("files")?.as_object()?;
    if files.is_empty() || files.len() > MAX_FILES {
        return None;
    }
    let decoded: Vec<BundleFile> = match version {
        1 if matches!(language, Language::TypeScript | Language::Python) => files
            .iter()
            .map(|(name, content)| {
                (plain_name(name) && content.is_string()).then(|| BundleFile {
                    name: name.clone(),
                    bytes: content.as_str().unwrap_or_default().as_bytes().to_vec(),
                })
            })
            .collect(),
        2 if language == Language::CSharp => {
            const KEYS: [&str; 8] = [
                "v",
                "language",
                "policy",
                "inputSchema",
                "outputSchema",
                "entry",
                "encoding",
                "files",
            ];
            let object = value.as_object()?;
            if object.len() != KEYS.len()
                || KEYS.iter().any(|key| !object.contains_key(*key))
                || value.get("encoding")?.as_str()? != "base64"
                || value.get("entry")?.as_str()? != "script.dll"
            {
                return None;
            }
            let mut seen = HashSet::new();
            let mut total = 0usize;
            files
                .iter()
                .map(|(name, content)| {
                    let lower = name.to_ascii_lowercase();
                    if !plain_name(name) || !lower.ends_with(".dll") || !seen.insert(lower) {
                        return None;
                    }
                    let encoded = content.as_str()?;
                    let decoded = STANDARD.decode(encoded).ok()?;
                    if STANDARD.encode(&decoded) != encoded {
                        return None;
                    }
                    total = total.checked_add(decoded.len())?;
                    if decoded.is_empty() || total > MAX_DECODED_BYTES || !managed_pe(&decoded) {
                        return None;
                    }
                    Some(BundleFile {
                        name: name.clone(),
                        bytes: decoded,
                    })
                })
                .collect()
        }
        _ => None,
    }?;
    let entry = value.get("entry")?.as_str()?;
    decoded
        .iter()
        .any(|file| file.name == entry)
        .then_some(decoded)
}

/// Checks that a bundle says what the version it was built from says.
#[derive(Debug, Clone, Copy, Default)]
pub struct Bundles;

impl BundleChecker for Bundles {
    fn check(&self, bundle: &[u8], expected: &Expected<'_>) -> Result<(), &'static str> {
        let value: Value =
            serde_json::from_slice(bundle).map_err(|_| "the artifact is not a bundle")?;
        let listed = decoded_files(bundle, expected.language)
            .ok_or("the artifact is not a bundle of this language")?;
        let entry = value
            .get("entry")
            .and_then(Value::as_str)
            .ok_or("the artifact names no entry point")?;
        if !listed.iter().any(|file| file.name == entry) {
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

    fn fake_managed_pe_with(marker: u8, pe32_plus: bool) -> Vec<u8> {
        let mut bytes = vec![0u8; 1536];
        bytes[0..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[70..72].copy_from_slice(&1u16.to_le_bytes());
        bytes[84..86].copy_from_slice(&(if pe32_plus { 240u16 } else { 224u16 }).to_le_bytes());
        bytes[86..88].copy_from_slice(&0x2000u16.to_le_bytes());
        let optional = 64 + 24;
        bytes[optional..optional + 2]
            .copy_from_slice(&(if pe32_plus { 0x20bu16 } else { 0x10bu16 }).to_le_bytes());
        let (directories, count_at, optional_size) = if pe32_plus {
            (optional + 112, optional + 108, 240)
        } else {
            (optional + 96, optional + 92, 224)
        };
        bytes[count_at..count_at + 4].copy_from_slice(&15u32.to_le_bytes());
        let cli = directories + 14 * 8;
        bytes[cli..cli + 4].copy_from_slice(&0x2000u32.to_le_bytes());
        bytes[cli + 4..cli + 8].copy_from_slice(&72u32.to_le_bytes());
        let section = optional + optional_size;
        bytes[section..section + 5].copy_from_slice(b".text");
        bytes[section + 8..section + 12].copy_from_slice(&0x400u32.to_le_bytes());
        bytes[section + 12..section + 16].copy_from_slice(&0x2000u32.to_le_bytes());
        bytes[section + 16..section + 20].copy_from_slice(&0x400u32.to_le_bytes());
        bytes[section + 20..section + 24].copy_from_slice(&0x200u32.to_le_bytes());
        let cli_header = 0x200;
        bytes[cli_header..cli_header + 4].copy_from_slice(&72u32.to_le_bytes());
        bytes[cli_header + 8..cli_header + 12].copy_from_slice(&0x2100u32.to_le_bytes());
        bytes[cli_header + 12..cli_header + 16].copy_from_slice(&64u32.to_le_bytes());
        bytes[cli_header + 16..cli_header + 20].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x300..0x304].copy_from_slice(b"BSJB");
        bytes[1535] = marker;
        bytes
    }

    fn fake_managed_pe(marker: u8) -> Vec<u8> {
        fake_managed_pe_with(marker, false)
    }

    fn csharp_bundle(policy: &Policy) -> Value {
        json!({
            "v": 2,
            "language": "csharp",
            "policy": policy.canonical(),
            "inputSchema": null,
            "outputSchema": null,
            "entry": "script.dll",
            "encoding": "base64",
            "files": {"script.dll": STANDARD.encode(fake_managed_pe(1))}
        })
    }

    fn csharp_expected(policy: &Policy) -> Expected<'_> {
        Expected {
            language: Language::CSharp,
            policy,
            input_schema: None,
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

    #[test]
    fn a_csharp_v2_bundle_carries_base64_managed_assemblies() {
        let policy = Policy::default();
        let script = fake_managed_pe(1);
        let dependency = fake_managed_pe(2);
        let bytes = serde_json::to_vec(&json!({
            "v": 2,
            "language": "csharp",
            "policy": policy.canonical(),
            "inputSchema": null,
            "outputSchema": null,
            "entry": "script.dll",
            "encoding": "base64",
            "files": {
                "script.dll": STANDARD.encode(&script),
                "Newtonsoft.Json.dll": STANDARD.encode(&dependency)
            }
        }))
        .expect("json");
        assert_eq!(Bundles.check(&bytes, &csharp_expected(&policy)), Ok(()));
        let decoded = decoded_files(&bytes, Language::CSharp).expect("decoded files");
        assert_eq!(decoded[0].bytes, dependency);
        assert_eq!(decoded[1].bytes, script);

        let mut pe32_plus = csharp_bundle(&policy);
        pe32_plus["files"] = json!({"script.dll": STANDARD.encode(fake_managed_pe_with(3, true))});
        let pe32_plus = serde_json::to_vec(&pe32_plus).expect("json");
        assert_eq!(Bundles.check(&pe32_plus, &csharp_expected(&policy)), Ok(()));
    }

    #[test]
    fn v1_text_bundles_remain_valid_for_typescript_and_python_only() {
        let policy = Policy::default();
        for (language, entry) in [
            (Language::TypeScript, "main.mjs"),
            (Language::Python, "main.py"),
        ] {
            let bytes = serde_json::to_vec(&json!({
                "v": 1, "language": language.as_str(), "policy": policy.canonical(),
                "inputSchema": null, "outputSchema": null, "entry": entry,
                "files": {entry: "source"}
            }))
            .expect("json");
            let expected = Expected {
                language,
                policy: &policy,
                input_schema: None,
                output_schema: None,
            };
            assert_eq!(Bundles.check(&bytes, &expected), Ok(()), "{language}");
            assert_eq!(files(&bytes, language).expect("text")[0].1, "source");
        }
        let csharp_v1 = bundle(&policy, json!({"language": "csharp"}));
        assert!(
            Bundles
                .check(&csharp_v1, &csharp_expected(&policy))
                .is_err()
        );
    }

    #[test]
    fn malformed_or_ambiguous_csharp_binaries_are_refused_before_signing() {
        let policy = Policy::default();
        let valid = csharp_bundle(&policy);
        let native = STANDARD.encode(vec![0u8; 512]);
        let mut cases = Vec::new();
        for (what, key, replacement) in [
            ("unknown encoding", "encoding", json!("hex")),
            ("missing entry", "entry", json!("missing.dll")),
            ("non-string content", "files", json!({"script.dll": 4})),
            ("invalid base64", "files", json!({"script.dll": "***"})),
            (
                "non-canonical base64",
                "files",
                json!({"script.dll": "AB=="}),
            ),
            ("empty file", "files", json!({"script.dll": ""})),
            ("native file", "files", json!({"script.dll": native})),
            (
                "non-DLL",
                "files",
                json!({"script.exe": STANDARD.encode(fake_managed_pe(1))}),
            ),
            (
                "a path",
                "files",
                json!({"../script.dll": STANDARD.encode(fake_managed_pe(1))}),
            ),
            (
                "a dotfile",
                "files",
                json!({".script.dll": STANDARD.encode(fake_managed_pe(1))}),
            ),
            (
                "case-insensitive duplicate assembly names",
                "files",
                json!({
                    "script.dll": STANDARD.encode(fake_managed_pe(1)),
                    "SCRIPT.DLL": STANDARD.encode(fake_managed_pe(2))
                }),
            ),
        ] {
            let mut value = valid.clone();
            value[key] = replacement;
            cases.push((what, value));
        }
        let mut unknown = valid.clone();
        unknown["surprise"] = json!(true);
        cases.push(("an unknown top-level field", unknown));
        let mut too_many = valid.clone();
        too_many["files"] = Value::Object(
            std::iter::once((
                "script.dll".to_owned(),
                Value::String(STANDARD.encode(fake_managed_pe(0))),
            ))
            .chain((0..MAX_FILES).map(|index| {
                (
                    format!("assembly{index}.dll"),
                    Value::String(STANDARD.encode(fake_managed_pe(index as u8))),
                )
            }))
            .collect(),
        );
        too_many["entry"] = json!("script.dll");
        cases.push(("too many files", too_many));
        let mut oversized = fake_managed_pe(1);
        oversized.resize(MAX_DECODED_BYTES + 1, 0);
        let mut too_large = valid.clone();
        too_large["files"] = json!({"script.dll": STANDARD.encode(oversized)});
        cases.push(("too many decoded bytes", too_large));

        let section = 64 + 24 + 224;
        for (what, invalid) in [
            ("an image without the DLL characteristic", {
                let mut bytes = fake_managed_pe(1);
                bytes[86..88].copy_from_slice(&0u16.to_le_bytes());
                bytes
            }),
            ("a CLR header in a section without raw bytes", {
                let mut bytes = fake_managed_pe(1);
                bytes[section + 16..section + 20].copy_from_slice(&0u32.to_le_bytes());
                bytes
            }),
            ("metadata beyond the section's raw bytes", {
                let mut bytes = fake_managed_pe(1);
                bytes[0x208..0x20c].copy_from_slice(&0x2400u32.to_le_bytes());
                bytes
            }),
            ("a truncated CLR header declaration", {
                let mut bytes = fake_managed_pe(1);
                bytes[0x200..0x204].copy_from_slice(&0u32.to_le_bytes());
                bytes
            }),
            ("a native-entry-point CLR image", {
                let mut bytes = fake_managed_pe(1);
                bytes[0x210..0x214].copy_from_slice(&0x11u32.to_le_bytes());
                bytes
            }),
        ] {
            let mut value = valid.clone();
            value["files"] = json!({"script.dll": STANDARD.encode(invalid)});
            cases.push((what, value));
        }

        for (what, value) in cases {
            let bytes = serde_json::to_vec(&value).expect("json");
            assert!(
                Bundles.check(&bytes, &csharp_expected(&policy)).is_err(),
                "accepted {what}"
            );
        }

        let duplicate = format!(
            "{{\"v\":2,\"v\":2,\"language\":\"csharp\",\"policy\":{},\"inputSchema\":null,\"outputSchema\":null,\"entry\":\"script.dll\",\"encoding\":\"base64\",\"files\":{{\"script.dll\":\"{}\"}}}}",
            serde_json::to_string(&policy.canonical()).expect("policy"),
            STANDARD.encode(fake_managed_pe(1))
        );
        assert!(
            Bundles
                .check(duplicate.as_bytes(), &csharp_expected(&policy))
                .is_err(),
            "accepted a duplicate JSON member"
        );
    }
}
