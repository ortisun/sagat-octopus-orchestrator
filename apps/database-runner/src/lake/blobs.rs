//! The blobs of a lake attempt, read under the lease with `ReadBlob` (feature 035, T035-035, D035-123 slice D, D035-129).
//!
//! A step names blobs that earlier steps of the same run committed, and the runner stages them into the attempt directory for the
//! host to read. The server checks the lease and audits the read; what this adds is that **what arrives is what was named, whole**:
//!
//! - the first message is the header, and it is for the blob that was asked for (a server that answers with another is refused);
//! - the chunks add up to the size the header declared, and not a byte more is taken (a stream that does not stop is cut at the
//!   declared size, not at the staging limit);
//! - at the end the SHA-256 of what was read is the one in the header. A blob that arrives truncated or altered is an error at the
//!   end of the read, and staging ([`AttemptDir::stage_blob`](sagat_infra_lake::staging::AttemptDir::stage_blob)) removes the
//!   partial file when the source fails, so a corrupt blob is never left in `in/` as if it were complete.
//!
//! The reader is generic over the stream of messages so that it is tested without a server; [`LeaseBlobs`] is the thin part that makes
//! the call.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Buf as _, Bytes};
use sagat_api_grpc::pb::{self, read_blob_response::Part};
use sagat_domain::blobs::BlobId;
use sagat_domain::workflow::Digest;
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncRead, ReadBuf};
use tokio_stream::{Stream, StreamExt as _};

use super::prepare::{BlobError, Blobs};
use crate::rpc::Rpc;

/// What the header promised.
#[derive(Debug)]
struct Promise {
    size: u64,
    sha256: String,
}

/// A blob being read: its bytes, checked against the header as they go by and at the end.
pub struct BlobReader<S> {
    stream: S,
    promise: Promise,
    chunk: Bytes,
    seen: u64,
    hasher: Sha256,
    state: State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Reading,
    /// The end was reached and the digest was right.
    Done,
    /// Something was wrong; every later read says so again.
    Failed,
}

fn corrupt() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "the blob is not what its header said",
    )
}

impl<S> BlobReader<S>
where
    S: Stream<Item = Result<pb::ReadBlobResponse, tonic::Status>> + Unpin,
{
    /// Reads the header from `stream` and checks it is for `blob`.
    ///
    /// # Errors
    /// The stream is empty or fails before the header, the first message is not a header, the header is for another blob, or its
    /// size or digest is not one.
    pub async fn start(mut stream: S, blob: BlobId) -> Result<Self, BlobError> {
        let Some(Ok(first)) = stream.next().await else {
            return Err(BlobError);
        };
        let Some(Part::Header(header)) = first.part else {
            return Err(BlobError);
        };
        let size = u64::try_from(header.size).map_err(|_| BlobError)?;
        let digest_ok = header.sha256.len() == 64
            && header
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if header.blob_id != blob.to_string() || !digest_ok {
            return Err(BlobError);
        }
        Ok(Self {
            stream,
            promise: Promise {
                size,
                sha256: header.sha256,
            },
            chunk: Bytes::new(),
            seen: 0,
            hasher: Sha256::new(),
            state: State::Reading,
        })
    }
}

impl<S> AsyncRead for BlobReader<S>
where
    S: Stream<Item = Result<pb::ReadBlobResponse, tonic::Status>> + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            match this.state {
                State::Done => return Poll::Ready(Ok(())),
                State::Failed => return Poll::Ready(Err(corrupt())),
                State::Reading => {}
            }
            if !this.chunk.is_empty() {
                let take = this.chunk.len().min(buf.remaining());
                buf.put_slice(&this.chunk[..take]);
                this.chunk.advance(take);
                return Poll::Ready(Ok(()));
            }
            match Pin::new(&mut this.stream).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(message))) => {
                    let Some(Part::Chunk(bytes)) = message.part else {
                        // a second header, or an empty message
                        this.state = State::Failed;
                        continue;
                    };
                    this.seen = this.seen.saturating_add(bytes.len() as u64);
                    if this.seen > this.promise.size {
                        // not a byte past the declared size is taken
                        this.state = State::Failed;
                        continue;
                    }
                    this.hasher.update(&bytes);
                    this.chunk = bytes;
                }
                Poll::Ready(Some(Err(_))) => {
                    this.state = State::Failed;
                    return Poll::Ready(Err(io::Error::other("the blob stream failed")));
                }
                Poll::Ready(None) => {
                    let digest = Digest::new(this.hasher.clone().finalize().into()).to_hex();
                    this.state = if this.seen == this.promise.size && digest == this.promise.sha256
                    {
                        State::Done
                    } else {
                        State::Failed
                    };
                }
            }
        }
    }
}

/// The blobs of one lease, read with `ReadBlob`.
pub struct LeaseBlobs {
    rpc: Rpc,
    lease: pb::TaskLease,
}

impl LeaseBlobs {
    #[expect(dead_code, reason = "the shell of the lake driver calls it (slice D)")]
    pub(crate) const fn new(rpc: Rpc, lease: pb::TaskLease) -> Self {
        Self { rpc, lease }
    }
}

impl Blobs for LeaseBlobs {
    type Reader = BlobReader<tonic::Streaming<pb::ReadBlobResponse>>;

    async fn open(&self, blob: BlobId) -> Result<Self::Reader, BlobError> {
        let lease = &self.lease;
        let request = self
            .rpc
            .request(pb::ReadBlobRequest {
                task_id: lease.task_id.clone(),
                run_id: lease.run_id.clone(),
                step_id: lease.step_id.clone(),
                attempt: lease.attempt,
                generation: lease.generation,
                lease_token: lease.lease_token.clone(),
                blob_id: blob.to_string(),
                instance: lease.instance.clone(),
            })
            .await
            .map_err(|_| BlobError)?;
        let stream = self
            .rpc
            .client
            .clone()
            .read_blob(request)
            .await
            .map_err(|_| BlobError)?
            .into_inner();
        BlobReader::start(stream, blob).await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use tokio::io::AsyncReadExt as _;

    use super::*;

    const BLOB: &str = "0192f5aa-0000-7000-8000-00000000000a";

    fn blob() -> BlobId {
        BLOB.parse().unwrap()
    }

    fn hex(bytes: &[u8]) -> String {
        Digest::new(Sha256::digest(bytes).into()).to_hex()
    }

    fn header(id: &str, size: i64, sha256: &str) -> pb::ReadBlobResponse {
        pb::ReadBlobResponse {
            part: Some(Part::Header(pb::ReadBlobHeader {
                blob_id: id.to_owned(),
                size,
                sha256: sha256.to_owned(),
                content_type: "application/x-ndjson".to_owned(),
            })),
        }
    }

    fn chunk(bytes: &[u8]) -> pb::ReadBlobResponse {
        pb::ReadBlobResponse {
            part: Some(Part::Chunk(Bytes::copy_from_slice(bytes))),
        }
    }

    type Messages = Vec<Result<pb::ReadBlobResponse, tonic::Status>>;

    /// The messages of a blob of `body` sent in pieces of `piece` bytes, with a header that says what `claim` says.
    fn blob_of(body: &[u8], piece: usize, claim: &[u8]) -> Messages {
        let mut messages: Messages = vec![Ok(header(
            BLOB,
            i64::try_from(claim.len()).unwrap(),
            &hex(claim),
        ))];
        messages.extend(body.chunks(piece.max(1)).map(|part| Ok(chunk(part))));
        messages
    }

    async fn read(messages: Messages) -> Result<Vec<u8>, String> {
        let mut reader = BlobReader::start(tokio_stream::iter(messages), blob())
            .await
            .map_err(|_| "start".to_owned())?;
        let mut out = Vec::new();
        reader
            .read_to_end(&mut out)
            .await
            .map(|_| out)
            .map_err(|error| error.to_string())
    }

    #[tokio::test]
    async fn a_blob_that_arrives_whole_is_read_whatever_the_size_of_the_pieces() {
        let body: Vec<u8> = (0..5000_u32).map(|n| (n % 251) as u8).collect();
        for piece in [1, 7, 1000, 5000, 9999] {
            assert_eq!(
                read(blob_of(&body, piece, &body)).await,
                Ok(body.clone()),
                "pieces of {piece}"
            );
        }
    }

    #[tokio::test]
    async fn an_empty_blob_is_an_empty_read() {
        assert_eq!(read(blob_of(b"", 10, b"")).await, Ok(Vec::new()));
    }

    #[tokio::test]
    async fn a_read_buffer_smaller_than_a_piece_gets_the_rest_next() {
        let body = b"0123456789abcdef".to_vec();
        let mut reader = BlobReader::start(tokio_stream::iter(blob_of(&body, 16, &body)), blob())
            .await
            .unwrap();
        let mut small = [0_u8; 5];
        let mut got = Vec::new();
        loop {
            let n = reader.read(&mut small).await.unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&small[..n]);
        }
        assert_eq!(got, body);
    }

    #[tokio::test]
    async fn a_blob_whose_bytes_are_not_the_ones_in_the_header_is_an_error_at_the_end() {
        let altered = b"0123456789abcdeX".to_vec();
        let said = b"0123456789abcdef".to_vec();
        let error = read(blob_of(&altered, 8, &said)).await.unwrap_err();
        assert!(error.contains("not what its header said"), "{error}");
    }

    #[tokio::test]
    async fn a_blob_that_ends_short_is_an_error_and_one_that_goes_past_its_size_is_cut() {
        let said = b"0123456789abcdef".to_vec();
        let error = read(blob_of(&said[..10], 4, &said)).await.unwrap_err();
        assert!(error.contains("not what its header said"), "short: {error}");

        // more bytes than the header said: refused as soon as the extra arrives, not at the end
        let mut longer = said.clone();
        longer.extend_from_slice(b"extra");
        let mut reader = BlobReader::start(tokio_stream::iter(blob_of(&longer, 16, &said)), blob())
            .await
            .unwrap();
        let mut out = Vec::new();
        let error = reader.read_to_end(&mut out).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(
            out.len() <= said.len(),
            "no byte past the declared size is handed on: {}",
            out.len()
        );
    }

    #[tokio::test]
    async fn a_header_whose_size_is_not_the_size_of_what_arrives_is_refused_even_when_the_digest_matches()
     {
        // sixteen bytes promised, ten sent, and the digest is of the ten: only the size can tell
        let said = b"0123456789abcdef";
        let messages: Messages = vec![
            Ok(header(BLOB, 16, &hex(&said[..10]))),
            Ok(chunk(&said[..10])),
        ];
        let error = read(messages).await.unwrap_err();
        assert!(error.contains("not what its header said"), "{error}");
    }

    #[tokio::test]
    async fn once_it_has_failed_every_read_says_so_again() {
        let mut reader = BlobReader::start(tokio_stream::iter(blob_of(b"abc", 3, b"abd")), blob())
            .await
            .unwrap();
        let mut out = Vec::new();
        assert!(reader.read_to_end(&mut out).await.is_err());
        assert!(reader.read_to_end(&mut out).await.is_err());
    }

    #[tokio::test]
    async fn a_stream_that_fails_in_the_middle_is_an_error() {
        let mut messages = blob_of(b"0123456789", 5, b"0123456789");
        messages.insert(2, Err(tonic::Status::unavailable("gone")));
        assert!(read(messages).await.is_err());
    }

    #[tokio::test]
    async fn a_second_header_in_the_middle_is_an_error() {
        let mut messages = blob_of(b"0123456789", 5, b"0123456789");
        messages.insert(2, Ok(header(BLOB, 10, &hex(b"0123456789"))));
        assert!(read(messages).await.is_err());
    }

    #[tokio::test]
    async fn a_header_that_is_not_for_the_blob_that_was_asked_for_is_refused() {
        let other = "0192f5aa-0000-7000-8000-00000000000b";
        let messages = vec![Ok(header(other, 3, &hex(b"abc"))), Ok(chunk(b"abc"))];
        assert_eq!(read(messages).await, Err("start".to_owned()));
    }

    #[tokio::test]
    async fn a_start_that_is_not_a_header_is_refused() {
        for messages in [
            Vec::new(),
            vec![Ok(chunk(b"abc"))],
            vec![Err(tonic::Status::unavailable("gone"))],
            vec![Ok(pb::ReadBlobResponse { part: None })],
            vec![Ok(header(BLOB, -1, &hex(b"")))],
            vec![Ok(header(BLOB, 3, "not-a-digest"))],
            vec![Ok(header(BLOB, 3, &hex(b"abc").to_uppercase()))],
            vec![Ok(header(BLOB, 3, &hex(b"abc")[..63]))],
        ] {
            assert_eq!(read(messages).await, Err("start".to_owned()));
        }
    }

    #[tokio::test]
    async fn a_corrupt_blob_is_not_left_in_the_attempt_directory_as_if_it_were_whole() {
        use sagat_infra_lake::protocol::{InputFormat, RelayToken};
        use sagat_infra_lake::staging::{AttemptDir, StagingLimits};

        let work = tempfile::tempdir().unwrap();
        let token = RelayToken::new("relay-token-canary-0123456789abcdef").unwrap();
        let mut directory = AttemptDir::create(
            work.path(),
            "0192f5aa-1111-4000-8000-000000000001",
            &token,
            StagingLimits::new(1 << 20, 1000).unwrap(),
        )
        .unwrap();

        let reader = BlobReader::start(
            tokio_stream::iter(blob_of(
                b"{\"a\":1}\n{\"a\":2}\n",
                4,
                b"{\"a\":1}\n{\"a\":3}\n",
            )),
            blob(),
        )
        .await
        .unwrap();
        assert!(
            directory
                .stage_blob(InputFormat::Ndjson, reader)
                .await
                .is_err()
        );
        assert!(
            !directory.path().join("in/blob-0.ndjson").exists(),
            "the partial file is removed"
        );

        // and a whole one is staged
        let body = b"{\"a\":1}\n{\"a\":2}\n".to_vec();
        let reader = BlobReader::start(tokio_stream::iter(blob_of(&body, 4, &body)), blob())
            .await
            .unwrap();
        let staged = directory
            .stage_blob(InputFormat::Ndjson, reader)
            .await
            .unwrap();
        assert_eq!(staged.bytes, body.len() as u64);
        assert_eq!(staged.records, Some(2));
    }
}
