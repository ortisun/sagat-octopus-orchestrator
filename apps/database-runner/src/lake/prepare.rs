//! Everything an attempt needs before a container is asked for (feature 035, T035-035, D035-123 slice B).
//!
//! From the lease input ([`AttemptInput`]), the connection the server resolved ([`Converted`]), a source of blobs and the
//! settings, to a [`Prepared`] attempt: the directory staged and filled, the request line, and the relay's configuration for
//! the user of the slot. Or a [`PrepareError`], whose code is the closed one the step is failed with. In order:
//!
//! 1. the mode is one the connection allows (the server checked at publication; a connection can have changed since, and a
//!    runner that wrote in a mode its connection no longer allows would be wrong in the one way that cannot be undone);
//! 2. the allowance of the relay is derived from the connection, which refuses a connection the host cannot open before
//!    anything is made;
//! 3. the token is made and the directory with it;
//! 4. the sources are staged against the **effective limit**, the intersection of the step's, the connection's and the
//!    settings' ([`effective`]); going over fails `ICEBERG_LIMIT_EXCEEDED` before any container starts;
//! 5. the request is built and encoded as a `Zeroizing` buffer.
//!
//! Every path that returns an error drops the directory, which removes itself and what is in it. The secrets of the
//! connection are moved into the request's connection and zeroised with it; the connection that came from the server is
//! consumed here.

use std::fmt;
use std::future::Future;
use std::time::{Duration, SystemTime};

use sagat_domain::blobs::BlobId;
use sagat_domain::database::DatabaseFailure;
use sagat_infra_lake::protocol::{
    AttemptInput, Connection, Limits, NodeLimits, Operation, ProbeBody, ProbeInput, Records,
    RelayAccess, RelayToken, Request, encode_request,
};
use sagat_infra_lake::relay::RelayConfig;
use sagat_infra_lake::staging::{AttemptDir, StagingError, StagingLimits};
use tokio::io::AsyncRead;
use zeroize::Zeroizing;

use super::connection::{Converted, Limits as ConnectionLimits, Secret};
use super::settings::Settings;

/// The only mode this runner serves (T035-080 and T035-085 add the others).
const APPEND: &str = "append";

/// Why an attempt could not be prepared. A closed kind: it names a part and never a value, and each is one code of the
/// contract's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PrepareError {
    /// The connection does not allow the mode of the step.
    #[error("the connection does not allow this write mode")]
    Mode,
    /// A connection the host cannot open, or an allowance the relay would refuse.
    #[error("the connection cannot be opened by the lake host")]
    Connection,
    /// The records are over the effective limit.
    #[error("the records are over the effective limit")]
    Limit,
    /// A blob could not be read or a file could not be written.
    #[error("a record source could not be read or staged")]
    Source,
    /// The request the host would be sent is not one it would accept.
    #[error("the request for the host cannot be built")]
    Request,
}

impl PrepareError {
    /// The code the step is failed with.
    #[must_use]
    pub const fn code(self) -> DatabaseFailure {
        match self {
            Self::Mode | Self::Connection | Self::Request => DatabaseFailure::Configuration,
            Self::Limit => DatabaseFailure::IcebergLimitExceeded,
            Self::Source => DatabaseFailure::WorkerError,
        }
    }
}

/// The limits an attempt runs under: the strictest of the step's, the connection's and the settings'.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effective {
    pub max_records: u64,
    pub max_bytes: u64,
    /// How long the attempt may take from the moment the runner starts on it, staging included: the step always names one,
    /// and the connection's may lower it. (The lease's own end is not a bound: a lease is renewed by its heartbeats, and one
    /// that is lost is a signal of its own.)
    pub timeout: Duration,
}

/// The intersection. A limit that is not set by the step or the connection is the next one up; the settings always set one.
#[must_use]
pub fn effective(step: NodeLimits, connection: ConnectionLimits, settings: &Settings) -> Effective {
    let records = [
        Some(u64::from(step.max_records)),
        connection.max_records.map(u64::from),
        Some(settings.max_records),
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or(settings.max_records);
    let timeout = [Some(step.timeout_ms), connection.timeout_ms]
        .into_iter()
        .flatten()
        .min()
        .map_or(
            Duration::from_millis(u64::from(step.timeout_ms)),
            |milliseconds| Duration::from_millis(u64::from(milliseconds)),
        );
    Effective {
        max_records: records,
        max_bytes: settings.max_bytes,
        timeout,
    }
}

/// A blob that could not be opened. It does not say why: the causes (not there, not allowed, the transport, a corrupt
/// stream) are the source's to log, and today every one of them fails the attempt the same way (G035-069 is about telling
/// them apart).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the blob could not be opened")]
pub struct BlobError;

/// Where the blobs come from: `ReadBlob` under the lease, in production.
pub trait Blobs: Send + Sync {
    type Reader: AsyncRead + Unpin + Send;

    /// Opens the blob for reading.
    ///
    /// # Errors
    /// Anything: the reason is the caller's to log, and the attempt fails with [`PrepareError::Source`].
    fn open(&self, blob: BlobId) -> impl Future<Output = Result<Self::Reader, BlobError>> + Send;
}

/// An attempt ready for a container.
pub struct Prepared {
    /// The directory, staged. Dropping it removes it.
    pub directory: AttemptDir,
    /// The request line, newline included. It holds the credentials, and is zeroed when dropped.
    pub request: Zeroizing<Vec<u8>>,
    /// The relay for this attempt: its allowance and its deny list, and the user of the slot as the only peer it serves.
    pub relay: RelayConfig,
    /// What was staged, for the choice of the class: the bytes of every source.
    pub staged_bytes: u64,
}

/// The request holds the credentials: only the shape of the thing is shown.
impl fmt::Debug for Prepared {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Prepared")
            .field("attempt", &self.directory.id())
            .field(
                "request",
                &format_args!("<{} bytes, not shown>", self.request.len()),
            )
            .field("staged_bytes", &self.staged_bytes)
            .finish_non_exhaustive()
    }
}

/// `at` as the host reads an instant: UTC, to the millisecond, `YYYY-MM-DDTHH:MM:SS.mmmZ`.
#[must_use]
pub fn instant_text(at: SystemTime) -> String {
    let at = time::OffsetDateTime::from(at);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

fn secrets(secret: &Secret) -> std::collections::BTreeMap<String, String> {
    secret
        .iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

fn staging(error: StagingError) -> PrepareError {
    match error {
        StagingError::LimitExceeded | StagingError::TooManyFiles => PrepareError::Limit,
        StagingError::AttemptId
        | StagingError::Exists
        | StagingError::Io
        | StagingError::ZeroLimit => PrepareError::Source,
    }
}

/// What is particular to one attempt.
#[derive(Debug, Clone, Copy)]
pub struct Attempt<'a> {
    /// The name of its directory and what the sandbox is asked for.
    pub id: &'a str,
    /// The user of the container, the only peer the relay serves: `40000 + slot` in production
    /// ([`sagat_infra_lake::sandbox_api::uid_for_slot`]), which the driver computes from the slot it chose.
    pub peer_uid: u32,
    /// When the host must be done, as the host is told.
    pub deadline_at: SystemTime,
    /// Development opens the loopback interface in the relay (the fixtures of the tests run on it).
    pub development: bool,
}

/// How many checks a probe runs, one after the other, each with the time the lease gives it.
const PROBE_CHECKS: u32 = 3;
/// What a probe may take besides its checks: the container to start, the host to connect, and the relay's first connections.
const PROBE_MARGIN_MS: u32 = 3_000;
/// The most a probe may take, whatever its checks are given: the `timeoutMs` of the fixed probe document (25 s), inside the 30 s
/// the step has.
const PROBE_CEILING_MS: u32 = 25_000;

/// The time an attempt that probes a connection may take, in milliseconds: its three checks and a margin, never more than the
/// step is allowed. The lease names the time of **one check** and the runner never sees the step's own limit, so this is where the
/// two meet. A connection may lower it further ([`effective`]); nothing raises it.
#[must_use]
pub const fn probe_timeout_ms(check_timeout_ms: u32) -> u32 {
    let wanted = PROBE_CHECKS
        .saturating_mul(check_timeout_ms)
        .saturating_add(PROBE_MARGIN_MS);
    if wanted < PROBE_CEILING_MS {
        wanted
    } else {
        PROBE_CEILING_MS
    }
}

/// [`probe_timeout_ms`] as a duration.
#[must_use]
pub const fn probe_timeout(check_timeout_ms: u32) -> Duration {
    Duration::from_millis(probe_timeout_ms(check_timeout_ms) as u64)
}

/// Prepares one attempt that probes a connection (D035-150): the directory the host finds its token in, the relay for the
/// user of the slot, and the request line of a probe. A probe stages nothing, has no mode to check (the connection's modes are
/// the writes') and no effect key, so there is nothing here that waits: the function does not await.
///
/// # Errors
/// [`PrepareError`], with nothing left on disk.
pub fn prepare_probe(
    input: &ProbeInput,
    connection: Converted,
    settings: &Settings,
    attempt: Attempt<'_>,
) -> Result<Prepared, PrepareError> {
    let Converted {
        catalog,
        storage,
        warehouse_locations,
        catalog_secret,
        storage_secret,
        ..
    } = connection;
    let connection = Connection::new(
        catalog,
        storage,
        warehouse_locations,
        secrets(&catalog_secret),
        secrets(&storage_secret),
    );
    let endpoints = connection
        .endpoint_policy()
        .map_err(|_| PrepareError::Connection)?;

    let token = RelayToken::generate().map_err(|_| PrepareError::Request)?;
    // the directory needs a budget to exist; a probe stages nothing in it
    let staging_limits = StagingLimits::new(1, 1).map_err(staging)?;
    let directory = AttemptDir::create(&settings.work_root, attempt.id, &token, staging_limits)
        .map_err(staging)?;

    let body = ProbeBody::new(connection, input)
        .into_body()
        .map_err(|_| PrepareError::Request)?;
    // the host reaches the relay where the container sees the attempt's directory
    let relay_access = RelayAccess::new(
        format!("/work/{}/relay.sock", directory.id()),
        token.clone(),
    );
    let request =
        encode_request(&Request::probe(relay_access, body)).map_err(|_| PrepareError::Request)?;

    let relay = RelayConfig::new(token, endpoints, settings.relay_allow.iter().copied())
        .with_deny(settings.relay_deny.iter().copied())
        .development(attempt.development)
        .with_peer_uid(attempt.peer_uid);
    Ok(Prepared {
        directory,
        request,
        relay,
        staged_bytes: 0,
    })
}

/// Prepares one attempt (see the module).
///
/// # Errors
/// [`PrepareError`], with nothing left on disk.
pub async fn prepare<B: Blobs>(
    mut input: AttemptInput,
    connection: Converted,
    blobs: &B,
    settings: &Settings,
    attempt: Attempt<'_>,
) -> Result<Prepared, PrepareError> {
    if !connection.allowed_modes.iter().any(|mode| mode == APPEND) {
        return Err(PrepareError::Mode);
    }
    let limits = effective(input.limits(), connection.limits, settings);

    let Converted {
        catalog,
        storage,
        warehouse_locations,
        catalog_secret,
        storage_secret,
        ..
    } = connection;
    let connection = Connection::new(
        catalog,
        storage,
        warehouse_locations,
        secrets(&catalog_secret),
        secrets(&storage_secret),
    );
    let endpoints = connection
        .endpoint_policy()
        .map_err(|_| PrepareError::Connection)?;

    let token = RelayToken::generate().map_err(|_| PrepareError::Request)?;
    let staging_limits =
        StagingLimits::new(limits.max_bytes, limits.max_records).map_err(staging)?;
    let mut directory = AttemptDir::create(&settings.work_root, attempt.id, &token, staging_limits)
        .map_err(staging)?;

    let mut staged_bytes = 0_u64;
    match input.take_records() {
        Records::Inline(records) => {
            let staged = directory.stage_inline(records).await.map_err(staging)?;
            staged_bytes = staged_bytes.saturating_add(staged.bytes);
        }
        Records::Blobs(list) => {
            for blob in &list {
                let reader = blobs
                    .open(blob.id)
                    .await
                    .map_err(|_| PrepareError::Source)?;
                let staged = directory
                    .stage_blob(blob.format, reader)
                    .await
                    .map_err(staging)?;
                staged_bytes = staged_bytes.saturating_add(staged.bytes);
            }
        }
    }

    let host_limits = Limits::new(
        settings.max_commit_retries,
        settings.max_decoded_bytes,
        settings.max_metadata_bytes,
        instant_text(attempt.deadline_at),
    )
    .map_err(|_| PrepareError::Request)?;
    let body = input
        .write_append(connection, directory.inputs(), host_limits)
        .into_body()
        .map_err(|_| PrepareError::Request)?;
    // the host reaches the relay where the container sees the attempt's directory
    let relay_access = RelayAccess::new(
        format!("/work/{}/relay.sock", directory.id()),
        token.clone(),
    );
    let request = encode_request(&Request::new(
        Operation::IcebergWrite,
        input.effect_key(),
        input.first_attempt_started_at(),
        relay_access,
        body,
    ))
    .map_err(|_| PrepareError::Request)?;

    let relay = RelayConfig::new(token, endpoints, settings.relay_allow.iter().copied())
        .with_deny(settings.relay_deny.iter().copied())
        .development(attempt.development)
        .with_peer_uid(attempt.peer_uid);
    Ok(Prepared {
        directory,
        request,
        relay,
        staged_bytes,
    })
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::time::UNIX_EPOCH;

    use sagat_api_grpc::pb;
    use sagat_infra_lake::relay::{Relay, SystemResolver, TcpConnector};
    use serde_json::{Value, json};

    use super::super::connection::convert;
    use super::super::fixtures::{SECRET, base, converted, message, my_uid, settings};
    use super::*;

    const ATTEMPT: &str = "0192f5aa-1111-7000-8000-000000000001";
    const BLOB_A: &str = "0192f5aa-0000-7000-8000-00000000000a";
    const BLOB_B: &str = "0192f5aa-0000-7000-8000-00000000000b";

    fn input(edit: impl FnOnce(&mut Value)) -> AttemptInput {
        let mut document = base();
        edit(&mut document);
        AttemptInput::read(&serde_json::to_vec(&document).unwrap()).unwrap()
    }

    fn attempt(peer_uid: u32) -> Attempt<'static> {
        Attempt {
            id: ATTEMPT,
            peer_uid,
            deadline_at: SystemTime::from(time::macros::datetime!(2026-11-05 10:10:00.123 UTC)),
            development: false,
        }
    }

    /// Blobs by id; an id that is not there cannot be opened.
    struct Held(HashMap<BlobId, Vec<u8>>);

    impl Blobs for Held {
        type Reader = Cursor<Vec<u8>>;

        async fn open(&self, blob: BlobId) -> Result<Self::Reader, BlobError> {
            self.0.get(&blob).cloned().map(Cursor::new).ok_or(BlobError)
        }
    }

    /// A blob that never ends.
    struct Endless;

    impl Blobs for Endless {
        type Reader = tokio::io::Repeat;

        async fn open(&self, _blob: BlobId) -> Result<Self::Reader, BlobError> {
            Ok(tokio::io::repeat(b'x'))
        }
    }

    fn nothing() -> Held {
        Held(HashMap::new())
    }

    fn entries(directory: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn line(prepared: &Prepared) -> Value {
        let text = &prepared.request;
        serde_json::from_slice(&text[..text.len() - 1]).unwrap()
    }

    #[tokio::test]
    async fn inline_records_are_staged_and_the_request_is_the_one_the_host_reads() {
        let work = tempfile::tempdir().unwrap();
        let prepared = prepare(
            input(|_| {}),
            converted(),
            &nothing(),
            &settings(work.path()),
            attempt(3),
        )
        .await
        .unwrap();

        let staged =
            std::fs::read_to_string(work.path().join(ATTEMPT).join("in/inline.ndjson")).unwrap();
        assert_eq!(staged, "{\"amount\":\"12.50\",\"sku\":\"A-1\"}\n");

        let request = line(&prepared);
        assert_eq!(request["protocol"], "sagat.lake.v1");
        assert_eq!(request["operation"], "icebergWrite");
        assert_eq!(request["effectKey"], "run-7f3a/load/1");
        assert_eq!(
            request["table"],
            json!({"namespace": ["sales"], "name": "orders"})
        );
        assert_eq!(
            request["inputs"],
            json!([{"file": "in/inline.ndjson", "format": "ndjson"}])
        );
        assert_eq!(request["limits"]["deadlineAt"], "2026-11-05T10:10:00.123Z");
        assert_eq!(request["limits"]["maxCommitRetries"], 4);
        assert_eq!(
            request["connection"]["credentials"]["catalog"]["clientSecret"],
            SECRET
        );
        assert_eq!(
            request["relay"]["socket"],
            format!("/work/{ATTEMPT}/relay.sock")
        );
        // the token the host is given is the one in the file the relay checks
        let token = std::fs::read_to_string(work.path().join(ATTEMPT).join("relay.token")).unwrap();
        assert_eq!(request["relay"]["token"], token.trim_end());
        assert_eq!(prepared.staged_bytes, staged.len() as u64);
    }

    #[tokio::test]
    async fn blobs_are_staged_in_the_order_written_with_their_formats() {
        let work = tempfile::tempdir().unwrap();
        let blobs = Held(HashMap::from([
            (BLOB_A.parse().unwrap(), b"{\"a\":1}\n{\"a\":2}\n".to_vec()),
            (BLOB_B.parse().unwrap(), b"PAR1-not-parsed".to_vec()),
        ]));
        let document = input(|d| {
            d["records"] = json!({"blobs": [
                {"id": BLOB_B, "format": "parquet"}, {"id": BLOB_A, "format": "ndjson"}
            ]});
        });
        let prepared = prepare(
            document,
            converted(),
            &blobs,
            &settings(work.path()),
            attempt(0),
        )
        .await
        .unwrap();
        let request = line(&prepared);
        assert_eq!(
            request["inputs"],
            json!([
                {"file": "in/blob-0.parquet", "format": "parquet"},
                {"file": "in/blob-1.ndjson", "format": "ndjson"}
            ])
        );
        let inside = work.path().join(ATTEMPT).join("in");
        assert_eq!(
            std::fs::read(inside.join("blob-0.parquet")).unwrap(),
            b"PAR1-not-parsed"
        );
        assert_eq!(
            std::fs::read(inside.join("blob-1.ndjson")).unwrap(),
            b"{\"a\":1}\n{\"a\":2}\n"
        );
        assert_eq!(prepared.staged_bytes, 15 + 16);
    }

    #[tokio::test]
    async fn a_connection_that_does_not_allow_the_mode_is_refused_and_nothing_is_made() {
        let work = tempfile::tempdir().unwrap();
        let mut connection = converted();
        connection.allowed_modes = vec!["overwrite".to_owned()];
        let error = prepare(
            input(|_| {}),
            connection,
            &nothing(),
            &settings(work.path()),
            attempt(0),
        )
        .await
        .unwrap_err();
        assert_eq!(error, PrepareError::Mode);
        assert_eq!(error.code(), DatabaseFailure::Configuration);
        assert!(entries(work.path()).is_empty());
    }

    #[tokio::test]
    async fn records_over_the_effective_limit_fail_before_a_container_and_leave_nothing() {
        let work = tempfile::tempdir().unwrap();
        let three =
            |d: &mut Value| d["records"] = json!({"inline": [{"a": 1}, {"a": 2}, {"a": 3}]});
        let mut small = settings(work.path());
        small.max_records = 2;
        let error = prepare(input(three), converted(), &nothing(), &small, attempt(0))
            .await
            .unwrap_err();
        assert_eq!(error, PrepareError::Limit);
        assert_eq!(error.code(), DatabaseFailure::IcebergLimitExceeded);
        assert!(
            entries(work.path()).is_empty(),
            "{:?}",
            entries(work.path())
        );

        // the last record that fits is taken
        small.max_records = 3;
        let prepared = prepare(input(three), converted(), &nothing(), &small, attempt(0))
            .await
            .unwrap();
        drop(prepared);
        assert!(
            entries(work.path()).is_empty(),
            "dropping the attempt removes its directory"
        );
    }

    #[tokio::test]
    async fn the_limit_of_the_step_and_of_the_connection_apply_before_the_settings() {
        let work = tempfile::tempdir().unwrap();
        let three = |d: &mut Value| {
            d["records"] = json!({"inline": [{"a": 1}, {"a": 2}, {"a": 3}]});
            d["limits"]["maxRecords"] = json!(2);
        };
        let error = prepare(
            input(three),
            converted(),
            &nothing(),
            &settings(work.path()),
            attempt(0),
        )
        .await
        .unwrap_err();
        assert_eq!(error, PrepareError::Limit, "the step says 2");

        let mut message = message();
        message.limits = Some(pb::LakeLimits {
            max_record_bytes: 0,
            max_records: 1,
            timeout_ms: 0,
        });
        let error = prepare(
            input(|d| d["records"] = json!({"inline": [{"a": 1}, {"a": 2}]})),
            convert(&mut message).unwrap(),
            &nothing(),
            &settings(work.path()),
            attempt(0),
        )
        .await
        .unwrap_err();
        assert_eq!(error, PrepareError::Limit, "the connection says 1");
        assert!(entries(work.path()).is_empty());
    }

    #[test]
    fn the_effective_limits_are_the_strictest_of_the_three() {
        let work = std::path::Path::new("/work");
        let step = NodeLimits {
            max_record_bytes: 4096,
            max_records: 500,
            timeout_ms: 60_000,
        };
        let none = ConnectionLimits {
            max_record_bytes: None,
            max_records: None,
            timeout_ms: None,
        };
        let by_settings = effective(step, none, &settings(work));
        assert_eq!(
            by_settings.max_records, 500,
            "the step is under the settings' 1000"
        );
        assert_eq!(by_settings.timeout, Duration::from_secs(60));
        assert_eq!(by_settings.max_bytes, 1 << 20);

        let tight = ConnectionLimits {
            max_records: Some(10),
            timeout_ms: Some(5_000),
            ..none
        };
        let by_connection = effective(step, tight, &settings(work));
        assert_eq!(by_connection.max_records, 10);
        assert_eq!(by_connection.timeout, Duration::from_secs(5));

        let mut low = settings(work);
        low.max_records = 7;
        assert_eq!(effective(step, tight, &low).max_records, 7);
        assert_eq!(effective(step, none, &low).max_records, 7);
    }

    #[tokio::test]
    async fn a_blob_that_cannot_be_opened_or_never_ends_fails_and_leaves_nothing() {
        let work = tempfile::tempdir().unwrap();
        let by_blob = |d: &mut Value| {
            d["records"] = json!({"blobs": [{"id": BLOB_A, "format": "ndjson"}]});
        };
        let error = prepare(
            input(by_blob),
            converted(),
            &nothing(),
            &settings(work.path()),
            attempt(0),
        )
        .await
        .unwrap_err();
        assert_eq!(error, PrepareError::Source);
        assert_eq!(error.code(), DatabaseFailure::WorkerError);
        assert!(entries(work.path()).is_empty());

        // an endless blob costs one chunk past the limit and no more
        let mut small = settings(work.path());
        small.max_bytes = 4096;
        let error = prepare(input(by_blob), converted(), &Endless, &small, attempt(0))
            .await
            .unwrap_err();
        assert_eq!(error, PrepareError::Limit);
        assert!(entries(work.path()).is_empty());
    }

    #[tokio::test]
    async fn a_connection_the_relay_cannot_allow_is_refused_before_the_directory_is_made() {
        let work = tempfile::tempdir().unwrap();
        let mut connection = converted();
        connection.storage =
            json!({"type": "gcs", "gcs": {"project": "p"}, "credentials": "static"});
        let error = prepare(
            input(|_| {}),
            connection,
            &nothing(),
            &settings(work.path()),
            attempt(0),
        )
        .await
        .unwrap_err();
        assert_eq!(error, PrepareError::Connection);
        assert_eq!(error.code(), DatabaseFailure::Configuration);
        assert!(entries(work.path()).is_empty());
    }

    #[tokio::test]
    async fn the_relay_serves_the_user_of_the_slot_and_only_that_user() {
        let work = tempfile::tempdir().unwrap();
        let prepared = prepare(
            input(|_| {}),
            converted(),
            &nothing(),
            &settings(work.path()),
            attempt(3),
        )
        .await
        .unwrap();
        let relay = Relay::new(
            prepared.relay,
            SystemResolver,
            TcpConnector::new(Duration::from_secs(1)),
        )
        .unwrap();
        assert!(
            relay.requires_peer(),
            "a relay without a peer check would serve whoever has the token"
        );
    }

    #[tokio::test]
    async fn an_attempt_directory_that_is_there_already_is_not_reused() {
        let work = tempfile::tempdir().unwrap();
        std::fs::create_dir(work.path().join(ATTEMPT)).unwrap();
        let error = prepare(
            input(|_| {}),
            converted(),
            &nothing(),
            &settings(work.path()),
            attempt(0),
        )
        .await
        .unwrap_err();
        assert_eq!(error, PrepareError::Source);
        // the directory that was there is not ours to remove
        assert!(work.path().join(ATTEMPT).is_dir());
    }

    #[test]
    fn the_instant_is_in_the_form_the_host_reads() {
        assert_eq!(instant_text(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        let at = SystemTime::from(time::macros::datetime!(2026-11-05 10:10:00.123 UTC));
        assert_eq!(instant_text(at), "2026-11-05T10:10:00.123Z");
        let at = SystemTime::from(time::macros::datetime!(2026-02-28 23:59:59.999999 UTC));
        assert_eq!(
            instant_text(at),
            "2026-02-28T23:59:59.999Z",
            "truncated, never rounded up"
        );
    }

    #[tokio::test]
    async fn what_is_debugged_shows_the_size_of_the_request_and_not_the_request() {
        let work = tempfile::tempdir().unwrap();
        let prepared = prepare(
            input(|_| {}),
            converted(),
            &nothing(),
            &settings(work.path()),
            attempt(0),
        )
        .await
        .unwrap();
        let shown = format!("{prepared:?}");
        assert!(!shown.contains(SECRET), "{shown}");
        assert!(!shown.contains("relay.sock"), "{shown}");
        assert!(shown.contains("bytes, not shown"), "{shown}");
    }

    #[test]
    fn every_error_is_one_code_of_the_closed_table() {
        for (error, code) in [
            (PrepareError::Mode, DatabaseFailure::Configuration),
            (PrepareError::Connection, DatabaseFailure::Configuration),
            (PrepareError::Request, DatabaseFailure::Configuration),
            (PrepareError::Limit, DatabaseFailure::IcebergLimitExceeded),
            (PrepareError::Source, DatabaseFailure::WorkerError),
        ] {
            assert_eq!(error.code(), code, "{error}");
            assert!(
                !error.code().retryable(),
                "{error}: a preparation failure is not retried blindly"
            );
        }
    }

    // -- the relay of a prepared attempt, reached as the host reaches it ----------------------------------------------

    struct Resolves(std::net::IpAddr);

    impl sagat_infra_lake::relay::Resolver for Resolves {
        async fn resolve(&self, _host: &str, _port: u16) -> std::io::Result<Vec<std::net::IpAddr>> {
            Ok(vec![self.0])
        }
    }

    struct NoNetwork;

    impl sagat_infra_lake::relay::Connector for NoNetwork {
        type Stream = tokio::io::DuplexStream;

        async fn connect(&self, _address: std::net::SocketAddr) -> std::io::Result<Self::Stream> {
            Err(std::io::Error::other("no network in this test"))
        }
    }

    /// What the relay of `prepared` answers when the host (this process, so `my_uid`) asks it to open the catalog's own endpoint,
    /// which the connection declares, and the name resolves to `address`; and what it counted as refused.
    async fn ask_the_relay(
        prepared: Prepared,
        address: std::net::IpAddr,
    ) -> (String, sagat_infra_lake::relay::RelayDenials) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let token = std::fs::read_to_string(prepared.directory.path().join("relay.token")).unwrap();
        let socket = prepared.directory.socket_path();
        let relay = Relay::new(prepared.relay, Resolves(address), NoNetwork).unwrap();
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let stop = tokio_util::sync::CancellationToken::new();
        let serving = tokio::spawn({
            let (relay, stop) = (relay.clone(), stop.clone());
            async move { relay.serve(listener, stop).await }
        });
        let mut stream = tokio::net::UnixStream::connect(&socket).await.unwrap();
        stream
            .write_all(
                format!(
                    "SAGAT-RELAY/1 {}\nCONNECT catalog.example:443 HTTP/1.1\r\n\r\n",
                    token.trim_end()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut answer = Vec::new();
        let _ = stream.read_to_end(&mut answer).await;
        stop.cancel();
        let _ = serving.await;
        let denials = relay.denials();
        drop(prepared.directory);
        (String::from_utf8_lossy(&answer).into_owned(), denials)
    }

    /// A short work root: a Unix socket's path is limited to about a hundred bytes.
    fn short_work() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("pr")
            .tempdir_in("/tmp")
            .unwrap()
    }

    async fn relay_prepared(
        work: &std::path::Path,
        configure: impl FnOnce(&mut Settings),
        peer_uid: u32,
        development: bool,
    ) -> Prepared {
        let mut settings = settings(work);
        configure(&mut settings);
        prepare(
            input(|_| {}),
            converted(),
            &nothing(),
            &settings,
            Attempt {
                peer_uid,
                development,
                ..attempt(0)
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn the_relay_refuses_an_address_the_operator_denied_even_when_a_declared_host_resolves_to_it()
     {
        let private: std::net::IpAddr = "10.0.0.5".parse().unwrap();

        // the platform's range is denied: the endpoint is declared by the connection and resolves into it
        let work = short_work();
        let prepared = relay_prepared(work.path(), |_| {}, my_uid(), false).await;
        let (answer, denials) = ask_the_relay(prepared, private).await;
        assert!(answer.starts_with("HTTP/1.1 403"), "{answer:?}");
        assert_eq!(denials.address_forbidden, 1);

        // without the deny list the same name and the same address are let through: the list is what refuses it
        let work = short_work();
        let prepared = relay_prepared(work.path(), |s| s.relay_deny.clear(), my_uid(), false).await;
        let (answer, denials) = ask_the_relay(prepared, private).await;
        assert!(!answer.starts_with("HTTP/1.1 403"), "{answer:?}");
        assert_eq!(denials.address_forbidden, 0);
    }

    #[tokio::test]
    async fn the_development_flag_opens_the_loopback_interface_and_only_in_development() {
        let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        let work = short_work();
        let prepared = relay_prepared(work.path(), |_| {}, my_uid(), false).await;
        let (answer, denials) = ask_the_relay(prepared, loopback).await;
        assert!(answer.starts_with("HTTP/1.1 403"), "{answer:?}");
        assert_eq!(denials.address_forbidden, 1);

        let work = short_work();
        let prepared = relay_prepared(work.path(), |_| {}, my_uid(), true).await;
        let (answer, denials) = ask_the_relay(prepared, loopback).await;
        assert!(!answer.starts_with("HTTP/1.1 403"), "{answer:?}");
        assert_eq!(denials.address_forbidden, 0);
    }

    #[tokio::test]
    async fn the_relay_serves_the_user_it_was_built_for_and_nobody_else() {
        let public: std::net::IpAddr = "93.184.216.34".parse().unwrap();
        let work = short_work();
        let prepared = relay_prepared(work.path(), |_| {}, my_uid(), false).await;
        let (answer, _) = ask_the_relay(prepared, public).await;
        assert!(
            answer.starts_with("HTTP/1.1"),
            "the user of the slot is served: {answer:?}"
        );

        // the same host, as another user than the one the relay was built for: closed before a byte is read
        let work = short_work();
        let prepared = relay_prepared(work.path(), |_| {}, my_uid().wrapping_add(1), false).await;
        let (answer, _) = ask_the_relay(prepared, public).await;
        assert_eq!(answer, "", "another user is not served");
    }

    /// A probe prepared on a work root short enough for the relay's socket, for the user of the slot `peer_uid`.
    fn probe_relay_prepared(
        work: &std::path::Path,
        configure: impl FnOnce(&mut Settings),
        peer_uid: u32,
    ) -> Prepared {
        let mut settings = settings(work);
        configure(&mut settings);
        prepare_probe(
            &ProbeInput::read(&super::super::fixtures::probe_input_json(|_| {})).unwrap(),
            converted(),
            &settings,
            Attempt {
                peer_uid,
                ..attempt(0)
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn the_relay_of_a_probe_serves_the_user_of_its_slot_and_nobody_else() {
        let public: std::net::IpAddr = "93.184.216.34".parse().unwrap();
        let work = short_work();
        let prepared = probe_relay_prepared(work.path(), |_| {}, my_uid());
        let (answer, _) = ask_the_relay(prepared, public).await;
        assert!(
            answer.starts_with("HTTP/1.1"),
            "the user of the slot is served: {answer:?}"
        );

        // the same host, as another user than the one the relay was built for: closed before a byte is read
        let work = short_work();
        let prepared = probe_relay_prepared(work.path(), |_| {}, my_uid().wrapping_add(1));
        let (answer, _) = ask_the_relay(prepared, public).await;
        assert_eq!(answer, "", "another user is not served");
    }

    #[tokio::test]
    async fn the_relay_of_a_probe_refuses_an_address_the_operator_denied_and_the_loopback() {
        let private: std::net::IpAddr = "10.0.0.5".parse().unwrap();
        let work = short_work();
        let prepared = probe_relay_prepared(work.path(), |_| {}, my_uid());
        let (answer, denials) = ask_the_relay(prepared, private).await;
        assert!(answer.starts_with("HTTP/1.1 403"), "{answer:?}");
        assert_eq!(denials.address_forbidden, 1);

        // a probe is the first thing a tenant can aim at an address of its own choosing: the loopback is closed to it too
        let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        let work = short_work();
        let prepared = probe_relay_prepared(work.path(), |_| {}, my_uid());
        let (answer, denials) = ask_the_relay(prepared, loopback).await;
        assert!(answer.starts_with("HTTP/1.1 403"), "{answer:?}");
        assert_eq!(denials.address_forbidden, 1);
    }

    // -- the probe (D035-150, slice S6b) -----------------------------------------------------------------------------------

    fn probe() -> ProbeInput {
        ProbeInput::read(&super::super::fixtures::probe_input_json(|_| {})).unwrap()
    }

    #[test]
    fn a_probe_is_prepared_with_the_request_the_host_reads_and_nothing_staged() {
        let work = tempfile::tempdir().unwrap();

        let prepared =
            prepare_probe(&probe(), converted(), &settings(work.path()), attempt(3)).unwrap();

        let request = line(&prepared);
        assert_eq!(request["protocol"], "sagat.lake.v1");
        assert_eq!(request["operation"], "probe");
        // a probe writes nothing that a later attempt would look for: the host refuses a line that names these
        for member in [
            "effectKey",
            "firstAttemptStartedAt",
            "table",
            "inputs",
            "limits",
        ] {
            assert!(request.get(member).is_none(), "{member}");
        }
        assert_eq!(
            request["probe"],
            json!({
                "connection": {"name": "lake-main", "revision": 3},
                "checks": ["CATALOG_AUTH", "WAREHOUSE", "STORAGE_ACCESS"],
                "checkTimeoutMs": 7000
            })
        );
        assert_eq!(
            request["connection"]["credentials"]["catalog"]["clientSecret"],
            SECRET
        );
        assert_eq!(
            request["relay"]["socket"],
            format!("/work/{ATTEMPT}/relay.sock")
        );
        // the token the host is given is the one in the file the relay checks
        let token = std::fs::read_to_string(work.path().join(ATTEMPT).join("relay.token")).unwrap();
        assert_eq!(request["relay"]["token"], token.trim_end());
        assert_eq!(prepared.staged_bytes, 0);
        assert!(
            entries(&work.path().join(ATTEMPT).join("in")).is_empty(),
            "nothing is staged for a probe"
        );
    }

    #[test]
    fn a_probe_does_not_need_the_connection_to_allow_a_write_mode() {
        // the modes a connection allows are the writes'; a probe of a connection that allows none is still a probe
        let work = tempfile::tempdir().unwrap();
        let mut connection = converted();
        connection.allowed_modes.clear();

        let prepared = prepare_probe(&probe(), connection, &settings(work.path()), attempt(0));

        assert!(prepared.is_ok(), "{prepared:?}");
    }

    #[test]
    fn a_probe_of_a_connection_the_relay_cannot_allow_is_refused_before_the_directory_is_made() {
        let work = tempfile::tempdir().unwrap();
        let mut connection = converted();
        connection.storage =
            json!({"type": "gcs", "gcs": {"project": "p"}, "credentials": "static"});

        let error =
            prepare_probe(&probe(), connection, &settings(work.path()), attempt(0)).unwrap_err();

        assert_eq!(error, PrepareError::Connection);
        assert_eq!(error.code(), DatabaseFailure::Configuration);
        assert!(entries(work.path()).is_empty());
    }

    #[test]
    fn a_probe_whose_directory_exists_is_not_prepared_and_the_existing_one_is_left_alone() {
        let work = tempfile::tempdir().unwrap();
        std::fs::create_dir(work.path().join(ATTEMPT)).unwrap();

        let error =
            prepare_probe(&probe(), converted(), &settings(work.path()), attempt(0)).unwrap_err();

        assert_eq!(error, PrepareError::Source);
        assert_eq!(entries(work.path()), [ATTEMPT]);
    }

    #[test]
    fn the_time_a_probe_may_take_is_three_checks_and_a_margin_up_to_the_step() {
        let seconds = |milliseconds| probe_timeout(milliseconds).as_secs_f64();
        // the lease gives 7 s to a check: three of them and the margin for the container and the relay fit the 25 s of the step
        assert!((seconds(7000) - 24.0).abs() < f64::EPSILON);
        assert!((seconds(100) - 3.3).abs() < 1e-9);
        // never more than the step is allowed (`timeoutMs` of the fixed probe document)
        assert!((seconds(60_000) - 25.0).abs() < f64::EPSILON);
        assert!((seconds(8000) - 25.0).abs() < f64::EPSILON);
    }
}
