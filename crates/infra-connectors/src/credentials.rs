//! The credentials of a connector, read from a secret of the namespace.
//!
//! The secret is a JSON document whose shape depends on the broker. Nothing here is ever logged or
//! shown: `Debug` prints that a value exists and not what it is.

use std::sync::Arc;

use sagat_application::connectors::{ConnectorRecord, ConnectorRuntime};
use sagat_domain::connectors::ConnectorKind;
use sagat_domain::secrets::{SecretCipher, associated_data};
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
    cipher: &Arc<dyn SecretCipher>,
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
    let aad = associated_data(
        connector.tenant_id,
        connector.namespace_id,
        &stored.meta.name,
        stored.meta.id,
        &stored.meta.key_id,
    );
    let value = cipher
        .open(&stored.sealed, &aad)
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
