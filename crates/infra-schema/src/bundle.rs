//! The artifact bundle, as a runner reports it and as a runner loads it: a canonical JSON document of
//! the code, the policy it was built under and the schemas of its input and result.
//!
//! ```json
//! { "v": 1, "language": "typescript", "policy": "v1;memory=…", "inputSchema": null,
//!   "outputSchema": null, "entry": "main.mjs", "files": { "main.mjs": "…" } }
//! ```

use base64::{Engine as _, engine::general_purpose::STANDARD};
use flate2::bufread::GzDecoder;
use sagat_application::scripts::{BundleChecker, Expected};
use sagat_domain::scripts::Language;
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;
use std::io::{Cursor, Read as _};

/// Files a bundle may hold.
pub const MAX_FILES: usize = 32;
/// Largest canonical artifact accepted by the control plane.
pub const MAX_BUNDLE_BYTES: usize = 2 * 1024 * 1024;
/// A v2 bundle is bounded before and after base64 decoding. The control plane separately applies
/// its 2 MiB artifact limit; this bound leaves room for JSON framing and the base64 expansion.
pub const MAX_DECODED_BYTES: usize = 1400 * 1024;
/// Largest executable produced by expanding a compressed Go artifact.
pub const MAX_EXPANDED_BYTES: usize = 8 * 1024 * 1024;

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

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn be_u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn checked_table(bytes: &[u8], offset: u64, count: u16, width: u16) -> Option<usize> {
    let offset = usize::try_from(offset).ok()?;
    let size = usize::from(count).checked_mul(usize::from(width))?;
    offset.checked_add(size).filter(|end| *end <= bytes.len())?;
    Some(offset)
}

/// Bounded validation of the native image before it is signed. The kernel remains the final ELF
/// loader; this excludes interpreters, another architecture, malformed header tables, executable
/// stacks and writable executable segments without trying to duplicate that loader.
fn static_elf(bytes: &[u8], target: &str) -> bool {
    let machine = match target {
        "linux-amd64" => 62,
        "linux-arm64" => 183,
        _ => return false,
    };
    if bytes.len() < 64
        || bytes.len() > MAX_EXPANDED_BYTES
        || bytes.get(..16) != Some(b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0")
        || u16_at(bytes, 16) != Some(2)
        || u16_at(bytes, 18) != Some(machine)
        || u32_at(bytes, 20) != Some(1)
        || u16_at(bytes, 52) != Some(64)
        || u16_at(bytes, 54) != Some(56)
    {
        return false;
    }
    let Some(phnum) = u16_at(bytes, 56).filter(|count| (1..=128).contains(count)) else {
        return false;
    };
    let Some(programs) =
        u64_at(bytes, 32).and_then(|offset| checked_table(bytes, offset, phnum, 56))
    else {
        return false;
    };
    let section_count = u16_at(bytes, 60).unwrap_or_default();
    if section_count != 0
        && (u16_at(bytes, 58) != Some(64)
            || u64_at(bytes, 40)
                .and_then(|offset| checked_table(bytes, offset, section_count, 64))
                .is_none())
    {
        return false;
    }
    let entry = u64_at(bytes, 24).unwrap_or_default();
    let mut executable_entry = false;
    let mut safe_stack = false;
    for index in 0..usize::from(phnum) {
        let at = programs + index * 56;
        let kind = u32_at(bytes, at).unwrap_or_default();
        let flags = u32_at(bytes, at + 4).unwrap_or_default();
        if kind == 3 {
            return false;
        }
        if kind == 0x6474_e551 {
            safe_stack = flags & 1 == 0;
        }
        if kind != 1 {
            continue;
        }
        let Some(offset) = u64_at(bytes, at + 8) else {
            return false;
        };
        let Some(vaddr) = u64_at(bytes, at + 16) else {
            return false;
        };
        let Some(file_size) = u64_at(bytes, at + 32) else {
            return false;
        };
        let Some(memory_size) = u64_at(bytes, at + 40) else {
            return false;
        };
        if memory_size < file_size
            || offset
                .checked_add(file_size)
                .and_then(|end| usize::try_from(end).ok())
                .is_none_or(|end| end > bytes.len())
            || flags & 3 == 3
        {
            return false;
        }
        executable_entry |=
            flags & 1 == 1 && entry >= vaddr && entry < vaddr.saturating_add(memory_size);
    }
    executable_entry && safe_stack
}

fn canonical_gzip(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 18
        || bytes.len() > MAX_DECODED_BYTES
        || bytes.get(..10) != Some(b"\x1f\x8b\x08\0\0\0\0\0\x02\x03")
    {
        return None;
    }
    let mut decoder = GzDecoder::new(Cursor::new(bytes));
    let mut expanded = Vec::new();
    decoder
        .by_ref()
        .take((MAX_EXPANDED_BYTES + 1) as u64)
        .read_to_end(&mut expanded)
        .ok()?;
    let consumed = usize::try_from(decoder.into_inner().position()).ok()?;
    (expanded.len() <= MAX_EXPANDED_BYTES && consumed == bytes.len()).then_some(expanded)
}

fn java_class_name(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 14
        || bytes.get(..4) != Some(b"\xca\xfe\xba\xbe")
        || be_u16_at(bytes, 4) != Some(0)
        || be_u16_at(bytes, 6) != Some(61)
    {
        return None;
    }
    let count = usize::from(be_u16_at(bytes, 8)?);
    if count < 2 {
        return None;
    }
    let mut utf8 = vec![None; count];
    let mut classes = vec![None; count];
    let mut at = 10usize;
    let mut index = 1usize;
    while index < count {
        let tag = *bytes.get(at)?;
        at += 1;
        match tag {
            1 => {
                let len = usize::from(be_u16_at(bytes, at)?);
                at += 2;
                let end = at.checked_add(len)?;
                utf8[index] = Some(std::str::from_utf8(bytes.get(at..end)?).ok()?.to_owned());
                at = end;
            }
            3 | 4 => at = at.checked_add(4)?.min(bytes.len()),
            5 | 6 => {
                at = at.checked_add(8)?.min(bytes.len());
                index += 1;
                if index >= count {
                    return None;
                }
            }
            7 => {
                classes[index] = Some(usize::from(be_u16_at(bytes, at)?));
                at += 2;
            }
            8 | 16 | 19 | 20 => at = at.checked_add(2)?.min(bytes.len()),
            9 | 10 | 11 | 12 | 17 | 18 => at = at.checked_add(4)?.min(bytes.len()),
            15 => at = at.checked_add(3)?.min(bytes.len()),
            _ => return None,
        }
        if at > bytes.len() {
            return None;
        }
        index += 1;
    }
    let this_class = usize::from(be_u16_at(bytes, at + 2)?);
    let name = classes.get(this_class)?.as_ref().copied()?;
    utf8.get(name)?.clone()
}

fn java_class(bytes: &[u8], file_name: &str) -> bool {
    let Some(expected) = file_name.strip_suffix(".class") else {
        return false;
    };
    (expected == "Script" || expected.starts_with("Script$") && expected.len() > 7)
        && java_class_name(bytes).as_deref() == Some(expected)
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
        3 if matches!(language, Language::Go | Language::Java) => {
            const KEYS: [&str; 10] = [
                "v",
                "language",
                "target",
                "abi",
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
                || value.get("abi")?.as_str()? != "sagat-inline-v1"
            {
                return None;
            }
            let carried_target = value.get("target")?.as_str()?;
            let (entry, encoding) = match language {
                Language::Go if matches!(carried_target, "linux-amd64" | "linux-arm64") => {
                    ("script.elf", "base64+gzip")
                }
                Language::Java => ("Script.class", "base64"),
                _ => return None,
            };
            if language == Language::Java && carried_target != "jvm-17"
                || value.get("entry")?.as_str()? != entry
                || value.get("encoding")?.as_str()? != encoding
                || language == Language::Go && files.len() != 1
            {
                return None;
            }
            let mut seen = HashSet::new();
            let mut total = 0usize;
            files
                .iter()
                .map(|(name, content)| {
                    let lower = name.to_ascii_lowercase();
                    if !plain_name(name) || !seen.insert(lower) {
                        return None;
                    }
                    let encoded = content.as_str()?;
                    let decoded = STANDARD.decode(encoded).ok()?;
                    if STANDARD.encode(&decoded) != encoded {
                        return None;
                    }
                    total = total.checked_add(decoded.len())?;
                    if decoded.is_empty() || total > MAX_DECODED_BYTES {
                        return None;
                    }
                    let valid = match language {
                        Language::Go => {
                            name == "script.elf"
                                && canonical_gzip(&decoded)
                                    .is_some_and(|elf| static_elf(&elf, carried_target))
                        }
                        Language::Java => java_class(&decoded, name),
                        _ => false,
                    };
                    valid.then(|| BundleFile {
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

/// The signed execution target carried by a v3 bundle.
#[must_use]
pub fn target(bytes: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    (value.get("v")?.as_u64()? == 3)
        .then(|| value.get("target")?.as_str().map(str::to_owned))
        .flatten()
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
    use flate2::{Compression, GzBuilder};
    use sagat_domain::scripts::Policy;
    use serde_json::json;
    use std::io::Write as _;

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

    fn fake_amd64_elf() -> Vec<u8> {
        let mut bytes = vec![0u8; 256];
        bytes[..16].copy_from_slice(b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0");
        bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&62u16.to_le_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&0x400080u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
        bytes[52..54].copy_from_slice(&64u16.to_le_bytes());
        bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&2u16.to_le_bytes());
        // Executable, read-only PT_LOAD containing the entry point.
        bytes[64..68].copy_from_slice(&1u32.to_le_bytes());
        bytes[68..72].copy_from_slice(&5u32.to_le_bytes());
        bytes[72..80].copy_from_slice(&0u64.to_le_bytes());
        bytes[80..88].copy_from_slice(&0x400000u64.to_le_bytes());
        let file_len = bytes.len() as u64;
        bytes[96..104].copy_from_slice(&file_len.to_le_bytes());
        bytes[104..112].copy_from_slice(&file_len.to_le_bytes());
        bytes[112..120].copy_from_slice(&0x1000u64.to_le_bytes());
        // A present, non-executable GNU stack.
        bytes[120..124].copy_from_slice(&0x6474_e551u32.to_le_bytes());
        bytes[124..128].copy_from_slice(&6u32.to_le_bytes());
        bytes
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut writer = GzBuilder::new()
            .mtime(0)
            .operating_system(3)
            .write(Vec::new(), Compression::best());
        writer.write_all(bytes).expect("gzip bytes");
        writer.finish().expect("finish gzip")
    }

    fn go_bundle_for(policy: &Policy, target: &str, compressed: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "v": 3, "language": "go", "target": target, "abi": "sagat-inline-v1",
            "policy": policy.canonical(), "inputSchema": null, "outputSchema": null,
            "entry": "script.elf", "encoding": "base64+gzip",
            "files": {"script.elf": STANDARD.encode(compressed)}
        }))
        .expect("go bundle")
    }

    fn go_bundle(policy: &Policy, compressed: &[u8]) -> Vec<u8> {
        go_bundle_for(policy, "linux-amd64", compressed)
    }

    fn go_expected(policy: &Policy) -> Expected<'_> {
        Expected {
            language: Language::Go,
            policy,
            input_schema: None,
            output_schema: None,
        }
    }

    fn fake_class_named(name: &str) -> Vec<u8> {
        // Constant pool: Utf8 class name, Class #1; this_class = #2.
        let mut bytes = b"\xca\xfe\xba\xbe\0\0\0=\0\x03\x01".to_vec();
        bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(b"\x07\0\x01");
        bytes.extend_from_slice(&0x0021u16.to_be_bytes());
        bytes.extend_from_slice(&2u16.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes
    }

    fn fake_script_class() -> Vec<u8> {
        fake_class_named("Script")
    }

    fn java_bundle(policy: &Policy, class: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "v": 3, "language": "java", "target": "jvm-17", "abi": "sagat-inline-v1",
            "policy": policy.canonical(), "inputSchema": null, "outputSchema": null,
            "entry": "Script.class", "encoding": "base64",
            "files": {"Script.class": STANDARD.encode(class)}
        }))
        .expect("java bundle")
    }

    fn java_expected(policy: &Policy) -> Expected<'_> {
        Expected {
            language: Language::Java,
            policy,
            input_schema: None,
            output_schema: None,
        }
    }

    #[test]
    fn v3_go_and_java_bundles_are_additive_to_v1_and_v2() {
        let policy = Policy::default();
        let elf = fake_amd64_elf();
        let go = go_bundle(&policy, &gzip(&elf));
        assert_eq!(Bundles.check(&go, &go_expected(&policy)), Ok(()));
        assert_eq!(
            canonical_gzip(&decoded_files(&go, Language::Go).expect("go")[0].bytes),
            Some(elf)
        );
        let mut arm64 = fake_amd64_elf();
        arm64[18..20].copy_from_slice(&183u16.to_le_bytes());
        let arm64 = go_bundle_for(&policy, "linux-arm64", &gzip(&arm64));
        assert!(decoded_files(&arm64, Language::Go).is_some());
        assert_eq!(target(&arm64).as_deref(), Some("linux-arm64"));

        let class = fake_script_class();
        let java = java_bundle(&policy, &class);
        assert_eq!(Bundles.check(&java, &java_expected(&policy)), Ok(()));
        assert_eq!(
            decoded_files(&java, Language::Java).expect("java")[0].bytes,
            class
        );
    }

    #[test]
    fn malformed_ambiguous_or_unsafe_go_bundles_are_refused() {
        let policy = Policy::default();
        let compressed = gzip(&fake_amd64_elf());
        let mut cases = Vec::new();

        let mut trailing = compressed.clone();
        trailing.push(0);
        cases.push(("trailing gzip data", go_bundle(&policy, &trailing)));
        let mut second_member = compressed.clone();
        second_member.extend(gzip(&fake_amd64_elf()));
        cases.push(("a second gzip member", go_bundle(&policy, &second_member)));
        let mut bad_crc = compressed.clone();
        let last = bad_crc.len() - 8;
        bad_crc[last] ^= 1;
        cases.push(("a bad gzip CRC", go_bundle(&policy, &bad_crc)));
        cases.push((
            "a truncated gzip stream",
            go_bundle(&policy, &compressed[..compressed.len() - 1]),
        ));
        let mut noncanonical = compressed.clone();
        noncanonical[4] = 1;
        cases.push((
            "a noncanonical gzip header",
            go_bundle(&policy, &noncanonical),
        ));
        cases.push((
            "a gzip expansion bomb",
            go_bundle(&policy, &gzip(&vec![0; MAX_EXPANDED_BYTES + 1])),
        ));
        let mut arm64 = fake_amd64_elf();
        arm64[18..20].copy_from_slice(&183u16.to_le_bytes());
        cases.push((
            "an ELF that does not match its signed target",
            go_bundle(&policy, &gzip(&arm64)),
        ));
        cases.push((
            "an unproven target",
            go_bundle_for(&policy, "linux-riscv64", &compressed),
        ));

        for (what, mutate) in [
            (
                "another machine",
                Box::new(|bytes: &mut Vec<u8>| bytes[18] = 3) as Box<dyn Fn(&mut Vec<u8>)>,
            ),
            (
                "an interpreter",
                Box::new(|bytes: &mut Vec<u8>| bytes[64] = 3),
            ),
            (
                "an executable stack",
                Box::new(|bytes: &mut Vec<u8>| bytes[124] = 7),
            ),
            (
                "a writable executable load",
                Box::new(|bytes: &mut Vec<u8>| bytes[68] = 7),
            ),
        ] {
            let mut elf = fake_amd64_elf();
            mutate(&mut elf);
            cases.push((what, go_bundle(&policy, &gzip(&elf))));
        }

        let mut unknown: Value =
            serde_json::from_slice(&go_bundle(&policy, &compressed)).expect("json");
        unknown["surprise"] = json!(true);
        cases.push((
            "an unknown envelope member",
            serde_json::to_vec(&unknown).expect("json"),
        ));
        let duplicate = String::from_utf8(go_bundle(&policy, &compressed))
            .expect("utf8")
            .replacen("{\"abi\"", "{\"v\":3,\"abi\"", 1)
            .into_bytes();
        cases.push(("a duplicate envelope member", duplicate));

        for (what, bytes) in cases {
            assert!(
                Bundles.check(&bytes, &go_expected(&policy)).is_err(),
                "accepted {what}"
            );
        }
    }

    #[test]
    fn malformed_or_mismatched_java_classes_are_refused() {
        let policy = Policy::default();
        let valid = fake_script_class();
        let mut cases = Vec::new();
        let mut preview = valid.clone();
        preview[4..6].copy_from_slice(&u16::MAX.to_be_bytes());
        cases.push(("a preview class", java_bundle(&policy, &preview)));
        let mut newer = valid.clone();
        newer[6..8].copy_from_slice(&62u16.to_be_bytes());
        cases.push(("another class major", java_bundle(&policy, &newer)));
        cases.push((
            "a mismatched internal class name",
            java_bundle(&policy, &fake_class_named("Other")),
        ));
        let mut jar: Value = serde_json::from_slice(&java_bundle(&policy, &valid)).expect("json");
        jar["files"] = json!({"script.jar": STANDARD.encode(&valid)});
        cases.push(("a jar", serde_json::to_vec(&jar).expect("json")));
        let mut arbitrary: Value =
            serde_json::from_slice(&java_bundle(&policy, &valid)).expect("json");
        arbitrary["files"] = json!({"Other.class": STANDARD.encode(fake_class_named("Other"))});
        cases.push((
            "an arbitrary class",
            serde_json::to_vec(&arbitrary).expect("json"),
        ));
        let mut unknown: Value =
            serde_json::from_slice(&java_bundle(&policy, &valid)).expect("json");
        unknown["surprise"] = json!(true);
        cases.push((
            "an unknown envelope member",
            serde_json::to_vec(&unknown).expect("json"),
        ));

        for (what, bytes) in cases {
            assert!(
                Bundles.check(&bytes, &java_expected(&policy)).is_err(),
                "accepted {what}"
            );
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
