//! The credentials of a connector, read from a secret of the namespace.
//!
//! The secret is a JSON document whose shape depends on the broker. Nothing here is ever logged or
//! shown: `Debug` prints that a value exists and not what it is.

use sagat_application::connectors::{ConnectorRecord, ConnectorRuntime};
use sagat_application::secrets::SecretReader;
use sagat_domain::connectors::ConnectorKind;
use serde_json::Value;

/// The credentials document is not a JSON object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAnObject;

/// How a consumer authenticates to its broker.
#[derive(Clone, Default)]
pub struct Credentials {
    values: serde_json::Map<String, Value>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials(<redacted>)")
    }
}

impl Credentials {
    /// None at all: a broker that needs no authentication.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Reads the credentials document. A document of the wrong shape for the broker is refused when
    /// it is used, not here, so that the message is about the broker.
    ///
    /// # Errors
    /// [`NotAnObject`] when the text is not a JSON object.
    pub fn parse(text: &[u8]) -> Result<Self, NotAnObject> {
        match serde_json::from_slice::<Value>(text) {
            Ok(Value::Object(values)) => Ok(Self { values }),
            _ => Err(NotAnObject),
        }
    }

    #[must_use]
    pub fn text(&self, key: &str) -> Option<&str> {
        self.values.get(key).and_then(Value::as_str)
    }
}

/// Redis authentication and trust material, validated before any connection is opened.
pub(crate) struct RedisCredentials<'a> {
    pub username: Option<&'a str>,
    pub password: Option<&'a str>,
    pub ca_cert: Option<&'a str>,
}
impl Credentials {
    pub(crate) fn redis(&self, named: bool) -> Result<RedisCredentials<'_>, ()> {
        if self
            .values
            .keys()
            .any(|key| !matches!(key.as_str(), "username" | "password" | "caCert"))
        {
            return Err(());
        }
        for (key, value) in &self.values {
            let limit = match key.as_str() {
                "username" => 256,
                "password" => 8192,
                _ => 48 * 1024,
            };
            if !value
                .as_str()
                .is_some_and(|v| !v.is_empty() && v.len() <= limit)
            {
                return Err(());
            }
        }
        let result = RedisCredentials {
            username: self.text("username"),
            password: self.text("password"),
            ca_cert: self.text("caCert"),
        };
        if (result.username.is_some() && result.password.is_none())
            || (named && result.password.is_none() && result.ca_cert.is_none())
        {
            return Err(());
        }
        Ok(result)
    }
}

/// Why a connector's credentials could not be had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialsError {
    /// The store could not be read.
    Store,
    /// The secret no longer exists, or no key of the ring opens it, or it is not a JSON object.
    Unusable,
}

/// Opens the credentials secret of `connector`, if it names one.
///
/// # Errors
/// [`CredentialsError`].
pub async fn open<R: ConnectorRuntime>(
    runtime: &R,
    cipher: &SecretReader,
    connector: &ConnectorRecord,
) -> Result<Credentials, CredentialsError> {
    if connector.secret_name.is_none() {
        return Ok(Credentials::none());
    }
    let stored = runtime
        .credentials(connector)
        .await
        .map_err(|_| CredentialsError::Store)?
        .ok_or(CredentialsError::Unusable)?;
    let value = cipher
        .resolve(&stored)
        .await
        .map_err(|_| CredentialsError::Unusable)?;
    Credentials::parse(value.expose()).map_err(|NotAnObject| CredentialsError::Unusable)
}

/// What kind of credentials a broker expects, for the message when they are wrong.
#[must_use]
pub const fn expects(kind: ConnectorKind) -> &'static str {
    match kind {
        ConnectorKind::Nats => {
            r#"{"token": "..."}, {"user": "...", "password": "..."} or {"creds": "<contents of a .creds file>"}"#
        }
        ConnectorKind::Kafka => {
            r#"{"mechanism": "PLAIN" | "SCRAM-SHA-256" | "SCRAM-SHA-512", "username": "...", "password": "..."}"#
        }
        ConnectorKind::Sqs => {
            r#"{"accessKeyId": "...", "secretAccessKey": "...", "sessionToken": "..."}"#
        }
    }
}
