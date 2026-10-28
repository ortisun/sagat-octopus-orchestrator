//! Explicit native trust and client identity; certificate/hostname verification stays enabled.
use crate::SagatError;
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime, pem::PemObject as _},
};
use std::sync::Arc;
use zeroize::Zeroizing;

#[derive(Clone, Default)]
pub struct TlsOptions {
    /// Additional PEM trust anchors; native platform roots remain enabled.
    pub ca_certificates: Vec<Vec<u8>>,
    pub identity: Option<TlsIdentity>,
    /// Certificate verification override. SNI follows the URL hostname; use that hostname in the
    /// URL when the server routes its certificate selection by SNI.
    pub server_name: Option<String>,
}
impl std::fmt::Debug for TlsOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsOptions")
            .field("extra_roots", &self.ca_certificates.len())
            .field("identity", &self.identity.as_ref().map(|_| "<redacted>"))
            .field(
                "server_name",
                &self.server_name.as_ref().map(|_| "<configured>"),
            )
            .finish()
    }
}
#[derive(Clone)]
pub struct TlsIdentity {
    certificate: Arc<Vec<u8>>,
    key: Arc<Zeroizing<Vec<u8>>>,
}
impl std::fmt::Debug for TlsIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TlsIdentity(<redacted>)")
    }
}
impl TlsIdentity {
    pub fn from_pem(
        certificate: impl AsRef<[u8]>,
        key: impl AsRef<[u8]>,
    ) -> Result<Self, SagatError> {
        let certificate = certificate.as_ref();
        let key = key.as_ref();
        if key.len() > 128 * 1024 {
            return Err(invalid());
        }
        certificates(certificate)?;
        PrivateKeyDer::from_pem_slice(key).map_err(|_| invalid())?;
        Ok(Self {
            certificate: Arc::new(certificate.to_vec()),
            key: Arc::new(Zeroizing::new(key.to_vec())),
        })
    }
    pub(crate) fn parsed(
        &self,
    ) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), SagatError> {
        Ok((
            certificates(self.certificate.as_slice())?,
            PrivateKeyDer::from_pem_slice(self.key.as_slice()).map_err(|_| invalid())?,
        ))
    }
}
fn certificates(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, SagatError> {
    if pem.is_empty() || pem.len() > 1024 * 1024 {
        return Err(invalid());
    }
    let certificates = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid())?;
    if certificates.is_empty() {
        return Err(invalid());
    }
    Ok(certificates)
}
pub(crate) fn configuration(options: &TlsOptions) -> Result<rustls::ClientConfig, SagatError> {
    if options.ca_certificates.len() > 16 {
        return Err(invalid());
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = Vec::new();
    for pem in &options.ca_certificates {
        roots.extend(certificates(pem)?);
    }
    #[cfg(not(target_os = "android"))]
    let verifier =
        rustls_platform_verifier::Verifier::new_with_extra_roots(roots, provider.clone())
            .map_err(|_| invalid())?;
    #[cfg(target_os = "android")]
    let verifier = {
        if !roots.is_empty() {
            return Err(invalid());
        }
        rustls_platform_verifier::Verifier::new(provider.clone()).map_err(|_| invalid())?
    };
    let verifier: Arc<dyn ServerCertVerifier> = match &options.server_name {
        Some(name) => Arc::new(NamedVerifier {
            inner: Arc::new(verifier),
            name: ServerName::try_from(name.clone()).map_err(|_| invalid())?,
        }),
        None => Arc::new(verifier),
    };
    // This installs a chain-and-name-validating verifier, never an accept-all implementation.
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| invalid())?
        .dangerous()
        .with_custom_certificate_verifier(verifier);
    match &options.identity {
        Some(identity) => {
            let (certificates, key) = identity.parsed()?;
            builder
                .with_client_auth_cert(certificates, key)
                .map_err(|_| invalid())
        }
        None => Ok(builder.with_no_client_auth()),
    }
}
struct NamedVerifier {
    inner: Arc<dyn ServerCertVerifier>,
    name: ServerName<'static>,
}
impl std::fmt::Debug for NamedVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NamedVerifier(<configured>)")
    }
}
impl ServerCertVerifier for NamedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.inner
            .verify_server_cert(end_entity, intermediates, &self.name, ocsp, now)
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, signature)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, signature)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}
fn invalid() -> SagatError {
    SagatError::new(
        "VALIDATION_FAILED",
        "TLS trust or identity configuration is invalid",
    )
}

pub(crate) fn grpc_configuration(
    options: &TlsOptions,
) -> Result<tonic::transport::ClientTlsConfig, SagatError> {
    // Apply the same bounded PEM/key/name checks as the REST transport before opening a socket.
    configuration(options)?;
    let mut config = tonic::transport::ClientTlsConfig::new().with_native_roots();
    for pem in &options.ca_certificates {
        config = config.ca_certificate(tonic::transport::Certificate::from_pem(pem));
    }
    if let Some(name) = &options.server_name {
        config = config.domain_name(name);
    }
    if let Some(identity) = &options.identity {
        config = config.identity(tonic::transport::Identity::from_pem(
            identity.certificate.as_slice(),
            identity.key.as_slice(),
        ));
    }
    Ok(config)
}
