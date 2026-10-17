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
        let parsed = self.decode(value)?;
        for secret in [value, &parsed.username, &parsed.password] {
            if content.windows(secret.len()).any(|w| w == secret.expose()) {
                return Ok(true);
            }
            let text =
                std::str::from_utf8(secret.expose()).map_err(|_| InvalidDatabaseCredential)?;
            let escaped =
                Zeroizing::new(serde_json::to_vec(text).map_err(|_| InvalidDatabaseCredential)?);
            // Match the escaped string content, including inside nested JSON, without quotes.
            let fragment = &escaped[1..escaped.len() - 1];
            if content.windows(fragment.len()).any(|w| w == fragment) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
