//! The fixture control endpoint uses HTTP. Configure crypto explicitly without global providers.
pub(super) fn client() -> Result<reqwest::Client, sagat_octopus_orchestrator_sdk::SagatError> {
    let invalid = || {
        sagat_octopus_orchestrator_sdk::SagatError::validation(
            "cannot configure fixture control transport",
        )
    };
    let tls = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| invalid())?
    .with_root_certificates(rustls::RootCertStore::empty())
    .with_no_client_auth();
    reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| invalid())
}
