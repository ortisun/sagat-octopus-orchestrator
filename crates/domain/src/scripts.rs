//! Scripts: code written on the platform, built into an immutable artifact and run in a sandbox
//! (`FR-005-001..008`).
//!
//! A **draft** is what an author edits. Publishing it makes a **version**: the source and the
//! lockfile are built, in isolation, into an **artifact** identified by its digest, and from then on
//! nothing about the version changes. A workflow names the digest, so a run is pinned to exactly the
//! code that was built. The same four things always give the same build key (source, lockfile,
//! toolchain, policy), which is what lets a build be reused only when nothing that matters changed.
//!
//! What is allowed to happen when the code runs (how much memory, how long, whether it may reach
//! the network at all) is decided here, as values, so it can be tested without a container.

use std::fmt;
use std::time::Duration;

uuid_id!(
    /// Identifier of a script (its drafts and versions).
    ScriptId
);
uuid_id!(
    /// Identifier of one published version of a script.
    ScriptVersionId
);
uuid_id!(
    /// Identifier of a build.
    BuildJobId
);
uuid_id!(
    /// Identifier of a test run: running a draft in the sandbox without publishing it.
    TestRunId
);

/// Why a value about a script was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    #[error("the source is empty, over 256 KiB or contains a NUL character")]
    InvalidSource,
    #[error("the lockfile is over 256 KiB or contains a NUL character")]
    InvalidLockfile,
    #[error("a digest is `sha256:` and 64 lower-case hexadecimal digits")]
    InvalidDigest,
    #[error("the language is not one of typescript, python, csharp, go or java")]
    UnknownLanguage,
    #[error("the memory limit is between 16 MiB and 2 GiB")]
    InvalidMemory,
    #[error("the CPU limit is between 100 and 4000 millicores")]
    InvalidCpu,
    #[error("the time limit is between 1 second and 15 minutes")]
    InvalidWallTime,
    #[error("the process limit is between 8 and 512")]
    InvalidProcesses,
    #[error("the output limit is between 1 KiB and 1 MiB")]
    InvalidOutputLimit,
    #[error("the scratch space limit is between 1 MiB and 512 MiB")]
    InvalidScratch,
    #[error("the version is not a build the platform made, or its signature does not verify")]
    NotATrustedArtifact,
}

/// Longest source accepted, in bytes.
pub const MAX_SOURCE_BYTES: usize = 256 * 1024;
/// Longest lockfile accepted, in bytes.
pub const MAX_LOCKFILE_BYTES: usize = 256 * 1024;

/// The language of a script. All five are named by the workflow schema; the ones that can be built
/// and run today are those for which [`Self::is_built`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Language {
    TypeScript,
    Python,
    CSharp,
    Go,
    Java,
}

impl Language {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Python => "python",
            Self::CSharp => "csharp",
            Self::Go => "go",
            Self::Java => "java",
        }
    }

    /// Whether the platform has a builder and a runner for it.
    #[must_use]
    pub const fn is_built(self) -> bool {
        matches!(self, Self::TypeScript | Self::Python)
    }

    /// The extension of the file the source is written to.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::TypeScript => "ts",
            Self::Python => "py",
            Self::CSharp => "cs",
            Self::Go => "go",
            Self::Java => "java",
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Language {
    type Err = ScriptError;

    fn from_str(text: &str) -> Result<Self, ScriptError> {
        Ok(match text {
            "typescript" => Self::TypeScript,
            "python" => Self::Python,
            "csharp" => Self::CSharp,
            "go" => Self::Go,
            "java" => Self::Java,
            _ => return Err(ScriptError::UnknownLanguage),
        })
    }
}

fn clean_text(value: &str, max: usize) -> bool {
    value.len() <= max && !value.contains('\0')
}

/// The text of a script, as the author wrote it.
#[derive(Clone, PartialEq, Eq)]
pub struct Source(String);

impl Source {
    /// # Errors
    /// [`ScriptError::InvalidSource`].
    pub fn parse(value: &str) -> Result<Self, ScriptError> {
        if value.trim().is_empty() || !clean_text(value, MAX_SOURCE_BYTES) {
            return Err(ScriptError::InvalidSource);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// A source is the author's code and may contain what they pasted into it: it is never printed.
impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Source({} bytes)", self.0.len())
    }
}

/// The text of a lockfile (`package-lock.json`, `requirements.txt` with hashes, `packages.lock.json`).
#[derive(Clone, PartialEq, Eq)]
pub struct Lockfile(String);

impl Lockfile {
    /// # Errors
    /// [`ScriptError::InvalidLockfile`].
    pub fn parse(value: &str) -> Result<Self, ScriptError> {
        if !clean_text(value, MAX_LOCKFILE_BYTES) {
            return Err(ScriptError::InvalidLockfile);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Lockfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Lockfile({} bytes)", self.0.len())
    }
}

/// A SHA-256 digest, written `sha256:` and 64 lower-case hexadecimal digits.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest(String);

impl Digest {
    /// Longest an abbreviation of a digest may be and still be recognised in a log or a message.
    pub const PREFIX: &'static str = "sha256:";

    /// # Errors
    /// [`ScriptError::InvalidDigest`].
    pub fn parse(value: &str) -> Result<Self, ScriptError> {
        let hex = value
            .strip_prefix(Self::PREFIX)
            .ok_or(ScriptError::InvalidDigest)?;
        if hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Self(value.to_owned()))
        } else {
            Err(ScriptError::InvalidDigest)
        }
    }

    /// The digest whose 32 bytes are `bytes`.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        use std::fmt::Write as _;
        let mut text = String::with_capacity(71);
        text.push_str(Self::PREFIX);
        for byte in bytes {
            let _ = write!(text, "{byte:02x}");
        }
        Self(text)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The 32 bytes.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 32] {
        let hex = &self.0[Self::PREFIX.len()..];
        let mut bytes = [0u8; 32];
        for (i, slot) in bytes.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap_or(0);
        }
        bytes
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a script may reach the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Egress {
    /// Not at all: the sandbox has no network interface other than the loopback.
    None,
    /// Only through the platform's guarded client, under the egress policy of the namespace: the
    /// script asks the supervisor to make a request, and the supervisor decides.
    Namespace,
}

impl Egress {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Namespace => "namespace",
        }
    }
}

/// What one run of a script may use. Every limit has a floor and a ceiling; a tenant may ask for
/// less than the ceiling and never for more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    memory_bytes: u64,
    cpu_millis: u32,
    wall: Duration,
    processes: u32,
    output_bytes: usize,
    scratch_bytes: u64,
}

const MIB: u64 = 1024 * 1024;

impl Limits {
    pub const MIN_MEMORY: u64 = 16 * MIB;
    pub const MAX_MEMORY: u64 = 2048 * MIB;
    pub const MAX_WALL: Duration = Duration::from_secs(15 * 60);

    /// # Errors
    /// The [`ScriptError`] of the first limit out of bounds.
    pub fn new(
        memory_bytes: u64,
        cpu_millis: u32,
        wall: Duration,
        processes: u32,
        output_bytes: usize,
        scratch_bytes: u64,
    ) -> Result<Self, ScriptError> {
        if !(Self::MIN_MEMORY..=Self::MAX_MEMORY).contains(&memory_bytes) {
            return Err(ScriptError::InvalidMemory);
        }
        if !(100..=4000).contains(&cpu_millis) {
            return Err(ScriptError::InvalidCpu);
        }
        if !(Duration::from_secs(1)..=Self::MAX_WALL).contains(&wall) {
            return Err(ScriptError::InvalidWallTime);
        }
        if !(8..=512).contains(&processes) {
            return Err(ScriptError::InvalidProcesses);
        }
        if !(1024..=1024 * 1024).contains(&output_bytes) {
            return Err(ScriptError::InvalidOutputLimit);
        }
        if !(MIB..=512 * MIB).contains(&scratch_bytes) {
            return Err(ScriptError::InvalidScratch);
        }
        Ok(Self {
            memory_bytes,
            cpu_millis,
            wall,
            processes,
            output_bytes,
            scratch_bytes,
        })
    }

    #[must_use]
    pub const fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }

    #[must_use]
    pub const fn cpu_millis(&self) -> u32 {
        self.cpu_millis
    }

    #[must_use]
    pub const fn wall(&self) -> Duration {
        self.wall
    }

    #[must_use]
    pub const fn processes(&self) -> u32 {
        self.processes
    }

    #[must_use]
    pub const fn output_bytes(&self) -> usize {
        self.output_bytes
    }

    #[must_use]
    pub const fn scratch_bytes(&self) -> u64 {
        self.scratch_bytes
    }

    /// The CPU time after which the kernel ends the process whatever the wall clock says: the
    /// wall limit times the share of a core it is given, rounded up, at least a second.
    #[must_use]
    pub fn cpu_seconds(&self) -> u64 {
        let share = f64::from(self.cpu_millis) / 1000.0;
        ((self.wall.as_secs_f64() * share).ceil() as u64).max(1)
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: 256 * MIB,
            cpu_millis: 1000,
            wall: Duration::from_secs(30),
            processes: 64,
            output_bytes: 64 * 1024,
            scratch_bytes: 32 * MIB,
        }
    }
}

/// The rules a build or a run is held to. Part of the build key, so a build is reused only while
/// the rules are the ones it was made under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub limits: Limits,
    pub egress: Egress,
}

impl Policy {
    /// The policy as text: what is hashed into the build key. Stable across releases of the code
    /// (changing it changes every key, which is what a change of policy must do).
    #[must_use]
    pub fn canonical(&self) -> String {
        format!(
            "v1;memory={};cpu={};wall={};pids={};output={};scratch={};egress={}",
            self.limits.memory_bytes,
            self.limits.cpu_millis,
            self.limits.wall.as_secs(),
            self.limits.processes,
            self.limits.output_bytes,
            self.limits.scratch_bytes,
            self.egress.as_str()
        )
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            egress: Egress::None,
        }
    }
}

/// Where a build or a test run is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

impl JobStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Running => "RUNNING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
        }
    }

    #[must_use]
    pub const fn is_final(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }

    /// Whether a job may go from `self` to `next`: forward only, and a job whose lease ran out may
    /// go back to the queue to be taken again.
    #[must_use]
    pub const fn can_become(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Queued, Self::Running)
                | (Self::Running, Self::Queued | Self::Succeeded | Self::Failed)
                | (Self::Queued, Self::Failed)
        )
    }
}

impl std::str::FromStr for JobStatus {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, ()> {
        Ok(match text {
            "QUEUED" => Self::Queued,
            "RUNNING" => Self::Running,
            "SUCCEEDED" => Self::Succeeded,
            "FAILED" => Self::Failed,
            _ => return Err(()),
        })
    }
}

/// Why a run of a script did not give a result. The code is what a step is failed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// The script raised an error or exited with a status other than zero.
    Error,
    /// It was still running when its time ran out.
    Timeout,
    /// It used more memory than it was given.
    OutOfMemory,
    /// It wrote more output than it may.
    OutputLimit,
    /// It tried something it is not allowed to do (a request the egress policy refuses, a secret
    /// it did not declare).
    Denied,
    /// Its input or its result does not match the schema it declared.
    SchemaViolation,
    /// It never produced a result (it exited without one).
    NoResult,
    /// The sandbox itself could not run it (not the script's fault).
    Unavailable,
}

impl Failure {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Error => "SCRIPT_ERROR",
            Self::Timeout => "SCRIPT_TIMEOUT",
            Self::OutOfMemory => "SCRIPT_OUT_OF_MEMORY",
            Self::OutputLimit => "SCRIPT_OUTPUT_LIMIT",
            Self::Denied => "SCRIPT_DENIED",
            Self::SchemaViolation => "SCRIPT_SCHEMA_VIOLATION",
            Self::NoResult => "SCRIPT_NO_RESULT",
            Self::Unavailable => "SCRIPT_SANDBOX_UNAVAILABLE",
        }
    }

    /// Whether running it again could end differently. A script that ran out of memory or broke its
    /// schema will do it again; a sandbox that was unavailable, or a time that ran out on a busy
    /// machine, may not.
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Timeout | Self::Unavailable | Self::Error)
    }
}

/// The text that replaces what must not be shown.
pub const REDACTED: &str = "[REDACTED]";

/// Shortest secret value that is searched for in output: a shorter one would redact ordinary words.
pub const MIN_REDACTED_LEN: usize = 6;

/// Output of a script, as it may be kept and shown: every value in `secrets` (the secrets this run
/// was handed) and what looks like a credential is replaced by [`REDACTED`].
///
/// This is a net, not a wall: a script can encode what it knows, and nothing here can see through
/// that. What stops a script from reading a secret it was not given is that it is not in the
/// sandbox; what is done here is to keep the ones it was given from reaching a log or a result by
/// accident.
#[must_use]
pub fn redact(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_owned();
    for secret in secrets {
        if secret.len() >= MIN_REDACTED_LEN {
            out = out.replace(secret, REDACTED);
        }
    }
    redact_credentials(&out)
}

/// Replaces the shapes of credentials that are recognisable without knowing the value: an AWS access
/// key id, a PEM private key block, and a bearer token in an `Authorization` header.
fn redact_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &text[i..];
        if let Some(header) = rest.strip_prefix("-----BEGIN ")
            && let Some(kind) = header.find("-----").map(|at| &header[..at])
            && kind.ends_with("PRIVATE KEY")
        {
            // The block ends after the closing dashes of its END line, or with the text.
            let block_end = rest
                .find("-----END ")
                .and_then(|at| {
                    let from = at + "-----END ".len();
                    rest[from..].find("-----").map(|close| from + close + 5)
                })
                .unwrap_or(rest.len());
            out.push_str(REDACTED);
            i += block_end;
            continue;
        }
        if (rest.starts_with("AKIA") || rest.starts_with("ASIA"))
            && rest.len() >= 20
            && rest.as_bytes()[4..20]
                .iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
        {
            out.push_str(REDACTED);
            i += 20;
            continue;
        }
        if let Some(after) = rest.strip_prefix("Bearer ")
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
        {
            let token = after
                .bytes()
                .take_while(|b| {
                    b.is_ascii_alphanumeric()
                        || matches!(b, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=')
                })
                .count();
            if token >= 8 {
                out.push_str("Bearer ");
                out.push_str(REDACTED);
                i += 7 + token;
                continue;
            }
        }
        let ch = rest.chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8().max(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_round_trips_and_only_some_are_built() {
        for language in [
            Language::TypeScript,
            Language::Python,
            Language::CSharp,
            Language::Go,
            Language::Java,
        ] {
            assert_eq!(language.as_str().parse::<Language>(), Ok(language));
        }
        assert_eq!(
            "cobol".parse::<Language>(),
            Err(ScriptError::UnknownLanguage)
        );
        assert!(Language::TypeScript.is_built() && Language::Python.is_built());
        assert!(!Language::Go.is_built() && !Language::Java.is_built());
    }

    #[test]
    fn a_source_has_text_and_a_bound_and_never_prints_itself() {
        assert!(Source::parse("export default () => 1").is_ok());
        assert_eq!(Source::parse("   \n"), Err(ScriptError::InvalidSource));
        assert_eq!(Source::parse("a\0b"), Err(ScriptError::InvalidSource));
        assert!(Source::parse(&"x".repeat(MAX_SOURCE_BYTES)).is_ok());
        assert_eq!(
            Source::parse(&"x".repeat(MAX_SOURCE_BYTES + 1)),
            Err(ScriptError::InvalidSource)
        );
        let shown = format!(
            "{:?}",
            Source::parse("const key = 'hunter2-hunter2'").expect("source")
        );
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(
            Lockfile::parse("").is_ok(),
            "an empty lockfile is a lockfile with nothing in it"
        );
        assert_eq!(
            Lockfile::parse(&"x".repeat(MAX_LOCKFILE_BYTES + 1)),
            Err(ScriptError::InvalidLockfile)
        );
    }

    #[test]
    fn a_digest_is_sha256_in_lower_case_hex_and_round_trips() {
        let bytes: [u8; 32] = core::array::from_fn(|i| u8::try_from(i * 7).unwrap_or(0));
        let digest = Digest::from_bytes(bytes);
        assert!(digest.as_str().starts_with("sha256:"));
        assert_eq!(Digest::parse(digest.as_str()).as_ref(), Ok(&digest));
        assert_eq!(digest.to_bytes(), bytes);
        for bad in [
            "",
            "sha256:",
            "sha256:ABCDEF",
            &format!("sha256:{}", "g".repeat(64)),
            &format!("sha256:{}", "a".repeat(63)),
            &format!("sha1:{}", "a".repeat(64)),
            &format!("sha256:{}", "A".repeat(64)),
        ] {
            assert_eq!(
                Digest::parse(bad),
                Err(ScriptError::InvalidDigest),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn limits_have_a_floor_and_a_ceiling_each() {
        let ok = |memory, cpu, wall, pids, output, scratch| {
            Limits::new(
                memory,
                cpu,
                Duration::from_secs(wall),
                pids,
                output,
                scratch,
            )
        };
        assert!(ok(256 * MIB, 1000, 30, 64, 64 * 1024, 32 * MIB).is_ok());
        assert_eq!(
            ok(MIB, 1000, 30, 64, 4096, MIB),
            Err(ScriptError::InvalidMemory)
        );
        assert_eq!(
            ok(4096 * MIB, 1000, 30, 64, 4096, MIB),
            Err(ScriptError::InvalidMemory)
        );
        assert_eq!(
            ok(64 * MIB, 50, 30, 64, 4096, MIB),
            Err(ScriptError::InvalidCpu)
        );
        assert_eq!(
            ok(64 * MIB, 5000, 30, 64, 4096, MIB),
            Err(ScriptError::InvalidCpu)
        );
        assert_eq!(
            ok(64 * MIB, 1000, 0, 64, 4096, MIB),
            Err(ScriptError::InvalidWallTime)
        );
        assert_eq!(
            ok(64 * MIB, 1000, 901, 64, 4096, MIB),
            Err(ScriptError::InvalidWallTime)
        );
        assert_eq!(
            ok(64 * MIB, 1000, 30, 4, 4096, MIB),
            Err(ScriptError::InvalidProcesses)
        );
        assert_eq!(
            ok(64 * MIB, 1000, 30, 600, 4096, MIB),
            Err(ScriptError::InvalidProcesses)
        );
        assert_eq!(
            ok(64 * MIB, 1000, 30, 64, 100, MIB),
            Err(ScriptError::InvalidOutputLimit)
        );
        assert_eq!(
            ok(64 * MIB, 1000, 30, 64, 4096, 0),
            Err(ScriptError::InvalidScratch)
        );
        let defaults = Limits::default();
        assert!(
            Limits::new(
                defaults.memory_bytes(),
                defaults.cpu_millis(),
                defaults.wall(),
                defaults.processes(),
                defaults.output_bytes(),
                defaults.scratch_bytes()
            )
            .is_ok(),
            "the defaults satisfy the bounds"
        );
    }

    #[test]
    fn cpu_time_follows_the_wall_clock_and_the_share_of_a_core() {
        let limits = |millis, wall| {
            Limits::new(64 * MIB, millis, Duration::from_secs(wall), 64, 4096, MIB).expect("limits")
        };
        assert_eq!(limits(1000, 30).cpu_seconds(), 30);
        assert_eq!(limits(500, 30).cpu_seconds(), 15);
        assert_eq!(limits(100, 1).cpu_seconds(), 1, "never under a second");
        assert_eq!(limits(2000, 10).cpu_seconds(), 20);
    }

    #[test]
    fn a_change_of_policy_is_a_change_of_its_text() {
        let base = Policy::default();
        let mut tighter = base;
        tighter.limits = Limits::new(
            64 * MIB,
            1000,
            Duration::from_secs(30),
            64,
            64 * 1024,
            32 * MIB,
        )
        .expect("limits");
        let mut open = base;
        open.egress = Egress::Namespace;
        let texts = [base.canonical(), tighter.canonical(), open.canonical()];
        assert_eq!(
            texts
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3,
            "{texts:?}"
        );
        assert!(
            texts[0].starts_with("v1;"),
            "versioned: the text can be changed on purpose"
        );
    }

    #[test]
    fn jobs_move_forward_and_a_lost_lease_puts_one_back() {
        use JobStatus::*;
        for (from, to) in [
            (Queued, Running),
            (Running, Succeeded),
            (Running, Failed),
            (Running, Queued),
            (Queued, Failed),
        ] {
            assert!(from.can_become(to), "{from:?} -> {to:?}");
        }
        for (from, to) in [
            (Succeeded, Running),
            (Failed, Queued),
            (Succeeded, Failed),
            (Queued, Succeeded),
            (Queued, Queued),
        ] {
            assert!(!from.can_become(to), "{from:?} -> {to:?}");
        }
        assert!(Succeeded.is_final() && Failed.is_final() && !Running.is_final());
        for status in [Queued, Running, Succeeded, Failed] {
            assert_eq!(status.as_str().parse::<JobStatus>(), Ok(status));
        }
    }

    #[test]
    fn what_a_script_cannot_do_twice_the_same_way_is_not_retried() {
        assert!(Failure::Timeout.retryable() && Failure::Unavailable.retryable());
        assert!(Failure::Error.retryable());
        for failure in [
            Failure::OutOfMemory,
            Failure::OutputLimit,
            Failure::Denied,
            Failure::SchemaViolation,
            Failure::NoResult,
        ] {
            assert!(!failure.retryable(), "{failure:?}");
        }
        let codes: std::collections::BTreeSet<_> = [
            Failure::Error,
            Failure::Timeout,
            Failure::OutOfMemory,
            Failure::OutputLimit,
            Failure::Denied,
            Failure::SchemaViolation,
            Failure::NoResult,
            Failure::Unavailable,
        ]
        .into_iter()
        .map(Failure::code)
        .collect();
        assert_eq!(codes.len(), 8, "every failure has a code of its own");
        assert!(codes.iter().all(|c| c.starts_with("SCRIPT_")));
    }

    #[test]
    fn the_secrets_a_run_was_given_do_not_reach_its_output() {
        let secrets = ["sk_live_0123456789abcdef", "tiny"];
        let out = redact(
            "charging with sk_live_0123456789abcdef now; tiny is too short",
            &secrets,
        );
        assert!(!out.contains("sk_live"), "{out}");
        assert!(out.contains(REDACTED));
        assert!(
            out.contains("tiny"),
            "a value of under six characters is not searched for: {out}"
        );
        assert_eq!(redact("nothing to hide", &secrets), "nothing to hide");
    }

    #[test]
    fn what_looks_like_a_credential_is_redacted_without_knowing_it() {
        let out = redact(
            "key AKIAIOSFODNN7EXAMPLE and Authorization: Bearer abcdefgh12345678.xyz",
            &[],
        );
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "{out}");
        assert!(!out.contains("abcdefgh12345678"), "{out}");
        assert!(out.contains("Bearer [REDACTED]"), "{out}");
        let pem =
            "before\n-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBg\n-----END PRIVATE KEY-----\nafter";
        let out = redact(pem, &[]);
        assert!(!out.contains("MIIEvQ"), "{out}");
        assert!(out.starts_with("before") && out.ends_with("after"), "{out}");
        // Ordinary text is left alone, including a word that starts like a key.
        assert_eq!(
            redact("AKIA is a word, Bearer short", &[]),
            "AKIA is a word, Bearer short"
        );
        assert_eq!(
            redact("héllo wörld ✓", &[]),
            "héllo wörld ✓",
            "multi-byte text survives"
        );
    }
}
