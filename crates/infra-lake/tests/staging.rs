//! The attempt directory and the record files staged into it (feature 035, T035-035, D035-112, D035-086,
//! `contracts/lake-runtime.md` "One attempt" steps 2 and 3).
//!
//! The runner makes `<work>/<attempt id>/` before any container exists, writes the relay token into it, and streams the
//! records the step names into `in/` while it counts them against the effective limit: going over fails before a
//! container is started, which is the cheapest place to fail and the only one before the credentials leave the runner.
//! The container finds the files and nothing else.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use sagat_infra_lake::protocol::{InputFormat, RelayToken};
use sagat_infra_lake::staging::{AttemptDir, StagingError, StagingLimits};
use serde_json::{Value, json};

const TOKEN: &str = "relay-token-canary-0123456789abcdef";
const ATTEMPT: &str = "0190a5e2-7b3c-7d2e-9f10-aabbccddeeff";

fn token() -> RelayToken {
    RelayToken::new(TOKEN).unwrap()
}

fn limits(bytes: u64, records: u64) -> StagingLimits {
    StagingLimits::new(bytes, records).unwrap()
}

fn roomy() -> StagingLimits {
    limits(1 << 30, 1 << 30)
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

fn made(work: &Path, limits: StagingLimits) -> AttemptDir {
    AttemptDir::create(work, ATTEMPT, &token(), limits).unwrap()
}

// -- the directory and the token --------------------------------------------------------------------------------------

#[test]
fn the_attempt_directory_has_the_modes_of_the_decision_and_the_token_a_newline() {
    let work = tempfile::tempdir().unwrap();
    let attempt = made(work.path(), roomy());

    let dir = work.path().join(ATTEMPT);
    assert_eq!(attempt.path(), dir);
    assert_eq!(mode(&dir), 0o770);
    assert_eq!(mode(&dir.join("in")), 0o770);
    assert_eq!(mode(&dir.join("relay.token")), 0o640);
    // what `read_token` of the host reads: the token and a newline, nothing else
    assert_eq!(
        std::fs::read_to_string(dir.join("relay.token")).unwrap(),
        format!("{TOKEN}\n")
    );
    assert_eq!(attempt.socket_path(), dir.join("relay.sock"));
}

#[test]
fn an_attempt_directory_is_made_once() {
    let work = tempfile::tempdir().unwrap();
    let _first = made(work.path(), roomy());
    assert_eq!(
        AttemptDir::create(work.path(), ATTEMPT, &token(), roomy()).unwrap_err(),
        StagingError::Exists
    );
}

#[test]
fn an_attempt_id_that_could_leave_the_work_directory_is_refused() {
    let work = tempfile::tempdir().unwrap();
    let long = "a".repeat(61);
    for id in [
        "",
        ".",
        "..",
        "../x",
        "a/b",
        "/abs",
        "-leading-hyphen",
        "UPPER",
        "0190a5e2-7b3c\n",
        "0190a5e2 7b3c",
        "g123",
        "é",
        &long,
    ] {
        assert_eq!(
            AttemptDir::create(work.path(), id, &token(), roomy()).unwrap_err(),
            StagingError::AttemptId,
            "{id:?}"
        );
    }
    let longest = "a".repeat(60);
    for id in [ATTEMPT, "0", "ab-cd", &longest] {
        let work = tempfile::tempdir().unwrap();
        assert!(
            AttemptDir::create(work.path(), id, &token(), roomy()).is_ok(),
            "{id:?}"
        );
    }
    // nothing was made for a refused id
    assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 0);
}

#[test]
fn a_work_directory_that_is_not_there_is_an_io_error_and_not_a_new_directory() {
    let work = tempfile::tempdir().unwrap();
    let missing = work.path().join("not-made");
    assert_eq!(
        AttemptDir::create(&missing, ATTEMPT, &token(), roomy()).unwrap_err(),
        StagingError::Io
    );
    assert!(!missing.exists());
}

#[test]
fn no_error_shows_the_token_or_a_path() {
    let work = tempfile::tempdir().unwrap();
    let errors = [
        AttemptDir::create(&work.path().join("missing"), ATTEMPT, &token(), roomy()).unwrap_err(),
        AttemptDir::create(work.path(), "../x", &token(), roomy()).unwrap_err(),
        StagingError::LimitExceeded,
        StagingError::TooManyFiles,
    ];
    for error in errors {
        let shown = format!("{error:?} {error}");
        assert!(!shown.contains(TOKEN), "{shown}");
        assert!(!shown.contains(work.path().to_str().unwrap()), "{shown}");
    }
}

#[test]
fn a_limit_of_zero_is_not_unlimited() {
    assert_eq!(
        StagingLimits::new(0, 10).unwrap_err(),
        StagingError::ZeroLimit
    );
    assert_eq!(
        StagingLimits::new(10, 0).unwrap_err(),
        StagingError::ZeroLimit
    );
    assert!(StagingLimits::new(1, 1).is_ok());
}

// -- inline records ---------------------------------------------------------------------------------------------------

fn read(attempt: &AttemptDir, name: &str) -> Vec<u8> {
    std::fs::read(attempt.path().join("in").join(name)).unwrap()
}

#[tokio::test]
async fn inline_records_are_one_json_object_a_line_in_inline_ndjson() {
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), roomy());

    let staged = attempt
        .stage_inline([
            json!({"id": 1, "name": "Ana"}),
            json!({"id": 2, "note": "line\nbreak and \u{e9}"}),
        ])
        .await
        .unwrap();

    assert_eq!(staged.records, Some(2));
    let text = String::from_utf8(read(&attempt, "inline.ndjson")).unwrap();
    // a newline inside a value is escaped, so a record is one line
    assert_eq!(text.lines().count(), 2);
    assert!(text.ends_with('\n'));
    let parsed: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(parsed[1]["note"], json!("line\nbreak and \u{e9}"));
    assert_eq!(mode(&attempt.path().join("in/inline.ndjson")), 0o640);
}

#[tokio::test]
async fn no_inline_record_is_an_empty_file_and_not_a_missing_one() {
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), roomy());
    let staged = attempt.stage_inline(Vec::<Value>::new()).await.unwrap();
    assert_eq!(staged.records, Some(0));
    assert_eq!(read(&attempt, "inline.ndjson"), b"");
}

#[tokio::test]
async fn the_inline_source_is_staged_once() {
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), roomy());
    attempt.stage_inline([json!({"a": 1})]).await.unwrap();
    assert_eq!(
        attempt.stage_inline([json!({"a": 2})]).await.unwrap_err(),
        StagingError::Exists
    );
}

#[tokio::test]
async fn the_limit_on_bytes_is_exact_for_inline_records() {
    // {"a":1}\n is nine bytes
    let line = json!({"a": 1});
    let size = serde_json::to_vec(&line).unwrap().len() as u64 + 1;
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), limits(2 * size, 100));
    attempt
        .stage_inline([line.clone(), line.clone()])
        .await
        .unwrap();

    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), limits(2 * size - 1, 100));
    assert_eq!(
        attempt
            .stage_inline([line.clone(), line.clone()])
            .await
            .unwrap_err(),
        StagingError::LimitExceeded
    );
}

#[tokio::test]
async fn the_limit_on_records_is_exact_for_inline_records() {
    let record = json!({"a": 1});
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), limits(1 << 20, 3));
    attempt.stage_inline(vec![record.clone(); 3]).await.unwrap();

    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), limits(1 << 20, 3));
    assert_eq!(
        attempt.stage_inline(vec![record; 4]).await.unwrap_err(),
        StagingError::LimitExceeded
    );
}

#[tokio::test]
async fn a_record_over_the_limit_leaves_no_partial_file_behind() {
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), limits(16, 100));
    assert_eq!(
        attempt
            .stage_inline([json!({"a": "a value much longer than sixteen bytes"})])
            .await
            .unwrap_err(),
        StagingError::LimitExceeded
    );
    assert!(!attempt.path().join("in/inline.ndjson").exists());
}

// -- blobs ------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_blob_is_copied_byte_for_byte_under_a_name_the_host_takes() {
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), roomy());
    let ndjson = b"{\"a\":1}\n{\"a\":2}\n".to_vec();
    let parquet = b"PAR1 any bytes at all \x00\xff PAR1".to_vec();

    let first = attempt
        .stage_blob(InputFormat::Ndjson, &ndjson[..])
        .await
        .unwrap();
    let second = attempt
        .stage_blob(InputFormat::Parquet, &parquet[..])
        .await
        .unwrap();

    assert_eq!(read(&attempt, "blob-0.ndjson"), ndjson);
    assert_eq!(read(&attempt, "blob-1.parquet"), parquet);
    assert_eq!(first.records, Some(2));
    // a Parquet file's rows are in its footer, which the host reads under its own budget (G035-058)
    assert_eq!(second.records, None);
    assert_eq!(first.bytes, ndjson.len() as u64);
    assert_eq!(mode(&attempt.path().join("in/blob-0.ndjson")), 0o640);
}

#[tokio::test]
async fn the_inputs_are_what_the_request_names_in_the_order_they_were_staged() {
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), roomy());
    attempt.stage_inline([json!({"a": 1})]).await.unwrap();
    attempt
        .stage_blob(InputFormat::Parquet, &b"x"[..])
        .await
        .unwrap();
    attempt
        .stage_blob(InputFormat::Ndjson, &b"{}\n"[..])
        .await
        .unwrap();

    let names: Vec<_> = attempt
        .inputs()
        .iter()
        .map(|input| input.name().to_owned())
        .collect();
    assert_eq!(names, ["inline.ndjson", "blob-0.parquet", "blob-1.ndjson"]);
    assert_eq!(attempt.inputs().len(), 3);
}

#[tokio::test]
async fn ndjson_records_are_counted_with_or_without_a_last_newline() {
    for (text, records) in [
        (&b""[..], 0),
        (b"{}\n", 1),
        (b"{}", 1),
        (b"{}\n{}", 2),
        (b"{}\n{}\n", 2),
        (b"{}\n\n{}\n", 3),
    ] {
        let work = tempfile::tempdir().unwrap();
        let mut attempt = made(work.path(), roomy());
        let staged = attempt.stage_blob(InputFormat::Ndjson, text).await.unwrap();
        assert_eq!(staged.records, Some(records), "{text:?}");
    }
}

#[tokio::test]
async fn the_limits_count_the_inline_records_and_every_blob_together() {
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), limits(1 << 20, 4));
    attempt
        .stage_inline([json!({"a": 1}), json!({"a": 2})])
        .await
        .unwrap();
    attempt
        .stage_blob(InputFormat::Ndjson, &b"{}\n{}\n"[..])
        .await
        .unwrap();
    // the fifth record is over the four
    assert_eq!(
        attempt
            .stage_blob(InputFormat::Ndjson, &b"{}\n"[..])
            .await
            .unwrap_err(),
        StagingError::LimitExceeded
    );
}

#[tokio::test]
async fn a_blob_that_never_ends_is_stopped_at_the_limit_and_not_read_to_its_end() {
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), limits(64 * 1024, 1 << 30));
    // an endless stream: only a bound on the reading ends the call
    let endless = tokio::io::repeat(b'x');
    assert_eq!(
        attempt
            .stage_blob(InputFormat::Parquet, endless)
            .await
            .unwrap_err(),
        StagingError::LimitExceeded
    );
}

#[tokio::test]
async fn the_limit_on_bytes_is_exact_for_a_blob_and_a_refused_blob_leaves_no_file() {
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), limits(100, 1 << 30));
    attempt
        .stage_blob(InputFormat::Parquet, &[7_u8; 100][..])
        .await
        .unwrap();

    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), limits(100, 1 << 30));
    assert_eq!(
        attempt
            .stage_blob(InputFormat::Parquet, &[7_u8; 101][..])
            .await
            .unwrap_err(),
        StagingError::LimitExceeded
    );
    assert!(!attempt.path().join("in/blob-0.parquet").exists());
    assert!(attempt.inputs().is_empty());
}

#[tokio::test]
async fn a_blob_whose_stream_fails_is_an_io_error_and_leaves_no_file() {
    struct Broken(bool);
    impl tokio::io::AsyncRead for Broken {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buffer: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            if self.0 {
                return std::task::Poll::Ready(Err(std::io::Error::other(
                    "the blob store went away",
                )));
            }
            self.0 = true;
            buffer.put_slice(b"first bytes");
            std::task::Poll::Ready(Ok(()))
        }
    }
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), roomy());
    assert_eq!(
        attempt
            .stage_blob(InputFormat::Parquet, Broken(false))
            .await
            .unwrap_err(),
        StagingError::Io
    );
    assert!(!attempt.path().join("in/blob-0.parquet").exists());
    assert!(attempt.inputs().is_empty());
}

#[tokio::test]
async fn a_step_names_at_most_thirty_two_files() {
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), roomy());
    for _ in 0..32 {
        attempt
            .stage_blob(InputFormat::Ndjson, &b"{}\n"[..])
            .await
            .unwrap();
    }
    assert_eq!(
        attempt
            .stage_blob(InputFormat::Ndjson, &b"{}\n"[..])
            .await
            .unwrap_err(),
        StagingError::TooManyFiles
    );
    assert_eq!(attempt.inputs().len(), 32);
}

#[tokio::test]
async fn staged_inputs_feed_the_request_builder() {
    use sagat_domain::database::TableIdentifier;
    use sagat_infra_lake::protocol::{Connection, Limits, WriteAppend};

    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), roomy());
    attempt.stage_inline([json!({"a": 1})]).await.unwrap();
    attempt
        .stage_blob(InputFormat::Parquet, &b"x"[..])
        .await
        .unwrap();

    let connection = Connection::new(
        json!({"type": "rest"}),
        json!({"type": "s3"}),
        vec!["s3://lake/".to_owned()],
        Default::default(),
        Default::default(),
    );
    let body = WriteAppend::new(
        TableIdentifier::new(vec!["sales".to_owned()], "orders").unwrap(),
        "main",
        connection,
        attempt.inputs(),
        Limits::new(4, 1 << 20, 1 << 20, "2026-11-05T10:10:00.000Z").unwrap(),
    )
    .into_body()
    .unwrap();
    assert!(body.contains_key("inputs"));
}

// -- the end of an attempt --------------------------------------------------------------------------------------------

#[tokio::test]
async fn removing_the_attempt_removes_the_directory_with_everything_in_it() {
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), roomy());
    attempt.stage_inline([json!({"a": 1})]).await.unwrap();
    let dir = attempt.path().to_path_buf();
    attempt.remove().unwrap();
    assert!(!dir.exists());
    assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 0);
}

/// The container is given the directory read-write in the runner's group, so a compromised host can leave a directory the
/// runner cannot enter (owned by the slot's user, mode 0700). The removal then fails, and the attempt must still be there
/// to retry and to report: a removal that consumed the handle would leave the tree and the token with nobody to say so.
/// (Skipped when the test runs as root, which can enter any directory.)
#[tokio::test]
async fn a_removal_that_fails_leaves_the_attempt_in_hand_for_a_retry() {
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), roomy());
    let planted = attempt.path().join("in/planted");
    std::fs::create_dir(&planted).unwrap();
    std::fs::write(planted.join("file"), b"x").unwrap();
    std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&planted).is_ok() {
        std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o700)).unwrap();
        return;
    }

    assert_eq!(attempt.remove().unwrap_err(), StagingError::Io);
    assert!(attempt.path().exists());

    // whoever owns the directory lets go of it, and the same handle removes the rest
    std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o700)).unwrap();
    let dir = attempt.path().to_path_buf();
    attempt.remove().unwrap();
    assert!(!dir.exists());
}

/// A call that is dropped before it ends (the caller's deadline) leaves nothing half made and does not use up the name:
/// the source stalled, the time ran out, and the attempt goes on or is removed. The call is polled once, up to the stalled
/// read, and then dropped by a branch that is ready at once: no timer, because a paused clock does not advance while a
/// blocking task (the writer) is alive.
#[tokio::test]
async fn a_stage_that_is_dropped_midway_leaves_no_file_and_the_name_can_be_used_again() {
    struct Stalled;
    impl tokio::io::AsyncRead for Stalled {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Pending
        }
    }
    let work = tempfile::tempdir().unwrap();
    let mut attempt = made(work.path(), roomy());

    let blob = attempt.path().join("in/blob-0.ndjson");
    tokio::select! {
        biased;
        _ = attempt.stage_blob(InputFormat::Ndjson, Stalled) => panic!("the stalled source ended by itself"),
        () = async {} => {}
    }
    // the file was made when the call was polled, and is gone now that it was dropped
    assert!(!blob.exists());
    assert!(attempt.inputs().is_empty());

    attempt
        .stage_blob(InputFormat::Ndjson, &b"{}\n"[..])
        .await
        .unwrap();
    assert_eq!(read(&attempt, "blob-0.ndjson"), b"{}\n");
}

#[test]
fn dropping_the_attempt_removes_the_directory_too() {
    let work = tempfile::tempdir().unwrap();
    let dir = {
        let attempt = made(work.path(), roomy());
        attempt.path().to_path_buf()
    };
    assert!(!dir.exists());
}

#[tokio::test]
async fn the_token_is_not_left_in_a_file_that_survives_the_attempt() {
    let work = tempfile::tempdir().unwrap();
    let attempt = made(work.path(), roomy());
    let token_file = attempt.path().join("relay.token");
    assert!(token_file.exists());
    drop(attempt);
    assert!(!token_file.exists());
}
