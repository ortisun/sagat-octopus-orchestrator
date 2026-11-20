use futures_util::{FutureExt as _, future::BoxFuture};
use std::{future::Future, panic::AssertUnwindSafe, sync::Arc};
use zeroize::Zeroizing;

/// Opaque provider failure. Provider details never become SDK errors or logs.
#[derive(Debug, Clone, Copy)]
pub struct TokenProviderError;
impl std::fmt::Display for TokenProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("token provider failed")
    }
}
impl std::error::Error for TokenProviderError {}
type Provider = dyn Fn() -> BoxFuture<'static, Result<String, TokenProviderError>> + Send + Sync;
#[derive(Clone)]
enum Kind {
    Bearer(Arc<Zeroizing<String>>),
    Dynamic(Arc<Provider>),
}
/// Credentials stay private and are refreshed for each REST attempt or unary RPC.
#[derive(Clone)]
pub struct TokenSource(Kind);
impl std::fmt::Debug for TokenSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenSource(<redacted>)")
    }
}
impl TokenSource {
    pub fn bearer(value: impl Into<String>) -> Self {
        Self(Kind::Bearer(Arc::new(Zeroizing::new(value.into()))))
    }
    pub fn dynamic<F, Fut>(provider: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, TokenProviderError>> + Send + 'static,
    {
        Self(Kind::Dynamic(Arc::new(move || {
            let future = std::panic::catch_unwind(AssertUnwindSafe(&provider));
            async move {
                match future {
                    Ok(future) => AssertUnwindSafe(future)
                        .catch_unwind()
                        .await
                        .map_err(|_| TokenProviderError)?,
                    Err(_) => Err(TokenProviderError),
                }
            }
            .boxed()
        })))
    }
    pub(crate) fn can_refresh(&self) -> bool {
        matches!(self.0, Kind::Dynamic(_))
    }
    pub(crate) async fn get(&self) -> Result<Zeroizing<String>, super::SagatError> {
        let value = match &self.0 {
            Kind::Bearer(value) => value.as_str().to_owned(),
            Kind::Dynamic(provider) => provider()
                .await
                .map_err(|_| super::SagatError::provider_failure())?,
        };
        if value.is_empty()
            || !value.is_ascii()
            || value
                .bytes()
                .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
        {
            return Err(if self.can_refresh() {
                super::SagatError::provider_failure()
            } else {
                super::SagatError::new(
                    "VALIDATION_FAILED",
                    "a bearer token must be nonempty visible ASCII without whitespace",
                )
            });
        }
        Ok(Zeroizing::new(value))
    }
}
