use crate::{ExecutionError, frame::BoundedStream};
use rustls::{
    ClientConfig, RootCertStore,
    pki_types::{CertificateDer, ServerName, pem::PemObject},
};
use sagat_domain::database::TlsPolicy;
use std::{
    net::SocketAddr,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};

pub(crate) trait Socket: AsyncRead + AsyncWrite + Unpin + Send {}
impl<S: AsyncRead + AsyncWrite + Unpin + Send> Socket for S {}
pub(crate) type TargetStream = BoundedStream<Box<dyn Socket>>;

pub(crate) async fn socket(
    address: SocketAddr,
    host: &str,
    tls: &TlsPolicy,
    development: bool,
) -> Result<Box<dyn Socket>, ExecutionError> {
    if matches!(tls, TlsPolicy::DevelopmentPlaintext) && !development {
        return Err(ExecutionError::Tls);
    }
    let mut tcp = TcpStream::connect(address)
        .await
        .map_err(|_| ExecutionError::Connection)?;
    tcp.set_nodelay(true)
        .map_err(|_| ExecutionError::Connection)?;
    match tls {
        TlsPolicy::DevelopmentPlaintext if development => Ok(Box::new(tcp)),
        TlsPolicy::DevelopmentPlaintext => Err(ExecutionError::Tls),
        TlsPolicy::VerifyFull { ca_pem } => {
            let mut roots = RootCertStore::empty();
            if let Some(pem) = ca_pem {
                for certificate in CertificateDer::pem_slice_iter(pem.as_bytes()) {
                    roots
                        .add(certificate.map_err(|_| ExecutionError::Tls)?)
                        .map_err(|_| ExecutionError::Tls)?;
                }
            } else {
                for certificate in rustls_native_certs::load_native_certs().certs {
                    roots.add(certificate).map_err(|_| ExecutionError::Tls)?;
                }
            }
            if roots.is_empty() {
                return Err(ExecutionError::Tls);
            }
            let config = ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .map_err(|_| ExecutionError::Tls)?
            .with_root_certificates(roots)
            .with_no_client_auth();
            let name = ServerName::try_from(host.to_owned()).map_err(|_| ExecutionError::Tls)?;
            // PostgreSQL SSLRequest on the already pinned socket, then verified TLS. No downgrade.
            tcp.write_all(&[0, 0, 0, 8, 4, 210, 22, 47])
                .await
                .map_err(|_| ExecutionError::Tls)?;
            let mut answer = [0];
            tcp.read_exact(&mut answer)
                .await
                .map_err(|_| ExecutionError::Tls)?;
            if answer != *b"S" {
                return Err(ExecutionError::Tls);
            }
            let stream = tokio_rustls::TlsConnector::from(Arc::new(config))
                .connect(name, tcp)
                .await
                .map_err(|_| ExecutionError::Tls)?;
            Ok(Box::new(stream))
        }
    }
}
pub(crate) fn bounded(
    stream: Box<dyn Socket>,
    result_limit: u32,
    exceeded: Arc<AtomicBool>,
) -> TargetStream {
    // Allow bounded protocol metadata as well as rows; actual serialized results have a separate
    // strict byte ceiling. The aggregate budget also bounds notices and startup parameter floods.
    let bytes = result_limit as usize;
    BoundedStream::new(stream, bytes + 64 * 1024, bytes * 4 + 256 * 1024, exceeded)
}
