//! The attempt directory and the record files staged into it (feature 035, T035-035, D035-112, D035-086).
//!
//! `contracts/lake-runtime.md`, "One attempt", steps 2 and 3: the runner makes `<work>/<attempt id>/` before any container
//! exists, writes the relay token into it, and streams the records the step names into `in/` while it counts them against
//! the effective limit. Going over fails before a container is started ([`StagingError::LimitExceeded`], which the driver
//! reports as `ICEBERG_LIMIT_EXCEEDED`): the cheapest place to fail, and the only one before the credentials leave the
//! runner. The container is then given this one directory and finds the files and nothing else.
//!
//! What the directory holds, with the modes of D035-086 (a plain user cannot give a file to the user of a slot, so the
//! group does the sharing and the modes are set explicitly, whatever the umask is):
//!
//! | Path | Mode | Content |
//! |---|---|---|
//! | `<attempt id>/` | 0770 | |
//! | `in/` | 0770 | the record files |
//! | `in/inline.ndjson` | 0640 | the inline records, one JSON value a line |
//! | `in/blob-<n>.<ndjson\|parquet>` | 0640 | a blob, byte for byte |
//! | `relay.token` | 0640 | the token and a newline |
//! | `relay.sock` | 0660 | made by the relay ([`AttemptDir::socket_path`]) |
//!
//! **What is counted.** The bytes of every file together, and the records of the inline source and of NDJSON blobs
//! (the lines). A Parquet file's rows are in its footer, which the runner does not parse: the host reads it under its own
//! decoded-bytes budget, so a Parquet blob counts toward the bytes and not toward the records (G035-058).
//!
//! **Nothing here shows the token or a path in an error**: [`StagingError`] is a closed kind. The directory is removed by
//! [`AttemptDir::remove`], and by `Drop` if the driver forgot, with the token in it.
//!
//! The file work is `std::fs`, in a blocking task for the streams, so the crate needs nothing beyond the features of
//! `tokio` it already uses.
//!
//! **What the caller owes.**
//! - A deadline. A source that stalls holds a blocking thread until the call ends, and nothing here times it out: the
//!   driver wraps `stage_*` in `tokio::time::timeout`. A call dropped midway leaves no half-made blob file and does not use
//!   up its name; the attempt is removed, not reused.
//! - The work volume. The attempt directory takes the group of the work root if that is setgid, and nothing here checks that
//!   it is the runner's: the volume's manifest says so.
//! - A look at what the container left. The directories are 0770 in the runner's group (D035-086), so a compromised host can
//!   create entries in them, among them a directory the runner cannot enter, which `remove` then cannot delete (O035-006,
//!   G035-060). The host itself writes nothing there: it reads `relay.token` and `in/*` and connects to `relay.sock`.

use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::io::{BufWriter, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use sagat_domain::database::MAX_RECORD_BLOBS;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio::sync::mpsc;

use crate::protocol::{InputFile, InputFormat, RelayToken};

/// The longest attempt id the host reads (`_ATTEMPT_ID` of `request.py` is a letter or digit, then up to 59 letters, digits
/// and hyphens) intersected with the lake profile's (lower-case hexadecimal digits and hyphens).
const MAX_ATTEMPT_ID: usize = 60;
const DIRECTORY_MODE: u32 = 0o770;
const FILE_MODE: u32 = 0o640;
/// How much of a blob is read at a time, and how many chunks wait for the writer.
const CHUNK: usize = 64 * 1024;
const QUEUED_CHUNKS: usize = 4;

/// Why staging stopped. A closed kind: no path, no token, no operating-system text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StagingError {
    #[error(
        "the attempt id is not 1 to 60 lower-case hexadecimal digits and hyphens that start with a hexadecimal digit"
    )]
    AttemptId,
    #[error("the attempt directory, or the inline source, is there already")]
    Exists,
    #[error("a file or a directory could not be made, written or removed")]
    Io,
    #[error("the records are over the effective limit")]
    LimitExceeded,
    #[error("a step names at most 32 record files")]
    TooManyFiles,
    #[error("a staging limit is zero, and zero is not unlimited")]
    ZeroLimit,
}

/// The effective limit of a write, as the staging counts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagingLimits {
    max_bytes: u64,
    max_records: u64,
}

impl StagingLimits {
    /// # Errors
    /// [`StagingError::ZeroLimit`] for a limit of zero.
    pub const fn new(max_bytes: u64, max_records: u64) -> Result<Self, StagingError> {
        if max_bytes == 0 || max_records == 0 {
            return Err(StagingError::ZeroLimit);
        }
        Ok(Self {
            max_bytes,
            max_records,
        })
    }
}

/// What one source added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Staged {
    pub bytes: u64,
    /// The records, or `None` for a Parquet file, whose rows the runner does not count.
    pub records: Option<u64>,
}

/// The directory of one attempt, with what was staged into it.
#[derive(Debug)]
pub struct AttemptDir {
    path: PathBuf,
    limits: StagingLimits,
    bytes: u64,
    records: u64,
    inputs: Vec<InputFile>,
    blobs: usize,
    inline: bool,
    removed: bool,
}

fn attempt_id_holds(id: &str) -> bool {
    let mut bytes = id.bytes();
    let digit_first = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    digit_first
        && id.len() <= MAX_ATTEMPT_ID
        && bytes.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) || byte == b'-')
}

fn make_file(path: &Path) -> Result<File, StagingError> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                StagingError::Exists
            } else {
                StagingError::Io
            }
        })?;
    // the umask can only take bits away; the mode of the decision is exact. On the open file, not on the path.
    file.set_permissions(Permissions::from_mode(FILE_MODE))
        .map_err(|_| StagingError::Io)?;
    Ok(file)
}

/// A directory made with its final mode, so that there is no instant at which it is wider than the decision (a `mkdir` and
/// a `chmod` after it leave one, and a process of another user could use it). The umask can only narrow what is asked, and
/// the `chmod` that follows restores the mode of the decision.
fn make_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.mode(DIRECTORY_MODE);
    builder.create(path)
}

/// A file that is removed if the call that made it is dropped before it ends (the caller's deadline), so that a stalled
/// source leaves nothing half written and does not use up the name.
struct Partial(Option<PathBuf>);

impl Partial {
    fn keep(mut self) {
        self.0 = None;
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

impl AttemptDir {
    /// Makes `<work_root>/<attempt_id>/` (which must not exist; `work_root` must), its `in/`, and `relay.token`.
    ///
    /// # Errors
    /// [`StagingError::AttemptId`] for an id that could name another path, [`StagingError::Exists`] for a directory that
    /// is there, [`StagingError::Io`] for anything else. Nothing is left behind on an error.
    pub fn create(
        work_root: &Path,
        attempt_id: &str,
        token: &RelayToken,
        limits: StagingLimits,
    ) -> Result<Self, StagingError> {
        if !attempt_id_holds(attempt_id) {
            return Err(StagingError::AttemptId);
        }
        let path = work_root.join(attempt_id);
        make_directory(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                StagingError::Exists
            } else {
                StagingError::Io
            }
        })?;
        // from here the directory is ours and `Drop` removes it if anything below fails
        let attempt = Self {
            path,
            limits,
            bytes: 0,
            records: 0,
            inputs: Vec::new(),
            blobs: 0,
            inline: false,
            removed: false,
        };
        attempt.prepare(token)?;
        Ok(attempt)
    }

    fn prepare(&self, token: &RelayToken) -> Result<(), StagingError> {
        fs::set_permissions(&self.path, Permissions::from_mode(DIRECTORY_MODE))
            .map_err(|_| StagingError::Io)?;
        let records = self.path.join("in");
        make_directory(&records).map_err(|_| StagingError::Io)?;
        fs::set_permissions(&records, Permissions::from_mode(DIRECTORY_MODE))
            .map_err(|_| StagingError::Io)?;
        let mut file = make_file(&self.path.join("relay.token"))?;
        file.write_all(token.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.flush())
            .map_err(|_| StagingError::Io)
    }

    /// The attempt's directory: what the container is given.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where the relay binds its socket. The relay makes it, and the driver gives it mode 0660.
    #[must_use]
    pub fn socket_path(&self) -> PathBuf {
        self.path.join("relay.sock")
    }

    /// The files staged so far, in the order they were staged, as the request names them.
    #[must_use]
    pub fn inputs(&self) -> Vec<InputFile> {
        self.inputs.clone()
    }

    fn room_for_a_file(&self) -> Result<(), StagingError> {
        if self.inputs.len() >= MAX_RECORD_BLOBS {
            return Err(StagingError::TooManyFiles);
        }
        Ok(())
    }

    fn input(&mut self, name: &str, format: InputFormat) -> Result<(), StagingError> {
        let input = InputFile::new(name, format).map_err(|_| StagingError::Io)?;
        self.inputs.push(input);
        Ok(())
    }

    /// Stages the inline records as `in/inline.ndjson`, one compact JSON value a line (a newline inside a value is
    /// escaped, so a record is a line). An empty source is an empty file: the host then reports an empty write.
    ///
    /// # Errors
    /// [`StagingError::Exists`] when the inline source was staged already, [`StagingError::LimitExceeded`] at the first
    /// record that goes over the limit (nothing is left of the file), [`StagingError::TooManyFiles`], [`StagingError::Io`].
    pub async fn stage_inline<I>(&mut self, records: I) -> Result<Staged, StagingError>
    where
        I: IntoIterator<Item = Value> + Send + 'static,
        I::IntoIter: Send,
    {
        if self.inline {
            return Err(StagingError::Exists);
        }
        self.room_for_a_file()?;
        let path = self.path.join("in/inline.ndjson");
        let room_bytes = self.limits.max_bytes - self.bytes;
        let room_records = self.limits.max_records - self.records;
        let target = path.clone();
        let written = tokio::task::spawn_blocking(move || {
            let file = make_file(&target)?;
            let mut writer = BufWriter::new(file);
            let (mut bytes, mut count) = (0_u64, 0_u64);
            for record in records {
                let mut line = serde_json::to_vec(&record).map_err(|_| StagingError::Io)?;
                line.push(b'\n');
                bytes += line.len() as u64;
                count += 1;
                if bytes > room_bytes || count > room_records {
                    return Err(StagingError::LimitExceeded);
                }
                writer.write_all(&line).map_err(|_| StagingError::Io)?;
            }
            writer.flush().map_err(|_| StagingError::Io)?;
            Ok(Staged {
                bytes,
                records: Some(count),
            })
        })
        .await
        .map_err(|_| StagingError::Io)
        .and_then(|outcome| outcome);
        match written {
            Ok(staged) => {
                self.bytes = self.bytes.saturating_add(staged.bytes);
                self.records = self.records.saturating_add(staged.records.unwrap_or(0));
                self.inline = true;
                self.input("inline.ndjson", InputFormat::Ndjson)?;
                Ok(staged)
            }
            Err(error) => {
                if error != StagingError::Exists {
                    let _ = fs::remove_file(&path);
                }
                Err(error)
            }
        }
    }

    /// Stages a blob, byte for byte, as `in/blob-<n>.<ndjson|parquet>`. The source is read in chunks and stopped at the
    /// limit: a blob that never ends costs one chunk past the limit and no more. NDJSON lines are counted as records; a
    /// Parquet file is not parsed.
    ///
    /// # Errors
    /// [`StagingError::LimitExceeded`] when the bytes or the records go over (nothing is left of the file),
    /// [`StagingError::TooManyFiles`], [`StagingError::Io`] when the source fails or the file cannot be written.
    pub async fn stage_blob<R>(
        &mut self,
        format: InputFormat,
        mut source: R,
    ) -> Result<Staged, StagingError>
    where
        R: AsyncRead + Unpin,
    {
        self.room_for_a_file()?;
        let extension = match format {
            InputFormat::Ndjson => "ndjson",
            InputFormat::Parquet => "parquet",
        };
        let name = format!("blob-{}.{extension}", self.blobs);
        let path = self.path.join("in").join(&name);
        let file = make_file(&path)?;
        let partial = Partial(Some(path));

        let (chunks, queue) = mpsc::channel::<Vec<u8>>(QUEUED_CHUNKS);
        let writer = tokio::task::spawn_blocking(move || {
            let mut writer = BufWriter::new(file);
            let mut queue = queue;
            while let Some(chunk) = queue.blocking_recv() {
                writer.write_all(&chunk).map_err(|_| StagingError::Io)?;
            }
            writer.flush().map_err(|_| StagingError::Io)
        });

        let copied = self.copy(format, &mut source, &chunks).await;
        drop(chunks);
        let written = writer
            .await
            .map_err(|_| StagingError::Io)
            .and_then(|outcome| outcome);
        // on an error `partial` removes the file as it goes out of scope, as it does when this call is dropped midway
        let staged = copied.and_then(|staged| written.map(|()| staged))?;
        self.bytes = self.bytes.saturating_add(staged.bytes);
        self.records = self.records.saturating_add(staged.records.unwrap_or(0));
        self.blobs += 1;
        self.input(&name, format)?;
        partial.keep();
        Ok(staged)
    }

    async fn copy<R: AsyncRead + Unpin>(
        &self,
        format: InputFormat,
        source: &mut R,
        chunks: &mpsc::Sender<Vec<u8>>,
    ) -> Result<Staged, StagingError> {
        let (mut bytes, mut lines) = (0_u64, 0_u64);
        let mut last = b'\n';
        let mut buffer = vec![0_u8; CHUNK];
        loop {
            let read = source
                .read(&mut buffer)
                .await
                .map_err(|_| StagingError::Io)?;
            if read == 0 {
                break;
            }
            let chunk = &buffer[..read];
            bytes += read as u64;
            if self.bytes.saturating_add(bytes) > self.limits.max_bytes {
                return Err(StagingError::LimitExceeded);
            }
            if format == InputFormat::Ndjson {
                lines += chunk.iter().filter(|byte| **byte == b'\n').count() as u64;
                last = chunk[read - 1];
                if self.records.saturating_add(lines) > self.limits.max_records {
                    return Err(StagingError::LimitExceeded);
                }
            }
            chunks
                .send(chunk.to_vec())
                .await
                .map_err(|_| StagingError::Io)?;
        }
        let records = match format {
            InputFormat::Ndjson => {
                // a last line without its newline is a record too
                if bytes > 0 && last != b'\n' {
                    lines += 1;
                }
                if self.records.saturating_add(lines) > self.limits.max_records {
                    return Err(StagingError::LimitExceeded);
                }
                Some(lines)
            }
            InputFormat::Parquet => None,
        };
        Ok(Staged { bytes, records })
    }

    /// Removes the directory and everything in it, the token included.
    ///
    /// The container had this directory read-write in the runner's group, so a compromised host can leave in it a directory
    /// the runner cannot enter (the slot's user owns it, mode 0700) and the removal then fails. It fails **without
    /// letting go of the attempt**: the handle stays, so the driver can retry, report the leftover and count it, and `Drop`
    /// tries once more. What the runner cannot enter, it cannot remove; that is O035-006 and G035-060.
    ///
    /// # Errors
    /// [`StagingError::Io`] when something could not be removed; a directory that is gone already is not an error.
    pub fn remove(&mut self) -> Result<(), StagingError> {
        match fs::remove_dir_all(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(StagingError::Io),
        }
        self.removed = true;
        Ok(())
    }
}

impl Drop for AttemptDir {
    fn drop(&mut self) {
        if !self.removed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
