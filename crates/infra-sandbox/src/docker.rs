//! The sandbox, made of containers.
//!
//! One container per build and per run, and nothing is shared between two of them. Each is created
//! with: no network interface (`none`), a read-only root filesystem, a small writable `/tmp` that is
//! not executable, every Linux capability dropped, `no-new-privileges`, the engine's default seccomp
//! profile, an unprivileged user, a private PID and IPC namespace, a memory limit with swap off, a
//! CPU quota and a CPU-time limit, a limit on processes, on file size and on open files, no log
//! driver, no mounts and nothing from the host. The code gets in as the first message on the
//! container's standard input and the result gets out as one line on standard output.
//!
//! This is as strong as the container runtime under it (`runc` here; a kernel bug or a runtime bug
//! is an escape). A stronger runtime (gVisor, Kata) is a matter of the engine's configuration, not
//! of this code, and is the recommended setting for a shared deployment.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bollard::Docker;
use bollard::container::LogOutput;
use bollard::models::{ContainerCreateBody, HostConfig, HostConfigLogConfig, ResourcesUlimits};
use bollard::query_parameters::{
    AttachContainerOptionsBuilder, CreateContainerOptionsBuilder, CreateImageOptionsBuilder,
    RemoveContainerOptionsBuilder, StatsOptionsBuilder,
};
use futures_util::StreamExt as _;
use sagat_application::sandbox::{
    BuildFailure, BuildRequest, Built, HostCalls, HttpRequest, MAX_RESULT_BYTES, Refusal,
    RunOutcome, RunRequest, Sandbox,
};
use sagat_domain::scripts::{Failure, Language, Limits, Policy, redact};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

use crate::lockfile;
use crate::protocol::{self, Message};
use sagat_infra_schema as schema;

/// The images the languages run in. Name them by digest in a deployment: a tag moves, a digest
/// does not, and the digest is part of every build key.
#[derive(Debug, Clone)]
pub struct Images {
    pub typescript: String,
    pub python: String,
    pub csharp_build: String,
    pub csharp_runtime: String,
}

impl Default for Images {
    fn default() -> Self {
        Self {
            typescript: "node:24-alpine".to_owned(),
            python: "python:3.13-alpine".to_owned(),
            csharp_build: "sagat/dotnet-builder:10.0.401".to_owned(),
            csharp_runtime: "sagat/dotnet-runtime:10.0.12".to_owned(),
        }
    }
}

const LABEL: &str = "sagat.sandbox";
/// Which sandbox (which `DockerSandbox` value) made a container: lets one ask what *it* left behind
/// without counting what another one is running.
const OWNER_LABEL: &str = "sagat.sandbox.owner";

/// Logs which step with the container engine failed and why, and turns the failure into `()`: what
/// the engine said is for the operator, never for the author of the script.
fn failed_at<E: std::fmt::Display>(stage: &'static str) -> impl FnOnce(E) {
    move |error| tracing::error!(stage, %error, "the sandbox could not be driven")
}
/// Standard error read from a script before it is cut off, as a multiple of what is kept: a script
/// that floods its log is stopped, not read for ever.
const LOG_FLOOD_FACTOR: usize = 16;
/// A line of the channel is at most the largest result plus its framing.
const MAX_LINE: usize = MAX_RESULT_BYTES + 16 * 1024;
/// A compiled C# assembly and its allowlisted managed dependencies cross this line. It remains
/// below the control plane's artifact cap plus bounded protocol framing.
const MAX_BUILD_LINE: usize = sagat_infra_schema::bundle::MAX_BUNDLE_BYTES + 64 * 1024;
const KILL_GRACE: Duration = Duration::from_secs(2);
/// How many times, and how far apart, a 137 without the out-of-memory flag is looked at again (two seconds in all: on
/// a loaded machine the engine's event has been seen to come later than a quarter of a second).
const OOM_FLAG_ATTEMPTS: u32 = 40;
const OOM_FLAG_PAUSE: Duration = Duration::from_millis(50);
/// Requests and secrets a script may ask its supervisor for in one run. Past it every ask is refused:
/// a script in a loop must not make the supervisor do the same work as many times as it can ask.
const MAX_HOST_CALLS: usize = 100;
/// Distinct refusals kept for the record of a run.
const MAX_DENIED: usize = 32;
/// How long the log is read after the script has said its last word: standard output and standard
/// error reach the supervisor by separate paths, so the end of one can arrive before the end of the
/// other. The shims exit right after their last message, so the stream normally ends at once.
const DRAIN: Duration = Duration::from_millis(750);
/// Lines a script may write on the channel in one run, whatever they say: past the host calls it is
/// answered, the rest is noise, and a script that keeps writing is stopped instead of read for ever.
const MAX_CHANNEL_LINES: usize = MAX_HOST_CALLS * 4;
/// Time a run is given beyond its wall clock for what is not the script's: starting the container
/// and reading it back. Nothing the engine does is allowed to hold a runner longer than this.
const SETUP_GRACE: Duration = Duration::from_secs(60);
/// The longest any one call to the container engine is waited for. An engine that stops answering
/// (it can: a daemon wedged on one container) must cost a run this much, not a runner for ever.
/// Not exercised against a wedged engine in the test suite: that needs one (G117).
const ENGINE_CALL: Duration = Duration::from_secs(20);

/// `future`, unless the engine takes longer than [`ENGINE_CALL`] to answer.
async fn within<T>(future: impl std::future::Future<Output = T>) -> Option<T> {
    tokio::time::timeout(ENGINE_CALL, future).await.ok()
}
/// What is kept of one refusal. A script names what it asks for and could name it at any length.
const MAX_DENIED_LEN: usize = 200;
/// Standard error is read this much past what is kept so a secret on the edge of the cut is still
/// whole when it is looked for; the cut is made after.
const LOG_REDACTION_MARGIN: usize = 8 * 1024;

/// The container engine, as a sandbox.
pub struct DockerSandbox {
    docker: Docker,
    images: Images,
    pulled: Mutex<HashSet<String>>,
    owner: String,
}

impl std::fmt::Debug for DockerSandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockerSandbox")
            .field("images", &self.images)
            .finish_non_exhaustive()
    }
}

/// What a container did, before anyone decides what it means.
#[derive(Debug, Default)]
struct Driven {
    message: Option<Message>,
    log: Vec<u8>,
    log_truncated: bool,
    denied: Vec<String>,
    exit_code: Option<i64>,
    oom: bool,
    timed_out: bool,
    flooded: bool,
    wall: Duration,
    start: Duration,
    memory_peak: Option<u64>,
    unavailable: bool,
    /// Values the supervisor handed to the script during the run: to be kept out of its log.
    handed: Vec<String>,
}

struct Job {
    image: String,
    command: Vec<String>,
    first_message: Vec<u8>,
    limits: Limits,
    max_line: usize,
}

/// The write side of the channel to a script.
type ToScript = Pin<Box<dyn AsyncWrite + Send>>;

/// Tells the script something, unless it takes longer than the run has: a script that stopped
/// reading its replies would otherwise fill the pipe and hold its supervisor, and the runner slot,
/// for ever. `false` means the run is over.
async fn send(input: &mut ToScript, bytes: &[u8], deadline: tokio::time::Instant) -> bool {
    let written = tokio::time::timeout_at(deadline, async {
        input.write_all(bytes).await?;
        input.flush().await
    })
    .await;
    matches!(written, Ok(Ok(())))
}

impl DockerSandbox {
    /// Connects to the container engine the environment names (the local socket by default).
    ///
    /// # Errors
    /// The engine cannot be reached.
    pub fn connect(images: Images) -> Result<Self, bollard::errors::Error> {
        Ok(Self {
            docker: Docker::connect_with_local_defaults()?,
            images,
            pulled: Mutex::new(HashSet::new()),
            owner: uuid::Uuid::now_v7().to_string(),
        })
    }

    fn build_image_of(&self, language: Language) -> Option<&str> {
        match language {
            Language::TypeScript => Some(&self.images.typescript),
            Language::Python => Some(&self.images.python),
            Language::CSharp => Some(&self.images.csharp_build),
            _ => None,
        }
    }

    fn runtime_image_of(&self, language: Language) -> Option<&str> {
        match language {
            Language::TypeScript => Some(&self.images.typescript),
            Language::Python => Some(&self.images.python),
            Language::CSharp => Some(&self.images.csharp_runtime),
            _ => None,
        }
    }

    async fn ensure_image(&self, reference: &str) -> Result<String, ()> {
        if let Ok(found) = self.docker.inspect_image(reference).await {
            return found.id.ok_or(());
        }
        let options = if reference.contains('@') {
            // `name@sha256:<hex>`: the digest is the whole reference, there is no tag to split off.
            CreateImageOptionsBuilder::default()
                .from_image(reference)
                .build()
        } else {
            let (name, tag) = reference
                .rsplit_once(':')
                .filter(|(_, t)| !t.contains('/'))
                .unwrap_or((reference, "latest"));
            CreateImageOptionsBuilder::default()
                .from_image(name)
                .tag(tag)
                .build()
        };
        let mut pull = self.docker.create_image(Some(options), None, None);
        while let Some(progress) = pull.next().await {
            if let Err(error) = progress {
                tracing::error!(image = reference, %error, "the image could not be pulled");
                return Err(());
            }
        }
        self.pulled
            .lock()
            .map_err(|_| ())?
            .insert(reference.to_owned());
        self.docker
            .inspect_image(reference)
            .await
            .map_err(|_| ())?
            .id
            .ok_or(())
    }

    /// Containers this sandbox made that still exist, running or not: what a run that cleaned up
    /// after itself leaves is none.
    pub async fn leftovers(&self) -> usize {
        use bollard::query_parameters::ListContainersOptionsBuilder;
        let filters = HashMap::from([(
            "label".to_owned(),
            vec![format!("{OWNER_LABEL}={}", self.owner)],
        )]);
        let options = ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&filters)
            .build();
        within(self.docker.list_containers(Some(options)))
            .await
            .and_then(Result::ok)
            .map_or(0, |found| found.len())
    }

    /// Removes the sandboxes older than `age` that something left behind (a supervisor that died
    /// mid-run): every container this crate makes carries a label.
    pub async fn sweep(&self, age: Duration) -> usize {
        use bollard::query_parameters::ListContainersOptionsBuilder;
        let filters = HashMap::from([("label".to_owned(), vec![format!("{LABEL}=1")])]);
        let options = ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&filters)
            .build();
        let Some(Ok(found)) = within(self.docker.list_containers(Some(options))).await else {
            return 0;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut removed = 0;
        for container in found {
            let created = u64::try_from(container.created.unwrap_or(0)).unwrap_or(0);
            if now.saturating_sub(created) >= age.as_secs()
                && let Some(id) = container.id
                && self.remove(&id).await
            {
                removed += 1;
            }
        }
        removed
    }

    async fn remove(&self, name: &str) -> bool {
        let options = RemoveContainerOptionsBuilder::default()
            .force(true)
            .v(true)
            .build();
        within(self.docker.remove_container(name, Some(options)))
            .await
            .is_some_and(|removed| removed.is_ok())
    }

    fn config(job: &Job, owner: &str) -> ContainerCreateBody {
        let limits = &job.limits;
        let memory = i64::try_from(limits.memory_bytes()).unwrap_or(i64::MAX);
        let scratch = limits.scratch_bytes();
        let ulimit = |name: &str, soft: i64, hard: i64| ResourcesUlimits {
            name: Some(name.to_owned()),
            soft: Some(soft),
            hard: Some(hard),
        };
        ContainerCreateBody {
            image: Some(job.image.clone()),
            cmd: Some(job.command.clone()),
            working_dir: Some("/tmp".to_owned()),
            user: Some("65534:65534".to_owned()),
            env: Some(vec![
                "HOME=/tmp".to_owned(),
                "LANG=C.UTF-8".to_owned(),
                "PYTHONDONTWRITEBYTECODE=1".to_owned(),
            ]),
            labels: Some(HashMap::from([
                (LABEL.to_owned(), "1".to_owned()),
                (OWNER_LABEL.to_owned(), owner.to_owned()),
            ])),
            network_disabled: Some(true),
            open_stdin: Some(true),
            stdin_once: Some(true),
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(false),
            host_config: Some(HostConfig {
                network_mode: Some("none".to_owned()),
                readonly_rootfs: Some(true),
                cap_drop: Some(vec!["ALL".to_owned()]),
                security_opt: Some(vec!["no-new-privileges".to_owned()]),
                privileged: Some(false),
                memory: Some(memory),
                memory_swap: Some(memory),
                nano_cpus: Some(i64::from(limits.cpu_millis()) * 1_000_000),
                pids_limit: Some(i64::from(limits.processes())),
                tmpfs: Some(HashMap::from([(
                    "/tmp".to_owned(),
                    format!("rw,noexec,nosuid,nodev,size={}", scratch),
                )])),
                ulimits: Some(vec![
                    ulimit("nofile", 256, 256),
                    ulimit("core", 0, 0),
                    ulimit(
                        "fsize",
                        i64::try_from(scratch).unwrap_or(i64::MAX),
                        i64::try_from(scratch).unwrap_or(i64::MAX),
                    ),
                    ulimit(
                        "cpu",
                        i64::try_from(limits.cpu_seconds()).unwrap_or(i64::MAX),
                        i64::try_from(limits.cpu_seconds() + 1).unwrap_or(i64::MAX),
                    ),
                ]),
                init: Some(true),
                ipc_mode: Some("private".to_owned()),
                auto_remove: Some(false),
                log_config: Some(HostConfigLogConfig {
                    typ: Some("none".to_owned()),
                    config: None,
                }),
                ..HostConfig::default()
            }),
            ..ContainerCreateBody::default()
        }
    }

    /// Runs `job` to its end, serving the host calls of the code in it with `host`.
    async fn drive<H: HostCalls>(&self, job: Job, host: Option<&H>) -> Driven {
        let began = Instant::now();
        let mut driven = Driven::default();
        let name = format!("sagat-sbx-{}", uuid::Uuid::now_v7());
        // If this future is dropped (the step was cancelled, the runner is stopping) the container
        // is still removed, in the background: a script never outlives the call that ran it. The
        // guard exists before the container does, so one made after the engine was given up on is
        // removed too.
        let guard = Removal::new(self.docker.clone(), name.clone());
        let create = CreateContainerOptionsBuilder::default().name(&name).build();
        match within(
            self.docker
                .create_container(Some(create), Self::config(&job, &self.owner)),
        )
        .await
        {
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                failed_at("create")(error);
                driven.unavailable = true;
                return driven;
            }
            None => {
                failed_at("create")("the engine did not answer in time");
                driven.unavailable = true;
                return driven;
            }
        }
        let outcome = match tokio::time::timeout(
            job.limits.wall() + SETUP_GRACE,
            self.drive_created(&name, &job, host, began, &mut driven),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(_) => {
                driven.timed_out = driven.message.is_none();
                Ok(())
            }
        };
        if outcome.is_err() {
            driven.unavailable = true;
        }
        self.remove(&name).await;
        guard.disarm();
        driven.wall = began.elapsed();
        driven
    }

    /// Whether the engine says, within a moment, that the container was killed for its memory.
    async fn oom_flag_settles(&self, name: &str) -> bool {
        for _ in 0..OOM_FLAG_ATTEMPTS {
            tokio::time::sleep(OOM_FLAG_PAUSE).await;
            match within(self.docker.inspect_container(name, None)).await {
                Some(Ok(info)) => {
                    if info.state.as_ref().and_then(|s| s.oom_killed) == Some(true) {
                        return true;
                    }
                }
                _ => return false,
            }
        }
        false
    }

    async fn drive_created<H: HostCalls>(
        &self,
        name: &str,
        job: &Job,
        host: Option<&H>,
        began: Instant,
        driven: &mut Driven,
    ) -> Result<(), ()> {
        let attach = AttachContainerOptionsBuilder::default()
            .stream(true)
            .stdin(true)
            .stdout(true)
            .stderr(true)
            .build();
        let mut attached = self
            .docker
            .attach_container(name, Some(attach))
            .await
            .map_err(failed_at("attach"))?;
        self.docker
            .start_container(name, None)
            .await
            .map_err(failed_at("start"))?;
        let sampler = self.sample_memory(name);
        let limits = job.limits;
        let deadline = tokio::time::Instant::now() + limits.wall();
        if !send(&mut attached.input, &job.first_message, deadline).await {
            failed_at("send")("the script did not take its first message");
            return Err(());
        }
        let keep = limits.output_bytes();
        let capacity = keep.saturating_add(LOG_REDACTION_MARGIN);
        let mut line: Vec<u8> = Vec::new();
        let mut lines = 0_usize;
        let mut secrets_handed: Vec<String> = Vec::new();
        let mut stderr_seen = 0_usize;
        let mut host_calls = 0_usize;
        let mut drain_until: Option<tokio::time::Instant> = None;
        'read: loop {
            let limit = drain_until.map_or(deadline, |end| end.min(deadline));
            let next = tokio::time::timeout_at(limit, attached.output.next()).await;
            let chunk = match next {
                Err(_) => {
                    driven.timed_out = driven.message.is_none();
                    break;
                }
                Ok(None) | Ok(Some(Err(_))) => break,
                Ok(Some(Ok(chunk))) => chunk,
            };
            match chunk {
                LogOutput::StdErr { message } => {
                    let room = capacity.saturating_sub(driven.log.len());
                    driven
                        .log
                        .extend_from_slice(&message[..message.len().min(room)]);
                    if message.len() > room {
                        driven.log_truncated = true;
                    }
                    // Past what is kept the script is read a while longer and then stopped.
                    stderr_seen = stderr_seen.saturating_add(message.len());
                    if driven.log_truncated
                        && stderr_seen > capacity.saturating_mul(LOG_FLOOD_FACTOR)
                    {
                        driven.flooded = true;
                        break;
                    }
                }
                LogOutput::StdOut { .. } | LogOutput::Console { .. }
                    if driven.message.is_some() => {}
                LogOutput::StdOut { message } | LogOutput::Console { message } => {
                    for byte in message.iter().copied() {
                        if byte != b'\n' {
                            line.push(byte);
                            if line.len() > job.max_line {
                                driven.flooded = true;
                                break 'read;
                            }
                            continue;
                        }
                        let parsed = protocol::parse(&line);
                        line.clear();
                        lines += 1;
                        if lines > MAX_CHANNEL_LINES {
                            driven.flooded = true;
                            break 'read;
                        }
                        let Some(parsed) = parsed else { continue };
                        if let Message::Fetch { id, .. } | Message::Secret { id, .. } = &parsed {
                            host_calls += 1;
                            if host_calls > MAX_HOST_CALLS {
                                deny(&mut driven.denied, "host calls over the limit".to_owned());
                                let refusal = protocol::refusal(*id, "denied");
                                if !send(&mut attached.input, &refusal, deadline).await {
                                    driven.timed_out = driven.message.is_none();
                                    break 'read;
                                }
                                continue;
                            }
                        }
                        match parsed {
                            Message::Ready => {
                                driven.start = began.elapsed();
                            }
                            Message::Fetch { id, request } => {
                                let reply =
                                    self.serve_fetch(host, id, request, driven, deadline).await;
                                match reply {
                                    Some(bytes) => {
                                        if !send(&mut attached.input, &bytes, deadline).await {
                                            driven.timed_out = true;
                                            break 'read;
                                        }
                                    }
                                    None => {
                                        driven.timed_out = true;
                                        break 'read;
                                    }
                                }
                            }
                            Message::Secret { id, name: secret } => {
                                let reply = match host {
                                    None => {
                                        deny(&mut driven.denied, format!("secret {secret}"));
                                        protocol::refusal(id, "denied")
                                    }
                                    Some(host) => match tokio::time::timeout_at(
                                        deadline,
                                        host.secret(&secret),
                                    )
                                    .await
                                    {
                                        Err(_) => {
                                            driven.timed_out = true;
                                            break 'read;
                                        }
                                        Ok(Ok(value)) => {
                                            secrets_handed.push(value.clone());
                                            protocol::secret_reply(id, &value)
                                        }
                                        Ok(Err(Refusal::Denied)) => {
                                            deny(&mut driven.denied, format!("secret {secret}"));
                                            protocol::refusal(id, "denied")
                                        }
                                        Ok(Err(Refusal::Failed)) => {
                                            protocol::refusal(id, "unavailable")
                                        }
                                    },
                                };
                                if !send(&mut attached.input, &reply, deadline).await {
                                    driven.timed_out = true;
                                    break 'read;
                                }
                            }
                            done @ (Message::Result(_)
                            | Message::Error { .. }
                            | Message::Built { .. }
                            | Message::Rejected(_)) => {
                                driven.message = Some(done);
                                drain_until = Some(tokio::time::Instant::now() + DRAIN);
                                // Whatever else the script says on the channel after this is not heard.
                                break;
                            }
                        }
                    }
                }
                LogOutput::StdIn { .. } => {}
            }
        }
        // The code has said what it had to say, or run out of time or room: end it, then see how.
        drop(attached);
        // A container that is already gone when it is time to end it was ended by something else.
        let ended_alone = match within(self.docker.inspect_container(name, None)).await {
            Some(Ok(info)) => info.state.is_some_and(|state| state.running == Some(false)),
            _ => false,
        };
        let _ = within(self.docker.kill_container(
            name,
            None::<bollard::query_parameters::KillContainerOptions>,
        ))
        .await;
        driven.memory_peak = sampler.finish().await;
        let end = Instant::now() + KILL_GRACE;
        loop {
            if let Some(Ok(state)) = within(self.docker.inspect_container(name, None)).await {
                let state = state.state.unwrap_or_default();
                if state.running != Some(true) || Instant::now() >= end {
                    driven.exit_code = state.exit_code;
                    driven.oom = state.oom_killed == Some(true);
                    // 137 is SIGKILL: what the kernel's out-of-memory killer sends, and what our own kill sends.
                    // The engine learns that the memory limit did it from an event that can arrive a moment after
                    // the one that says the container stopped (it does, on the CI's Linux), so a script that
                    // gave no answer, did not run out of time and was not flooded, and ended on a 137, is asked
                    // about again for a moment before the flag is believed to be false.
                    if !driven.oom
                        && state.running != Some(true)
                        && driven.exit_code == Some(137)
                        && driven.message.is_none()
                        && !driven.timed_out
                        && !driven.flooded
                    {
                        // The engine's own word first. On some hosts it never comes (the kernel killed the
                        // script, `exit 137`, and `OOMKilled` stays false): a container that had already died
                        // by SIGKILL on its own, when nothing of ours had asked it to stop and it had said
                        // nothing, is taken to have been killed for its memory, the one thing that kills a
                        // script that way inside its sandbox. That is an inference, and the only one made.
                        driven.oom = self.oom_flag_settles(name).await || ended_alone;
                    }
                    break;
                }
            } else {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // The CPU-time limit (SIGXCPU, 128 + 24) can end a burning script a moment before the wall
        // clock does: it is the same limit, and says the same thing.
        if driven.message.is_none() && driven.exit_code == Some(152) {
            driven.timed_out = true;
        }
        tracing::debug!(
            stderr_seen,
            lines,
            flooded = driven.flooded,
            timed_out = driven.timed_out,
            oom = driven.oom,
            exit_code = ?driven.exit_code,
            answered = driven.message.is_some(),
            log_len = driven.log.len(),
            log_truncated = driven.log_truncated,
            "a sandbox run ended"
        );
        driven.handed = secrets_handed;
        Ok(())
    }

    async fn serve_fetch<H: HostCalls>(
        &self,
        host: Option<&H>,
        id: u64,
        request: HttpRequest,
        driven: &mut Driven,
        deadline: tokio::time::Instant,
    ) -> Option<Vec<u8>> {
        let target = destination(&request.url);
        let Some(host) = host else {
            deny(&mut driven.denied, format!("fetch {target}"));
            return Some(protocol::refusal(id, "denied"));
        };
        match tokio::time::timeout_at(deadline, host.fetch(request)).await {
            Err(_) => None,
            Ok(Ok(reply)) => Some(protocol::fetch_reply(id, &reply)),
            Ok(Err(Refusal::Denied)) => {
                deny(&mut driven.denied, format!("fetch {target}"));
                Some(protocol::refusal(id, "denied"))
            }
            Ok(Err(Refusal::Failed)) => Some(protocol::refusal(id, "unavailable")),
        }
    }

    fn sample_memory(&self, name: &str) -> Sampler {
        let docker = self.docker.clone();
        let name = name.to_owned();
        let handle = tokio::spawn(async move {
            let options = StatsOptionsBuilder::default().stream(true).build();
            let mut stats = docker.stats(&name, Some(options));
            let mut peak: Option<u64> = None;
            while let Some(Ok(sample)) = stats.next().await {
                if let Some(usage) = sample.memory_stats.and_then(|m| m.usage) {
                    peak = Some(peak.map_or(usage, |p| p.max(usage)));
                }
            }
            peak
        });
        Sampler(handle)
    }
}

/// Removes a container when dropped, unless it was disarmed because the caller removed it.
struct Removal {
    docker: Docker,
    name: String,
    armed: bool,
}

impl Removal {
    const fn new(docker: Docker, name: String) -> Self {
        Self {
            docker,
            name,
            armed: true,
        }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for Removal {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let (docker, name) = (self.docker.clone(), self.name.clone());
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let options = RemoveContainerOptionsBuilder::default()
                    .force(true)
                    .v(true)
                    .build();
                let _ = docker.remove_container(&name, Some(options)).await;
            });
        }
    }
}

struct Sampler(tokio::task::JoinHandle<Option<u64>>);

impl Sampler {
    async fn finish(self) -> Option<u64> {
        // The stream ends with the container; give it a moment to say its last sample.
        match tokio::time::timeout(Duration::from_millis(300), self.0).await {
            Ok(Ok(peak)) => peak,
            _ => None,
        }
    }
}

/// Notes what a script was refused, once.
fn deny(denied: &mut Vec<String>, what: String) {
    let what: String = what.chars().take(MAX_DENIED_LEN).collect();
    if denied.len() < MAX_DENIED && !denied.contains(&what) {
        denied.push(what);
    }
}

/// `scheme://host[:port]` of a URL: what may be recorded about a refused request. The path and the
/// query can be the credential.
fn destination(url: &str) -> String {
    let Some(from) = url.find("://").map(|at| at + 3) else {
        return url.chars().take(MAX_DENIED_LEN).collect();
    };
    let authority_end = url[from..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |rel| from + rel);
    // `user:password@` is a credential too: only what follows the last `@` is the host.
    let host_from = url[from..authority_end]
        .rfind('@')
        .map_or(from, |at| from + at + 1);
    format!("{}{}", &url[..from], &url[host_from..authority_end])
        .chars()
        .take(MAX_DENIED_LEN)
        .collect()
}

fn shim_version() -> String {
    let mut hash = Sha256::new();
    for part in [RUNTIME_MJS, RUNTIME_PY, BUILD_MJS, BUILD_PY] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

const RUNTIME_MJS: &str = include_str!("../shims/runtime.mjs");
const RUNTIME_PY: &str = include_str!("../shims/runtime.py");
const BUILD_MJS: &str = include_str!("../shims/build.mjs");
const BUILD_PY: &str = include_str!("../shims/build.py");

/// The artifact: the code to run and the rules it was built under. The rules are part of the bytes
/// and so of the digest and the signature, which is what lets a digest decide what limits a script
/// runs under: two builds of the same code under other limits are two artifacts.
fn artifact_json(
    language: Language,
    policy: &Policy,
    contract: (Option<&[u8]>, Option<&[u8]>),
    entry: &str,
    files: &[(String, String)],
) -> Vec<u8> {
    let schema = |bytes: Option<&[u8]>| bytes.and_then(|b| serde_json::from_slice::<Value>(b).ok());
    let files: std::collections::BTreeMap<&str, &str> = files
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let value = if language == Language::CSharp {
        json!({
            "v": 2,
            "language": language.as_str(),
            "policy": policy.canonical(),
            "inputSchema": schema(contract.0),
            "outputSchema": schema(contract.1),
            "entry": entry,
            "encoding": "base64",
            "files": files,
        })
    } else {
        json!({
            "v": 1,
            "language": language.as_str(),
            "policy": policy.canonical(),
            "inputSchema": schema(contract.0),
            "outputSchema": schema(contract.1),
            "entry": entry,
            "files": files,
        })
    };
    serde_json::to_vec(&value).unwrap_or_default()
}

/// The schemas an artifact was built with: what its input and its result must satisfy.
#[must_use]
pub fn artifact_contract(artifact: &[u8]) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let Ok(value) = serde_json::from_slice::<Value>(artifact) else {
        return (None, None);
    };
    let schema = |key: &str| {
        value
            .get(key)
            .filter(|v| v.is_object())
            .and_then(|v| serde_json::to_vec(v).ok())
    };
    (schema("inputSchema"), schema("outputSchema"))
}

/// The rules an artifact was built under.
#[must_use]
pub fn artifact_policy(artifact: &[u8]) -> Option<Policy> {
    let value: Value = serde_json::from_slice(artifact).ok()?;
    Policy::parse_canonical(value.get("policy")?.as_str()?).ok()
}

fn parse_artifact(
    bytes: &[u8],
    language: Language,
) -> Option<Vec<sagat_infra_schema::bundle::BundleFile>> {
    sagat_infra_schema::bundle::decoded_files(bytes, language)
}

impl Sandbox for DockerSandbox {
    async fn toolchain(&self, language: Language) -> Result<String, BuildFailure> {
        let build_image = self
            .build_image_of(language)
            .ok_or(BuildFailure::Unsupported)?;
        let build = self
            .ensure_image(build_image)
            .await
            .map_err(|()| BuildFailure::Unavailable)?;
        if language == Language::CSharp {
            let runtime_image = self
                .runtime_image_of(language)
                .ok_or(BuildFailure::Unsupported)?;
            let runtime = self
                .ensure_image(runtime_image)
                .await
                .map_err(|()| BuildFailure::Unavailable)?;
            Ok(format!(
                "build={build};runtime={runtime};protocol=artifact-v2"
            ))
        } else {
            Ok(format!("{build};shim={}", shim_version()))
        }
    }

    async fn build(&self, request: BuildRequest) -> Result<Built, BuildFailure> {
        let language = request.language;
        let image = self
            .build_image_of(language)
            .ok_or(BuildFailure::Unsupported)?
            .to_owned();
        let packages = lockfile::approved(language, request.lockfile.as_ref())
            .map_err(BuildFailure::Lockfile)?;
        let toolchain = self.toolchain(language).await?;
        let command = match language {
            Language::TypeScript => vec![
                "node",
                "--no-warnings",
                "--input-type=module",
                "-e",
                BUILD_MJS,
            ],
            Language::Python => vec!["python", "-I", "-B", "-c", BUILD_PY],
            Language::CSharp => vec!["dotnet", "/opt/sagat/build/Sagat.DotnetBuild.dll"],
            _ => return Err(BuildFailure::Unsupported),
        };
        let job = Job {
            image,
            command: command.into_iter().map(str::to_owned).collect(),
            first_message: protocol::build_message(
                request.source.as_str(),
                request.lockfile.as_ref().map(|lockfile| lockfile.as_str()),
                &packages,
            ),
            // Reading a script takes a fraction of what running it may: a build gets at most the
            // default limits, whatever the policy of the script allows it later.
            limits: request.policy.within(&Policy::default()).limits,
            max_line: if language == Language::CSharp {
                MAX_BUILD_LINE
            } else {
                MAX_LINE
            },
        };
        let driven = self.drive::<NoHost>(job, None).await;
        if driven.unavailable {
            return Err(BuildFailure::Unavailable);
        }
        let (log, _) = shown_log(
            &driven.log,
            &[],
            request.policy.limits.output_bytes(),
            driven.log_truncated,
        );
        match driven.message {
            Some(Message::Built { entry, files }) => {
                let artifact = artifact_json(
                    language,
                    &request.policy,
                    (
                        request.input_schema.as_deref(),
                        request.output_schema.as_deref(),
                    ),
                    &entry,
                    &files,
                );
                let package_components: Vec<Value> = packages
                    .iter()
                    .flat_map(|package| {
                        [
                            json!({
                                "type": "library", "name": package.id, "version": package.version,
                                "hashes": [{"alg": "SHA-512", "content": package.archive_sha512}],
                                "properties": [{
                                    "name": "nuget:contentHash", "value": package.content_hash
                                }],
                            }),
                            json!({
                                "type": "file", "name": format!("{}.dll", package.id),
                                "version": package.version,
                                "hashes": [{"alg": "SHA-256", "content": package.assembly_sha256}],
                            }),
                        ]
                    })
                    .collect();
                let mut components = vec![json!({
                    "type": "platform", "name": language.as_str(), "version": toolchain,
                })];
                components.extend(package_components);
                let sbom = serde_json::to_vec(&json!({
                    "bomFormat": "CycloneDX", "specVersion": "1.5", "version": 1,
                    "metadata": {"component": {"type": "application", "name": "script", "version": "1"}},
                    "components": components,
                }))
                .unwrap_or_default();
                Ok(Built {
                    artifact,
                    sbom,
                    toolchain,
                    log,
                })
            }
            Some(Message::Rejected(message)) => Err(BuildFailure::Rejected(redact(&message, &[]))),
            _ if driven.timed_out || driven.oom || driven.flooded => Err(BuildFailure::Limit),
            _ => {
                let first = log.lines().next().unwrap_or_default();
                Err(BuildFailure::Rejected(if first.is_empty() {
                    "the build produced nothing".to_owned()
                } else {
                    format!("the build produced nothing: {first}")
                }))
            }
        }
    }

    async fn run<H: HostCalls>(&self, request: RunRequest, host: &H) -> RunOutcome {
        let language = request.language;
        let failed = |failure: Failure, log: String| RunOutcome {
            result: Err(failure),
            log,
            log_truncated: false,
            denied: Vec::new(),
            exit_code: None,
            wall: Duration::ZERO,
            memory_peak_bytes: None,
            start: Duration::ZERO,
            inferred_input_schema: None,
            inferred_output_schema: None,
        };
        let Some(image) = self.runtime_image_of(language).map(str::to_owned) else {
            return failed(
                Failure::Unavailable,
                "no runner for this language".to_owned(),
            );
        };
        let Some(files) = parse_artifact(&request.artifact, language) else {
            return failed(
                Failure::Unavailable,
                "the artifact is not one this runner can load".to_owned(),
            );
        };
        let Ok(input) = serde_json::from_slice::<Value>(&request.input) else {
            return failed(Failure::SchemaViolation, "the input is not JSON".to_owned());
        };
        if let Some(declared) = &request.input_schema
            && let Ok(declared) = serde_json::from_slice::<Value>(declared)
            && let Err(why) = schema::validate(&declared, &input)
        {
            return failed(
                Failure::SchemaViolation,
                format!("the input does not match its schema: {why}"),
            );
        }
        let command = match language {
            Language::TypeScript => vec![
                "node",
                "--no-warnings",
                "--input-type=module",
                "-e",
                RUNTIME_MJS,
            ],
            Language::Python => vec!["python", "-I", "-B", "-c", RUNTIME_PY],
            Language::CSharp => vec!["dotnet", "/opt/sagat/runtime/Sagat.DotnetRuntime.dll"],
            _ => {
                return failed(
                    Failure::Unavailable,
                    "no runner for this language".to_owned(),
                );
            }
        };
        if self.ensure_image(&image).await.is_err() {
            return failed(
                Failure::Unavailable,
                "the runtime image is not available".to_owned(),
            );
        }
        let job = Job {
            image,
            command: command.into_iter().map(str::to_owned).collect(),
            first_message: if language == Language::CSharp {
                protocol::run_binary_message(&input, &files, request.idempotency_key.as_deref())
            } else {
                let text: Option<Vec<(String, String)>> = files
                    .into_iter()
                    .map(|file| {
                        String::from_utf8(file.bytes)
                            .ok()
                            .map(|text| (file.name, text))
                    })
                    .collect();
                let Some(text) = text else {
                    return failed(
                        Failure::Unavailable,
                        "the artifact is not one this runner can load".to_owned(),
                    );
                };
                protocol::run_message(&input, &text, request.idempotency_key.as_deref())
            },
            limits: request.policy.limits,
            max_line: MAX_LINE,
        };
        let driven = self.drive(job, Some(host)).await;

        let mut hide: Vec<&str> = request.secrets_to_hide.iter().map(String::as_str).collect();
        hide.extend(driven.handed.iter().map(String::as_str));
        let keep = request.policy.limits.output_bytes();
        let (log, log_truncated) = shown_log(&driven.log, &hide, keep, driven.log_truncated);
        let denied: Vec<String> = driven.denied.iter().map(|d| redact(d, &hide)).collect();
        let result = if driven.unavailable {
            Err(Failure::Unavailable)
        } else if driven.timed_out {
            Err(Failure::Timeout)
        } else if driven.flooded {
            // The supervisor stopped the script for what it wrote; that the kernel ended it a moment
            // later, for the memory its unread output took, is not the reason it was stopped.
            Err(Failure::OutputLimit)
        } else if driven.oom {
            Err(Failure::OutOfMemory)
        } else {
            match &driven.message {
                Some(Message::Result(value)) => {
                    let bytes = serde_json::to_vec(value).unwrap_or_default();
                    if bytes.len() > MAX_RESULT_BYTES {
                        Err(Failure::OutputLimit)
                    } else if let Some(declared) = &request.output_schema
                        && let Ok(declared) = serde_json::from_slice::<Value>(declared)
                        && schema::validate(&declared, value).is_err()
                    {
                        Err(Failure::SchemaViolation)
                    } else {
                        // A secret in the result is kept out of it. It is taken out of the strings of
                        // the value, not of its text: a secret with a quote or a backslash in it is
                        // written differently in the text, and the result stays valid JSON.
                        let shown = redact_value(value, &hide);
                        Ok(serde_json::to_vec(&shown).unwrap_or(bytes))
                    }
                }
                Some(Message::Error { message, stack }) => {
                    let mut text = redact(message, &hide);
                    if !stack.is_empty() {
                        text.push('\n');
                        text.push_str(&redact(stack, &hide));
                    }
                    return RunOutcome {
                        result: Err(if !driven.denied.is_empty() && text.contains("denied") {
                            Failure::Denied
                        } else {
                            Failure::Error
                        }),
                        log: join_log(log, &text, keep),
                        log_truncated,
                        denied,
                        exit_code: driven.exit_code,
                        wall: driven.wall,
                        memory_peak_bytes: driven.memory_peak,
                        start: driven.start,
                        inferred_input_schema: None,
                        inferred_output_schema: None,
                    };
                }
                _ => Err(Failure::NoResult),
            }
        };
        let (inferred_input, inferred_output) = match &result {
            Ok(bytes) => (
                request
                    .input_schema
                    .is_none()
                    .then(|| schema::infer(&input)),
                request
                    .output_schema
                    .is_none()
                    .then(|| {
                        serde_json::from_slice::<Value>(bytes)
                            .map(|v| schema::infer(&v))
                            .ok()
                    })
                    .flatten(),
            ),
            Err(_) => (None, None),
        };
        RunOutcome {
            result,
            log,
            log_truncated,
            denied,
            exit_code: driven.exit_code,
            wall: driven.wall,
            memory_peak_bytes: driven.memory_peak,
            start: driven.start,
            inferred_input_schema: inferred_input.and_then(|v| serde_json::to_vec(&v).ok()),
            inferred_output_schema: inferred_output.and_then(|v| serde_json::to_vec(&v).ok()),
        }
    }
}

/// The log of a run as it may be kept: what the run was handed and what looks like a credential
/// taken out first and the cut made after, so a secret on the edge of the cut is not shown by its
/// first half. Also says whether anything was cut.
fn shown_log(raw: &[u8], hide: &[&str], keep: usize, truncated: bool) -> (String, bool) {
    let mut text = redact(&String::from_utf8_lossy(raw), hide);
    let over = text.len() > keep;
    if over {
        let mut cut = keep;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    (text, truncated || over)
}

/// `value` with every string, and every key, redacted.
fn redact_value(value: &Value, hide: &[&str]) -> Value {
    match value {
        Value::String(text) => Value::String(redact(text, hide)),
        Value::Array(items) => Value::Array(items.iter().map(|i| redact_value(i, hide)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (redact(k, hide), redact_value(v, hide)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn join_log(log: String, error: &str, max: usize) -> String {
    let mut text = log;
    if !error.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(error);
    }
    if text.len() > max {
        let mut cut = max;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    text
}

/// A builder asks nothing of the outside world.
struct NoHost;

impl HostCalls for NoHost {
    async fn fetch(
        &self,
        _: HttpRequest,
    ) -> Result<sagat_application::sandbox::HttpReply, Refusal> {
        Err(Refusal::Denied)
    }

    async fn secret(&self, _: &str) -> Result<String, Refusal> {
        Err(Refusal::Denied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REDACTED_STR: &str = sagat_domain::scripts::REDACTED;

    #[test]
    fn only_the_host_of_a_refused_request_is_recorded() {
        assert_eq!(
            destination("http://169.254.169.254/latest/meta-data/iam?token=abc"),
            "http://169.254.169.254"
        );
        assert_eq!(
            destination("https://api.example.com:8443/x/secret-path"),
            "https://api.example.com:8443"
        );
        assert_eq!(destination("https://host"), "https://host");
        assert_eq!(destination("not a url"), "not a url");
    }

    #[test]
    fn the_credentials_in_the_address_of_a_refused_request_are_never_recorded() {
        for (url, shown) in [
            (
                "https://svc:s3cr3t@api.example.com/x",
                "https://api.example.com",
            ),
            (
                "https://svc:s3cr3t@api.example.com:8443?a=b",
                "https://api.example.com:8443",
            ),
            ("http://a:b@c@host/p", "http://host"),
            ("https://user@host#frag", "https://host"),
        ] {
            assert_eq!(destination(url), shown, "{url}");
        }
        assert!(destination(&format!("https://{}", "a".repeat(5000))).len() <= MAX_DENIED_LEN);
    }

    #[test]
    fn a_refusal_is_kept_short_and_the_list_of_them_is_bounded() {
        let mut denied = Vec::new();
        deny(&mut denied, format!("secret {}", "x".repeat(250_000)));
        assert_eq!(denied[0].chars().count(), MAX_DENIED_LEN);
        for i in 0..100 {
            deny(&mut denied, format!("secret n{i}"));
        }
        assert_eq!(denied.len(), MAX_DENIED);
    }

    #[test]
    fn a_secret_is_taken_out_of_the_strings_of_a_result_however_json_writes_it() {
        let secret = "pa\"ss\\word\nnext";
        let value = json!({"echo": secret, "list": ["x", secret], "k": {secret: 1}, "n": 424_242});
        let shown = redact_value(&value, &[secret, "424242"]);
        let text = shown.to_string();
        assert!(!text.contains("pa\\\"ss"), "{text}");
        assert!(!text.contains("word"), "{text}");
        assert_eq!(shown["echo"], REDACTED_STR);
        assert_eq!(shown["list"][1], REDACTED_STR);
        assert_eq!(
            shown["n"], 424_242,
            "only text is searched, so the value stays valid JSON"
        );
        serde_json::from_str::<Value>(&text).expect("still JSON");
    }

    #[test]
    fn a_secret_on_the_edge_of_the_cut_is_not_shown_by_its_first_half() {
        let secret = "sk_live_0123456789abcdef";
        let raw = format!("{}{secret} and more", "y".repeat(100 - 8));
        let (log, truncated) = shown_log(raw.as_bytes(), &[secret], 100, false);
        assert!(truncated);
        assert!(!log.contains("sk_live"), "{log}");
        assert!(log.len() <= 100);
    }

    #[test]
    fn a_pem_block_without_an_end_does_not_take_the_json_around_it_with_it() {
        let value = json!({"a": "-----BEGIN PRIVATE KEY-----\nabc", "b": 1});
        let shown = redact_value(&value, &[]);
        assert_eq!(shown["b"], 1);
        serde_json::from_str::<Value>(&shown.to_string()).expect("still JSON");
    }

    #[test]
    fn an_artifact_names_only_plain_files_and_only_its_own_language() {
        let ok = artifact_json(
            Language::TypeScript,
            &Policy::default(),
            (None, None),
            "main.mjs",
            &[("main.mjs".to_owned(), "1".to_owned())],
        );
        assert!(parse_artifact(&ok, Language::TypeScript).is_some());
        assert_eq!(artifact_policy(&ok), Some(Policy::default()));
        let tighter = Policy {
            limits: Limits::new(
                64 * 1024 * 1024,
                500,
                std::time::Duration::from_secs(5),
                16,
                4096,
                1024 * 1024,
            )
            .expect("limits"),
            ..Policy::default()
        };
        let other = artifact_json(
            Language::TypeScript,
            &tighter,
            (Some(br#"{"type":"object"}"#), Some(br#"{"type":"string"}"#)),
            "main.mjs",
            &[("main.mjs".to_owned(), "1".to_owned())],
        );
        assert_ne!(
            ok, other,
            "the same code under other limits is another artifact"
        );
        assert_eq!(artifact_policy(&other), Some(tighter));
        assert_eq!(
            artifact_contract(&other),
            (
                Some(br#"{"type":"object"}"#.to_vec()),
                Some(br#"{"type":"string"}"#.to_vec())
            )
        );
        assert_eq!(artifact_contract(&ok), (None, None));
        assert_eq!(artifact_policy(b"not json"), None);
        assert_eq!(
            artifact_policy(br#"{"v":1,"policy":"v1;memory=1"}"#),
            None,
            "a policy that does not hold is no policy"
        );
        assert!(
            parse_artifact(&ok, Language::Python).is_none(),
            "another language's artifact"
        );
        for bad in [
            br#"{"v":2,"language":"typescript","entry":"a","files":{"a":"1"}}"#.as_slice(),
            br#"{"v":1,"language":"typescript","entry":"a","files":{}}"#.as_slice(),
            br#"{"v":1,"language":"typescript","entry":"a","files":{"../etc/passwd":"1"}}"#
                .as_slice(),
            br#"{"v":1,"language":"typescript","entry":"a","files":{"a/b":"1"}}"#.as_slice(),
            br#"{"v":1,"language":"typescript","entry":"a","files":{".hidden":"1"}}"#.as_slice(),
            b"not json".as_slice(),
        ] {
            assert!(
                parse_artifact(bad, Language::TypeScript).is_none(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn the_shims_are_part_of_the_toolchain_identity() {
        let version = shim_version();
        assert_eq!(version.len(), 16);
        assert_eq!(version, shim_version(), "stable for the same files");
    }

    #[test]
    fn a_log_with_the_error_appended_stays_within_its_bound_on_a_character() {
        let joined = join_log("héllo".to_owned(), "wörld ✓ more text", 12);
        assert!(joined.len() <= 12);
        assert!(joined.is_char_boundary(joined.len()));
    }
}
