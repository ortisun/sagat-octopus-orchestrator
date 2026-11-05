//! The `lake` container profile (feature 035, D035-048, T035-028).
//!
//! One container per attempt of an Iceberg write, an object-storage write, a probe or a browse. It runs the pinned
//! Python host of `runners/lake` and nothing else, and it is the credential-free side of the lake runtime: the
//! credentials, the record files and the only network route belong to the lake runner (contracts/lake-runtime.md).
//!
//! The container is created with: no network interface but loopback, a read-only root, a small writable `/tmp` that
//! cannot run code, every Linux capability dropped, `no-new-privileges`, the engine's default seccomp profile, a user
//! of its own per slot, a private PID and IPC namespace, a memory limit with swap off, a CPU quota, a limit on
//! processes, on file size and on open files, no core dumps, no log driver and **one** mount: its own attempt
//! directory, taken as a sub-path of the work volume, so that another attempt's directory is not there to be read. A
//! start carries no credential: the host finds the relay socket and its token in the attempt directory, where the
//! lake runner put them.
//!
//! What this cannot say is how strong the runtime under it is (`runc` here; a stronger runtime is the engine's
//! configuration, as for the script sandbox in [`crate::docker`]), and whether the engine gives the container a user
//! namespace of its own: that is a daemon setting (`userns-remap`, rootless mode, gVisor), not a property of one
//! container, so the deployment states it and the isolation tests of T035-015 show what holds on the engine they run
//! against (G035-022).

pub mod engine;

use std::collections::HashMap;

use bollard::models::{
    ContainerCreateBody, HostConfig, HostConfigLogConfig, Mount, MountTypeEnum, MountVolumeOptions,
    ResourcesUlimits,
};

/// How many attempts can run side by side on one node, at most; a slot is a user id.
pub const MAX_SLOTS: u32 = 4096;
/// The user id of slot 0. Well above the range of system accounts, and no slot is root.
const FIRST_UID: u32 = 40_000;
/// Where an attempt finds its directory.
const WORK_ROOT: &str = "/work";
/// The host program, in the runtime image (`runners/lake/Dockerfile`).
const HOST_PROGRAM: &str = "/opt/sagat/lake/sagat_lake/host.py";
/// A label on every container this module makes: the sweep and the per-node ceiling look for it.
pub const LABEL: &str = "sagat.lake";
/// Which lake runner made a container.
pub const OWNER_LABEL: &str = "sagat.lake.owner";
/// The attempt a container belongs to.
pub const ATTEMPT_LABEL: &str = "sagat.lake.attempt";
const MAX_OPEN_FILES: i64 = 1024;

/// What the profile refuses. The reason names the field, never its value: a refused value may be anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LakeProfileError {
    /// The attempt id is not 1 to 64 lower-case hexadecimal digits and hyphens.
    AttemptId,
    /// The slot is past the last one.
    Slot,
    /// A limit is zero, and zero is not unlimited.
    Limits,
    /// An image, volume or owner name is empty, too long or carries a character it must not.
    Name,
}

impl std::fmt::Display for LakeProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AttemptId => {
                "the attempt id is not 1 to 64 lower-case hexadecimal digits and hyphens"
            }
            Self::Slot => "the slot is past the last one",
            Self::Limits => "a limit of the lake container is zero",
            Self::Name => "an image, volume or owner name is not acceptable",
        })
    }
}

impl std::error::Error for LakeProfileError {}

/// The budget of one attempt's container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LakeLimits {
    /// `memory.max`; swap is off.
    pub memory_bytes: u64,
    /// `pids.max`.
    pub processes: u32,
    /// The CPU quota, in thousandths of a CPU.
    pub cpu_millis: u32,
    /// The size of the attempt directory, and the largest file a process may write.
    pub work_bytes: u64,
    /// The size of `/tmp`.
    pub tmp_bytes: u64,
}

/// The work volume. It is created by the deployment, with a size limit, and mounted in the lake runner too: the runner
/// writes the inputs, the relay socket and the token into `<attempt id>/` before it asks for the container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptMount {
    /// The name of the volume.
    pub volume: String,
}

/// One container to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LakeSpec {
    /// The runtime image, by digest in a deployment: a tag moves, a digest does not.
    pub image: String,
    /// The attempt, which is also the name of its directory.
    pub attempt_id: String,
    /// The slot of the attempt on this node; it selects the user id.
    pub slot: u32,
    /// The work volume.
    pub mount: AttemptMount,
    /// The budget.
    pub limits: LakeLimits,
    /// Which lake runner asks.
    pub owner: String,
}

/// The user id of a slot, or `None` past the last slot.
#[must_use]
pub const fn uid_for_slot(slot: u32) -> Option<u32> {
    if slot < MAX_SLOTS {
        Some(FIRST_UID + slot)
    } else {
        None
    }
}

/// The command that starts the host on an attempt directory.
#[must_use]
pub fn host_command(attempt_id: &str) -> Vec<String> {
    ["python3", "-I", "-X", "utf8", HOST_PROGRAM, "--attempt-dir"]
        .into_iter()
        .map(str::to_owned)
        .chain([format!("{WORK_ROOT}/{attempt_id}")])
        .collect()
}

fn attempt_id_ok(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
}

fn plain(name: &str, longest: usize) -> bool {
    !name.is_empty()
        && name.len() <= longest
        && !name.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// A volume name as the engine accepts it: it starts with a letter or digit and goes on with those, `_`, `.` and `-`.
fn volume_name_ok(name: &str) -> bool {
    plain(name, 255)
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// What the engine is asked to create for `spec`.
///
/// # Errors
/// [`LakeProfileError`] for an attempt id that could leave the work directory, a slot past the last, a zero limit or a
/// name that is empty or carries a control character.
pub fn container_config(spec: &LakeSpec) -> Result<ContainerCreateBody, LakeProfileError> {
    if !attempt_id_ok(&spec.attempt_id) {
        return Err(LakeProfileError::AttemptId);
    }
    let uid = uid_for_slot(spec.slot).ok_or(LakeProfileError::Slot)?;
    let limits = &spec.limits;
    if limits.memory_bytes == 0
        || limits.processes == 0
        || limits.cpu_millis == 0
        || limits.work_bytes == 0
        || limits.tmp_bytes == 0
    {
        return Err(LakeProfileError::Limits);
    }
    if !plain(&spec.image, 512) || !volume_name_ok(&spec.mount.volume) || !plain(&spec.owner, 128) {
        return Err(LakeProfileError::Name);
    }
    let clamp = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    let memory = clamp(limits.memory_bytes);
    let work = clamp(limits.work_bytes);
    let ulimit = |name: &str, value: i64| ResourcesUlimits {
        name: Some(name.to_owned()),
        soft: Some(value),
        hard: Some(value),
    };
    let directory = format!("{WORK_ROOT}/{}", spec.attempt_id);
    Ok(ContainerCreateBody {
        image: Some(spec.image.clone()),
        cmd: Some(host_command(&spec.attempt_id)),
        working_dir: Some("/tmp".to_owned()),
        user: Some(format!("{uid}:{uid}")),
        env: Some(vec![
            "PATH=/usr/local/bin:/usr/bin:/bin".to_owned(),
            "HOME=/tmp".to_owned(),
            "TMPDIR=/tmp".to_owned(),
            "AWS_EC2_METADATA_DISABLED=true".to_owned(),
        ]),
        labels: Some(HashMap::from([
            (LABEL.to_owned(), "1".to_owned()),
            (OWNER_LABEL.to_owned(), spec.owner.clone()),
            (ATTEMPT_LABEL.to_owned(), spec.attempt_id.clone()),
        ])),
        network_disabled: Some(true),
        tty: Some(false),
        host_config: Some(HostConfig {
            network_mode: Some("none".to_owned()),
            readonly_rootfs: Some(true),
            cap_drop: Some(vec!["ALL".to_owned()]),
            security_opt: Some(vec!["no-new-privileges".to_owned()]),
            privileged: Some(false),
            memory: Some(memory),
            memory_swap: Some(memory),
            nano_cpus: Some(i64::from(limits.cpu_millis) * 1_000_000),
            pids_limit: Some(i64::from(limits.processes)),
            tmpfs: Some(HashMap::from([(
                "/tmp".to_owned(),
                format!("rw,noexec,nosuid,nodev,size={}", limits.tmp_bytes),
            )])),
            mounts: Some(vec![Mount {
                target: Some(directory),
                source: Some(spec.mount.volume.clone()),
                typ: Some(MountTypeEnum::VOLUME),
                read_only: Some(false),
                volume_options: Some(MountVolumeOptions {
                    no_copy: Some(true),
                    subpath: Some(spec.attempt_id.clone()),
                    ..MountVolumeOptions::default()
                }),
                ..Mount::default()
            }]),
            ulimits: Some(vec![
                ulimit("nofile", MAX_OPEN_FILES),
                ulimit("core", 0),
                ulimit("fsize", work),
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
    })
}
