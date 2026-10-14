//! Deployment-approved webhook identities; endpoint authors cannot select a key URL.

use std::collections::HashMap;
use std::sync::Arc;

use sagat_application::error::AppError;
use sagat_application::events::WebhookIdentityVerifier;
use sagat_domain::identity::Slug;
use serde::Deserialize;

use super::{AuthConfig, AuthError, HttpJwksFetcher, TokenVerifier};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebhookOidcConfig {
    pub name: String,
    pub issuer: String,
    pub audience: String,
    pub jwks_uri: String,
    #[serde(default)]
    pub required_token_type: Option<String>,
}

/// A separate bounded key cache for each issuer/audience profile.
#[derive(Default)]
pub struct WebhookOidcVerifiers {
    profiles: HashMap<String, TokenVerifier>,
}

impl WebhookOidcVerifiers {
    /// Build deployment-controlled profiles. Public endpoint settings cannot call this path.
    ///
    /// # Errors
    /// A static reason, without configuration values, for invalid or duplicate profiles.
    pub fn new(configs: &[WebhookOidcConfig], development: bool) -> Result<Self, &'static str> {
        if configs.len() > 32 {
            return Err("at most 32 webhook OIDC profiles are allowed");
        }
        let mut profiles = HashMap::new();
        for item in configs {
            if Slug::parse(&item.name).is_err()
                || item.issuer.len() > 2048
                || item.audience.is_empty()
                || item.audience.len() > 512
                || item.jwks_uri.len() > 2048
            {
                return Err("invalid webhook OIDC profile");
            }
            for url in [&item.issuer, &item.jwks_uri] {
                let parsed = reqwest::Url::parse(url).map_err(|_| "invalid webhook OIDC URL")?;
                if parsed.host_str().is_none()
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                    || parsed.fragment().is_some()
                    || !(parsed.scheme() == "https" || (development && parsed.scheme() == "http"))
                {
                    return Err("webhook OIDC URLs require HTTPS without embedded credentials");
                }
            }
            let mut config = AuthConfig::new(&item.issuer, &item.audience, &item.jwks_uri);
            config.allow_insecure_jwks_http = development;
            config
                .required_token_type
                .clone_from(&item.required_token_type);
            let fetcher =
                HttpJwksFetcher::new(&config).map_err(|_| "webhook OIDC verifier unavailable")?;
            if profiles
                .insert(
                    item.name.clone(),
                    TokenVerifier::new(Arc::new(fetcher), &config),
                )
                .is_some()
            {
                return Err("duplicate webhook OIDC profile");
            }
        }
        Ok(Self { profiles })
    }
}

impl WebhookIdentityVerifier for WebhookOidcVerifiers {
    fn supports(&self, profile: &Slug) -> bool {
        self.profiles.contains_key(profile.as_str())
    }

    fn verify<'a>(
        &'a self,
        profile: &'a Slug,
        token: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, AppError>> + Send + 'a>>
    {
        Box::pin(async move {
            let verifier = self
                .profiles
                .get(profile.as_str())
                .ok_or(AppError::Unauthenticated)?;
            verifier
                .verify_identity(token)
                .await
                .map_err(|error| match error {
                    AuthError::Unavailable => AppError::Unavailable,
                    _ => AppError::Unauthenticated,
                })
        })
    }
}
