//! OIDC/JWT authentication.

mod config;
mod jwks;
mod verifier;
mod webhooks;
pub use webhooks::{WebhookOidcConfig, WebhookOidcVerifiers};

pub use config::AuthConfig;
pub use jwks::{
    BoxFuture, HttpJwksFetcher, JwksCache, JwksError, JwksFetcher, VerificationKey,
    install_crypto_provider,
};
pub use verifier::{AuthError, InvalidReason, TokenVerifier, VerifiedClaims};
