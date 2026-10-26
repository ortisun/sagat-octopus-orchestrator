//! Strict PostgreSQL credential JSON. Neither parser diagnostics nor credential fields escape.
use sagat_application::database::{
    DatabaseCredentials, DatabaseCredentialsCodec, InvalidDatabaseCredential,
};
use sagat_domain::secrets::SecretValue;
use serde::Deserialize;
use zeroize::Zeroizing;

pub struct JsonDatabaseCredentials;
struct SensitiveString(Zeroizing<String>);
impl<'de> Deserialize<'de> for SensitiveString {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|s| Self(Zeroizing::new(s)))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialJson {
    username: SensitiveString,
    password: SensitiveString,
}
impl DatabaseCredentialsCodec for JsonDatabaseCredentials {
    fn decode(
        &self,
        value: &SecretValue,
    ) -> Result<DatabaseCredentials, InvalidDatabaseCredential> {
        let parsed: CredentialJson =
            serde_json::from_slice(value.expose()).map_err(|_| InvalidDatabaseCredential)?;
        let username = &parsed.username.0;
        let password = &parsed.password.0;
        if username.is_empty()
            || username.len() > 63
            || username.chars().any(char::is_control)
            || password.is_empty()
            || password.contains('\0')
        {
            return Err(InvalidDatabaseCredential);
        }
        Ok(DatabaseCredentials {
            username: SecretValue::new(username.as_bytes().to_vec())
                .map_err(|_| InvalidDatabaseCredential)?,
            password: SecretValue::new(password.as_bytes().to_vec())
                .map_err(|_| InvalidDatabaseCredential)?,
        })
    }
    fn contains_credential(
        &self,
        value: &SecretValue,
        content: &[u8],
    ) -> Result<bool, InvalidDatabaseCredential> {
        use serde::de::DeserializeSeed as _;
        if content.len() > sagat_domain::database::DatabaseLimits::MAX.max_result_bytes as usize {
            return Err(InvalidDatabaseCredential);
        }
        let parsed = self.decode(value)?;
        let needles = [value, &parsed.username, &parsed.password]
            .map(|s| memchr::memmem::Finder::new(s.expose()));
        if needles.iter().any(|needle| needle.find(content).is_some()) {
            return Ok(true);
        }
        let found = std::cell::Cell::new(false);
        let scan = ContentScan {
            needles: &needles,
            found: &found,
        };
        let mut json = serde_json::Deserializer::from_slice(content);
        scan.deserialize(&mut json)
            .map_err(|_| InvalidDatabaseCredential)?;
        json.end().map_err(|_| InvalidDatabaseCredential)?;
        Ok(found.get())
    }
}

/// Walk every JSON string, including duplicate map keys/values, without first constructing a map
/// that would discard duplicates. Check both decoded Unicode/escapes and base64 binary content.
/// This recognizes the canonical result encodings; it cannot prevent arbitrary transformations
/// performed by authorized SQL or a compromised executor.
#[derive(Clone, Copy)]
struct ContentScan<'a, 'secret> {
    needles: &'a [memchr::memmem::Finder<'secret>],
    found: &'a std::cell::Cell<bool>,
}
impl ContentScan<'_, '_> {
    fn check(&self, bytes: &[u8]) {
        if self
            .needles
            .iter()
            .any(|needle| needle.find(bytes).is_some())
        {
            self.found.set(true);
        }
    }
}
impl<'de> serde::de::DeserializeSeed<'de> for ContentScan<'_, '_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}
impl<'de> serde::de::Visitor<'de> for ContentScan<'_, '_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("bounded database result JSON")
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<(), E> {
        use base64::Engine as _;
        self.check(value.as_bytes());
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(value) {
            self.check(&Zeroizing::new(decoded));
        }
        Ok(())
    }
    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<(), E> {
        self.visit_str(&Zeroizing::new(value))
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<(), A::Error> {
        while sequence.next_element_seed(self)?.is_some() {}
        Ok(())
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while map.next_key_seed(self)?.is_some() {
            map.next_value_seed(self)?;
        }
        Ok(())
    }
}
