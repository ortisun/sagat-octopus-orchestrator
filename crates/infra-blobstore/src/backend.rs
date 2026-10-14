//! The [`BlobBackend`] over `object_store`, and the builder that turns settings into one.

use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::prefix::PrefixStore;
use object_store::{
    ClientOptions, ObjectStore, ObjectStoreExt, PutPayload, RetryConfig, WriteMultipart,
};
use sagat_application::blobs::{
    BackendFailure, BlobBackend, BlobIoError, BlobKey, BoxFuture, ByteStream,
};

use crate::config::{BlobStoreConfig, ConfigError, Credentials, PartSize, R2Config, S3Config};

/// Parts of one upload sent at the same time. With the part size this bounds the memory one upload
/// holds: about (this + 1) × the part size.
const PARTS_IN_FLIGHT: usize = 4;

/// One request may take this long, a whole part included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A few quick retries on connection errors and 5xx, then give up: a caller is waiting on this, and
/// the layers above retry whole operations with their own policy.
fn retries() -> RetryConfig {
    RetryConfig {
        max_retries: 3,
        retry_timeout: Duration::from_secs(30),
        ..RetryConfig::default()
    }
}

/// `with_client_options` replaces every client option, `allow_http` included, so whether plain HTTP
/// is allowed travels inside the options instead of being set on the builder (where setting it first
/// and the options second silently turns it off again).
fn client_options(allow_http: bool) -> ClientOptions {
    ClientOptions::new()
        .with_timeout(REQUEST_TIMEOUT)
        .with_connect_timeout(CONNECT_TIMEOUT)
        .with_allow_http(allow_http)
}

/// Any [`ObjectStore`] as a [`BlobBackend`].
#[derive(Debug)]
pub struct ObjectStoreBackend {
    provider: &'static str,
    store: Arc<dyn ObjectStore>,
    part_size: usize,
}

/// Builds the backend the settings describe.
///
/// # Errors
/// [`ConfigError::Build`] if the store cannot be set up (a directory that cannot be created, a
/// malformed endpoint); the cause is logged, not returned.
pub fn build_backend(
    config: &BlobStoreConfig,
    part_size: PartSize,
) -> Result<Arc<dyn BlobBackend>, ConfigError> {
    // HTTPS to a store goes through reqwest with rustls and no provider of its own: the same `ring`
    // provider the rest of the workspace uses. Installing it twice is harmless.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (provider, store): (&'static str, Arc<dyn ObjectStore>) = match config {
        BlobStoreConfig::Memory => ("memory", Arc::new(InMemory::new())),
        BlobStoreConfig::Fs { root } => {
            std::fs::create_dir_all(root).map_err(|_| {
                tracing::error!("the blob directory could not be created");
                ConfigError::Build
            })?;
            let fs = LocalFileSystem::new_with_prefix(root).map_err(|_| {
                tracing::error!("the blob directory could not be opened");
                ConfigError::Build
            })?;
            ("fs", Arc::new(fs))
        }
        BlobStoreConfig::S3(s3) => ("s3", s3_store(s3)?),
        BlobStoreConfig::R2(r2) => ("r2", r2_store(r2)?),
    };
    let store = match config.prefix() {
        Some(prefix) => Arc::new(PrefixStore::new(store, prefix)) as Arc<dyn ObjectStore>,
        None => store,
    };
    Ok(Arc::new(ObjectStoreBackend {
        provider,
        store,
        part_size: part_size.bytes(),
    }))
}

impl BlobStoreConfig {
    fn prefix(&self) -> Option<&str> {
        match self {
            Self::S3(s3) => s3.prefix.as_deref(),
            Self::R2(r2) => r2.prefix.as_deref(),
            Self::Memory | Self::Fs { .. } => None,
        }
    }
}

fn s3_store(config: &S3Config) -> Result<Arc<dyn ObjectStore>, ConfigError> {
    // Explicit credentials are used as given and nothing else is read; without them the standard AWS
    // chain applies (environment, web identity token, container or instance role).
    let mut builder = if config.credentials.is_some() {
        AmazonS3Builder::new()
    } else {
        AmazonS3Builder::from_env()
    }
    .with_bucket_name(&config.bucket)
    .with_region(&config.region)
    .with_virtual_hosted_style_request(!config.path_style)
    .with_retry(retries())
    .with_client_options(client_options(config.allow_http));
    if let Some(endpoint) = &config.endpoint {
        builder = builder.with_endpoint(endpoint);
    }
    builder = with_credentials(builder, config.credentials.as_ref());
    finish(builder)
}

/// R2 is an S3 API: region `auto`, path-style addressing, and an endpoint made of the account id.
fn r2_store(config: &R2Config) -> Result<Arc<dyn ObjectStore>, ConfigError> {
    let endpoint = config.endpoint();
    let builder = AmazonS3Builder::new()
        .with_bucket_name(&config.bucket)
        .with_region("auto")
        .with_endpoint(&endpoint)
        .with_virtual_hosted_style_request(false)
        .with_retry(retries())
        .with_client_options(client_options(endpoint.starts_with("http://")));
    finish(with_credentials(builder, Some(&config.credentials)))
}

fn with_credentials(
    builder: AmazonS3Builder,
    credentials: Option<&Credentials>,
) -> AmazonS3Builder {
    let Some(credentials) = credentials else {
        return builder;
    };
    let builder = builder
        .with_access_key_id(&credentials.access_key_id)
        .with_secret_access_key(credentials.secret_access_key.expose());
    match &credentials.session_token {
        Some(token) => builder.with_token(token.expose()),
        None => builder,
    }
}

fn finish(builder: AmazonS3Builder) -> Result<Arc<dyn ObjectStore>, ConfigError> {
    builder
        .build()
        .map(|store| Arc::new(store) as Arc<dyn ObjectStore>)
        .map_err(|_| {
            tracing::error!("the S3 client could not be set up");
            ConfigError::Build
        })
}

impl ObjectStoreBackend {
    /// How a store's error looks to the callers. Logged here, with the provider and never with a
    /// credential, because the callers only get the category.
    fn failure(&self, error: &object_store::Error) -> BackendFailure {
        use object_store::Error as E;
        match error {
            E::NotFound { .. } => BackendFailure::NotFound,
            E::PermissionDenied { .. }
            | E::Unauthenticated { .. }
            | E::UnknownConfigurationKey { .. }
            | E::InvalidPath { .. }
            | E::NotSupported { .. }
            | E::NotImplemented { .. } => {
                tracing::error!(
                    provider = self.provider,
                    "the object store refused a request"
                );
                BackendFailure::Refused
            }
            _ => {
                tracing::warn!(
                    provider = self.provider,
                    "the object store failed a request"
                );
                BackendFailure::Unavailable
            }
        }
    }

    fn path(key: &BlobKey) -> Result<Path, BackendFailure> {
        // Keys are made of two UUIDs; this only fails if that stops being true.
        Path::parse(key.as_str()).map_err(|_| BackendFailure::Refused)
    }

    async fn put_stream(&self, key: &BlobKey, mut body: ByteStream) -> Result<(), BackendFailure> {
        let path = Self::path(key)?;

        // Up to one part is held back. A blob that ends inside it is a single request, which is
        // cheaper and atomic by itself; a longer one becomes a multipart upload.
        let mut remainder = Bytes::new();
        let first = next_part(&mut body, &mut remainder, self.part_size).await?;
        if first.len() < self.part_size {
            return self
                .store
                .put(&path, PutPayload::from(first))
                .await
                .map(|_| ())
                .map_err(|error| self.failure(&error));
        }

        let upload = self
            .store
            .put_multipart(&path)
            .await
            .map_err(|error| self.failure(&error))?;
        let mut writer = WriteMultipart::new_with_chunk_size(upload, self.part_size);
        writer.put(first);
        let streamed: Result<(), BackendFailure> = async {
            loop {
                writer
                    .wait_for_capacity(PARTS_IN_FLIGHT)
                    .await
                    .map_err(|error| self.failure(&error))?;
                let part = next_part(&mut body, &mut remainder, self.part_size).await?;
                if part.is_empty() {
                    break;
                }
                let last = part.len() < self.part_size;
                writer.put(part);
                if last {
                    break;
                }
            }
            Ok(())
        }
        .await;
        match streamed {
            Ok(()) => writer
                .finish()
                .await
                .map(|_| ())
                .map_err(|error| self.failure(&error)),
            Err(failure) => {
                // Without this the parts already sent would sit in the bucket, and be billed, until
                // a lifecycle rule collected them.
                if writer.abort().await.is_err() {
                    tracing::warn!(
                        provider = self.provider,
                        "an abandoned multipart upload could not be aborted; provider lifecycle cleanup is required"
                    );
                }
                Err(failure)
            }
        }
    }
}

/// Coalesce untrusted fragmentation into one allocation per part. Empty chunks retain no
/// metadata; large source chunks are split without queuing arbitrarily many multipart requests.
async fn next_part(
    body: &mut ByteStream,
    remainder: &mut Bytes,
    part_size: usize,
) -> Result<Bytes, BackendFailure> {
    let mut part = BytesMut::with_capacity(part_size);
    while part.len() < part_size {
        if remainder.is_empty() {
            match body.next().await {
                None => break,
                Some(Err(error)) => return Err(BackendFailure::Body(error)),
                Some(Ok(chunk)) => *remainder = chunk,
            }
        }
        let count = remainder.len().min(part_size - part.len());
        part.extend_from_slice(&remainder.split_to(count));
    }
    Ok(part.freeze())
}

impl BlobBackend for ObjectStoreBackend {
    fn provider(&self) -> &'static str {
        self.provider
    }

    fn put<'a>(
        &'a self,
        key: &'a BlobKey,
        body: ByteStream,
    ) -> BoxFuture<'a, Result<(), BackendFailure>> {
        Box::pin(self.put_stream(key, body))
    }

    fn get<'a>(&'a self, key: &'a BlobKey) -> BoxFuture<'a, Result<ByteStream, BackendFailure>> {
        Box::pin(async move {
            let path = Self::path(key)?;
            let result = self
                .store
                .get(&path)
                .await
                .map_err(|error| self.failure(&error))?;
            let provider = self.provider;
            let stream = result.into_stream().map(move |chunk| {
                chunk.map_err(|_| {
                    tracing::warn!(provider, "reading an object was interrupted");
                    BlobIoError::Interrupted
                })
            });
            Ok(Box::pin(stream) as ByteStream)
        })
    }

    fn delete<'a>(&'a self, key: &'a BlobKey) -> BoxFuture<'a, Result<(), BackendFailure>> {
        Box::pin(async move {
            let path = Self::path(key)?;
            match self.store.delete(&path).await {
                Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
                Err(error) => Err(self.failure(&error)),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TrackedChunk(Arc<AtomicUsize>, [u8; 1]);
    impl AsRef<[u8]> for TrackedChunk {
        fn as_ref(&self) -> &[u8] {
            &self.1
        }
    }
    impl Drop for TrackedChunk {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn hostile_fragmentation_does_not_retain_source_allocations() {
        let live = Arc::new(AtomicUsize::new(0));
        let observed = live.clone();
        let stream = futures_util::stream::iter((0..100_000).flat_map(move |_| {
            assert_eq!(observed.fetch_add(1, Ordering::SeqCst), 0);
            [
                Ok(Bytes::new()),
                Ok(Bytes::from_owner(TrackedChunk(observed.clone(), [42]))),
            ]
        }));
        let mut body: ByteStream = Box::pin(stream);
        let mut remainder = Bytes::new();
        let mut total = 0;
        loop {
            let part = next_part(&mut body, &mut remainder, 4096).await.unwrap();
            assert!(part.len() <= 4096);
            assert!(part.iter().all(|byte| *byte == 42));
            total += part.len();
            if part.len() < 4096 {
                break;
            }
        }
        assert_eq!(total, 100_000);
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn oversized_source_chunk_is_split_into_bounded_parts() {
        let mut body: ByteStream = Box::pin(futures_util::stream::iter([Ok(Bytes::from_static(
            b"abcdefghij",
        ))]));
        let mut remainder = Bytes::new();
        assert_eq!(
            next_part(&mut body, &mut remainder, 4).await.unwrap(),
            "abcd"
        );
        assert_eq!(
            next_part(&mut body, &mut remainder, 4).await.unwrap(),
            "efgh"
        );
        assert_eq!(next_part(&mut body, &mut remainder, 4).await.unwrap(), "ij");
    }
}
