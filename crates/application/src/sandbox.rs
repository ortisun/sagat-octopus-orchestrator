//! The ports of the sandbox: what building and running a script need from the outside world, and
//! the keys that make a build repeatable (`FR-005-002`, `FR-005-004`, `FR-005-008`).
//!
//! Nothing here names a container. A [`Sandbox`] is whatever can build source into an artifact and
//! run an artifact without trusting it: the adapter in `sagat-infra-sandbox` does it with a
//! container per build and per run. JSON travels as bytes: the application neither reads nor
//! writes it, and the adapter, which must, owns the schemas.

use std::future::Future;
use std::time::Duration;

use sagat_domain::scripts::{Digest, Failure, Language, Lockfile, Policy, Source};
use sha2::{Digest as _, Sha256};

/// The longest result a script may return, in bytes. Below the most a run may retain.
pub const MAX_RESULT_BYTES: usize = 240 * 1024;

/// Source to be built.
#[derive(Debug, Clone)]
pub struct BuildRequest {
    pub language: Language,
    pub source: Source,
    /// Empty or absent: the script has no dependencies.
    pub lockfile: Option<Lockfile>,
    pub policy: Policy,
}

/// What a build made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    /// The artifact, as the sandbox will be given it. Its digest is the identity of the version.
    pub artifact: Vec<u8>,
    /// The software bill of materials: the toolchain and every package the lockfile pins (JSON).
    pub sbom: Vec<u8>,
    /// What built it: the digest of the toolchain image and the version of the runner's shim. Part
    /// of the build key.
    pub toolchain: String,
    /// The log of the build, redacted and bounded.
    pub log: String,
}

/// Why a build produced nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildFailure {
    /// The source does not compile (or parse), with the first message the toolchain gave.
    Rejected(String),
    /// The lockfile is not one the platform accepts: unreadable, or it pins something the policy
    /// does not allow, or without the integrity hashes that make it a lock.
    Lockfile(String),
    /// The language has no builder here.
    Unsupported,
    /// The build used more than it was given.
    Limit,
    /// The sandbox could not be used (the daemon is down, the image is missing).
    Unavailable,
}

/// A request a script makes of the outside world, which the supervisor may refuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// What came back from a request the supervisor made for a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Why the supervisor did not make a request, or did not hand over a secret, for a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Not allowed: the egress policy, the declared secrets. Counted and audited.
    Denied,
    /// Allowed, but it could not be done now.
    Failed,
}

/// The things a running script may ask its supervisor to do on its behalf. The sandbox has no
/// network and no secrets; this is the only way out and the only way in.
pub trait HostCalls: Send + Sync {
    /// An HTTP request, under the egress policy of the namespace.
    fn fetch(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpReply, Refusal>> + Send;

    /// A secret the script declared, for the length of this run.
    fn secret(&self, name: &str) -> impl Future<Output = Result<String, Refusal>> + Send;
}

/// A run of an artifact.
#[derive(Debug, Clone)]
pub struct RunRequest {
    pub language: Language,
    pub artifact: Vec<u8>,
    /// The input, JSON.
    pub input: Vec<u8>,
    /// JSON Schema the input must satisfy, if the script declared one.
    pub input_schema: Option<Vec<u8>>,
    /// JSON Schema the result must satisfy, if the script declared one.
    pub output_schema: Option<Vec<u8>>,
    pub policy: Policy,
    /// Values handed to this run, to be kept out of its logs and its result.
    pub secrets_to_hide: Vec<String>,
}

/// What a run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    /// The result (JSON), or why there is none.
    pub result: Result<Vec<u8>, Failure>,
    /// Standard error of the script, redacted and bounded to the policy's output limit.
    pub log: String,
    /// More was written than is kept.
    pub log_truncated: bool,
    /// What the script was refused: `fetch https://...`, `secret name`. One entry per refusal.
    pub denied: Vec<String>,
    pub exit_code: Option<i64>,
    pub wall: Duration,
    /// The most memory the sandbox was seen to use, when it could be seen.
    pub memory_peak_bytes: Option<u64>,
    /// Time from asking for a container to the script's first instruction (cold or warm start).
    pub start: Duration,
    /// Schemas inferred from this run's input and result (JSON Schema), for the author's form.
    pub inferred_input_schema: Option<Vec<u8>>,
    pub inferred_output_schema: Option<Vec<u8>>,
}

/// Builds and runs scripts without trusting them.
pub trait Sandbox: Send + Sync {
    /// The identity of the toolchain that builds `language` (the digest of its image and the
    /// version of the shim), which is part of every build key.
    fn toolchain(
        &self,
        language: Language,
    ) -> impl Future<Output = Result<String, BuildFailure>> + Send;

    fn build(
        &self,
        request: BuildRequest,
    ) -> impl Future<Output = Result<Built, BuildFailure>> + Send;

    fn run<H: HostCalls>(
        &self,
        request: RunRequest,
        host: &H,
    ) -> impl Future<Output = RunOutcome> + Send;
}

/// The digest of `bytes`.
#[must_use]
pub fn digest_of(bytes: &[u8]) -> Digest {
    Digest::from_bytes(Sha256::digest(bytes).into())
}

/// What makes two builds the same build: the source, the lockfile, the toolchain and the policy,
/// each length-prefixed so no two different tuples hash alike. A build is reused only when every
/// one of them is identical; a change of policy therefore invalidates the cache.
#[must_use]
pub fn build_key(
    language: Language,
    source: &Source,
    lockfile: Option<&Lockfile>,
    toolchain: &str,
    policy: &Policy,
) -> Digest {
    let mut hash = Sha256::new();
    for part in [
        b"sagat/build/v1".as_slice(),
        language.as_str().as_bytes(),
        source.as_str().as_bytes(),
        lockfile.map_or("", Lockfile::as_str).as_bytes(),
        toolchain.as_bytes(),
        policy.canonical().as_bytes(),
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    Digest::from_bytes(hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sagat_domain::scripts::{Egress, Limits};

    fn key(source: &str, lock: Option<&str>, toolchain: &str, policy: &Policy) -> Digest {
        build_key(
            Language::TypeScript,
            &Source::parse(source).expect("source"),
            lock.map(|l| Lockfile::parse(l).expect("lockfile")).as_ref(),
            toolchain,
            policy,
        )
    }

    #[test]
    fn a_build_is_the_same_build_only_if_everything_that_matters_is() {
        let policy = Policy::default();
        let base = key(
            "export default () => 1",
            None,
            "node@sha256:aa;shim1",
            &policy,
        );
        assert_eq!(
            base,
            key(
                "export default () => 1",
                None,
                "node@sha256:aa;shim1",
                &policy
            )
        );
        assert_ne!(
            base,
            key(
                "export default () => 2",
                None,
                "node@sha256:aa;shim1",
                &policy
            )
        );
        assert_ne!(
            base,
            key(
                "export default () => 1",
                Some("{}"),
                "node@sha256:aa;shim1",
                &policy
            )
        );
        assert_ne!(
            base,
            key(
                "export default () => 1",
                None,
                "node@sha256:bb;shim1",
                &policy
            )
        );
        assert_ne!(
            base,
            key(
                "export default () => 1",
                None,
                "node@sha256:aa;shim2",
                &policy
            )
        );
        let mut open = policy;
        open.egress = Egress::Namespace;
        assert_ne!(
            base,
            key(
                "export default () => 1",
                None,
                "node@sha256:aa;shim1",
                &open
            )
        );
        let mut tighter = policy;
        tighter.limits = Limits::new(
            64 * 1024 * 1024,
            1000,
            Duration::from_secs(30),
            64,
            64 * 1024,
            32 * 1024 * 1024,
        )
        .expect("limits");
        assert_ne!(
            base,
            key(
                "export default () => 1",
                None,
                "node@sha256:aa;shim1",
                &tighter
            )
        );
    }

    #[test]
    fn the_fields_of_a_key_cannot_be_shifted_into_one_another() {
        let policy = Policy::default();
        // Without length prefixes the source "ab" and lockfile "c" would hash like "a" and "bc".
        assert_ne!(
            key("ab", Some("c"), "t", &policy),
            key("a", Some("bc"), "t", &policy)
        );
        assert_ne!(key("a", None, "bc", &policy), key("ab", None, "c", &policy));
    }

    #[test]
    fn a_digest_is_the_sha256_of_the_bytes() {
        // The well-known digest of the empty input.
        assert_eq!(
            digest_of(b"").as_str(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_ne!(digest_of(b"a"), digest_of(b"b"));
    }
}
