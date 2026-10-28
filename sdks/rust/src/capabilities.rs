//! Attempt-scoped credentials. Retained views cannot extend a handler's capability lifetime.
use crate::{
    SagatError, protocol as pb,
    worker_rpc::{Rpc, error_of},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio_util::sync::CancellationToken;
use zeroize::{Zeroize, Zeroizing};
type Values = BTreeMap<String, Zeroizing<Vec<u8>>>;
struct SecretState {
    active: AtomicBool,
    values: Mutex<Option<Values>>,
}
#[derive(Clone)]
struct Authorization {
    rpc: Rpc,
    request: pb::ResolveSecretsRequest,
}
impl Drop for Authorization {
    fn drop(&mut self) {
        self.request.lease_token.zeroize();
    }
}
pub(crate) struct Capabilities {
    authorization: Mutex<Option<Authorization>>,
    minor: u32,
    state: Arc<SecretState>,
    resolving: tokio::sync::Mutex<()>,
    ended: CancellationToken,
}
impl Capabilities {
    pub fn new(rpc: Rpc, minor: u32, mut lease: pb::TaskLease) -> Self {
        let request = pb::ResolveSecretsRequest {
            task_id: lease.task_id,
            run_id: lease.run_id,
            step_id: lease.step_id,
            attempt: lease.attempt,
            generation: lease.generation,
            lease_token: std::mem::take(&mut lease.lease_token),
            traceparent: lease.traceparent,
            instance: if minor >= 5 { lease.instance } else { None },
        };
        Self {
            authorization: Mutex::new(Some(Authorization { rpc, request })),
            minor,
            state: Arc::new(SecretState {
                active: AtomicBool::new(true),
                values: Mutex::new(None),
            }),
            resolving: tokio::sync::Mutex::new(()),
            ended: CancellationToken::new(),
        }
    }
    pub fn invalidate(&self) {
        self.state.active.store(false, Ordering::Release);
        self.ended.cancel();
        if let Ok(mut authorization) = self.authorization.lock() {
            authorization.take();
        }
        if let Ok(mut values) = self.state.values.lock() {
            values.take();
        }
    }
    #[cfg(test)]
    pub(crate) fn authorization_retained(&self) -> bool {
        self.authorization.lock().is_ok_and(|v| v.is_some())
    }
    pub async fn secrets(&self) -> Result<SecretSet, SagatError> {
        self.check()?;
        if self.minor < 2 {
            return Err(SagatError::new(
                "UNSUPPORTED_PROTOCOL",
                "secrets require worker protocol 1.2",
            ));
        }
        let _single = self.resolving.lock().await;
        self.check()?;
        if self.state.values.lock().map_err(|_| ended())?.is_some() {
            return Ok(SecretSet(self.state.clone()));
        }
        let authorization = self
            .authorization
            .lock()
            .map_err(|_| ended())?
            .as_ref()
            .cloned()
            .ok_or_else(ended)?;
        let operation = async move {
            let request = authorization
                .rpc
                .request_with(
                    || {
                        self.check()?;
                        Ok(authorization.request.clone())
                    },
                    &authorization.request.traceparent,
                )
                .await?;
            authorization
                .rpc
                .client
                .clone()
                // At most sixteen 16-KiB values plus names, versions and protobuf framing.
                .max_decoding_message_size(512 * 1024)
                .resolve_secrets(request)
                .await
                .map_err(|e| error_of("ResolveSecrets", e))
                .map(|r| r.into_inner())
        };
        let reply = tokio::select! {biased;_=self.ended.cancelled()=>return Err(ended()),result=operation=>result?};
        let values = validate(reply)?;
        let mut cache = self.state.values.lock().map_err(|_| ended())?;
        self.check()?;
        *cache = Some(values);
        Ok(SecretSet(self.state.clone()))
    }
    fn check(&self) -> Result<(), SagatError> {
        if self.state.active.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(ended())
        }
    }
}
pub(crate) struct AttemptGuard(pub Arc<Capabilities>);
impl Drop for AttemptGuard {
    fn drop(&mut self) {
        self.0.invalidate();
    }
}
/// A view of the current attempt's secrets. All accesses fail after the handler returns.
#[derive(Clone)]
pub struct SecretSet(Arc<SecretState>);
impl std::fmt::Debug for SecretSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretSet(<redacted>)")
    }
}
impl SecretSet {
    pub fn names(&self) -> Result<Vec<String>, SagatError> {
        let values = self.0.values.lock().map_err(|_| ended())?;
        if !self.0.active.load(Ordering::Acquire) {
            return Err(ended());
        }
        Ok(values.as_ref().ok_or_else(ended)?.keys().cloned().collect())
    }
    /// The caller owns a separate zeroizing copy; the SDK never exposes its cache buffer.
    pub fn get(&self, name: &str) -> Result<Option<SecretValue>, SagatError> {
        let values = self.0.values.lock().map_err(|_| ended())?;
        if !self.0.active.load(Ordering::Acquire) {
            return Err(ended());
        }
        Ok(values
            .as_ref()
            .ok_or_else(ended)?
            .get(name)
            .map(|v| SecretValue(Zeroizing::new(v.to_vec()))))
    }
}
pub struct SecretValue(Zeroizing<Vec<u8>>);
impl SecretValue {
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }
    pub fn as_text(&self) -> Result<&str, SagatError> {
        std::str::from_utf8(self.as_bytes())
            .map_err(|_| SagatError::new("VALIDATION_FAILED", "the secret is not UTF-8 text"))
    }
}
impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue(<redacted>)")
    }
}
fn ended() -> SagatError {
    SagatError::new(
        "VALIDATION_FAILED",
        "the attempt is over; its secrets have been dropped",
    )
}

fn validate(reply: pb::ResolveSecretsResponse) -> Result<Values, SagatError> {
    // Wrap every credential before validation; even a malformed answer drops zeroizing buffers.
    let received: Vec<_> = reply
        .secrets
        .into_iter()
        .map(|secret| (secret.name, Zeroizing::new(secret.value), secret.version))
        .collect();
    if received.len() > 16 {
        return Err(SagatError::new(
            "PROTOCOL",
            "invalid secret resolution response",
        ));
    }
    let mut values = BTreeMap::new();
    for (name, value, version) in received {
        if name.is_empty()
            || name.len() > 64
            || !name.as_bytes()[0].is_ascii_lowercase()
            || !name.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
            })
            || value.is_empty()
            || value.len() > 16 * 1024
            || version == 0
            || values.contains_key(&name)
        {
            return Err(SagatError::new(
                "PROTOCOL",
                "invalid secret resolution response",
            ));
        }
        values.insert(name, value);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn secret(name: &str, value: Vec<u8>) -> pb::ResolvedSecret {
        pb::ResolvedSecret {
            name: name.into(),
            value,
            version: 1,
        }
    }
    #[test]
    fn malformed_names_do_not_become_a_successful_cache() {
        assert!(
            validate(pb::ResolveSecretsResponse {
                secrets: vec![secret("BAD\n", vec![1])]
            })
            .is_err()
        );
    }
    #[test]
    fn empty_and_oversized_values_do_not_become_a_successful_cache() {
        for value in [vec![], vec![1; 16385]] {
            assert!(
                validate(pb::ResolveSecretsResponse {
                    secrets: vec![secret("database", value)]
                })
                .is_err()
            );
        }
    }
    #[test]
    fn too_many_secrets_do_not_become_a_successful_cache() {
        assert!(
            validate(pb::ResolveSecretsResponse {
                secrets: (0..17)
                    .map(|i| secret(&format!("key-{i}"), vec![1]))
                    .collect()
            })
            .is_err()
        );
    }
}

mod blobs;

pub use blobs::BlobReader;
