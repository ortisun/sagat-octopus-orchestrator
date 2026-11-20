//! Settings of lake mode, read from environment variables like the script mode's (feature 035, ADR-0079 rules 1, 3, 5, 6 and 8).
//!
//! Lake mode serves only lake containers and holds no token of the control plane, so a process that is given any setting of the
//! script mode refuses to start rather than quietly being both. The peer user is required, never defaulted, and the ids that
//! would make the socket serve more than the lake runner are refused. The limits are the ones of the plan (`plan.md`, the table
//! of limits): a default that is what a node should run with, and a cap that catches the mistake of a unit.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use sagat_control_plane::config::{ConfigError, Environment, Lookup, optional, required};
use sagat_infra_lake::sandbox_api::{Class, FIRST_UID, MAX_SLOTS};
use sagat_infra_sandbox::lake::{image_ok, owner_ok, volume_name_ok};

/// The socket in the socket directory.
pub const SOCKET_NAME: &str = "lake-sandbox.sock";
/// The lock in the socket directory that keeps one process to it.
pub const LOCK_NAME: &str = ".lock";
/// The id the kernel shows for a user it cannot map into the namespace of the reader: never a peer to allow.
const OVERFLOW_UID: u32 = 65_534;
const MIB: u64 = 1024 * 1024;

/// The caps of the plan's limit table, and of the units: an operator who writes bytes where MiB is meant gets a refusal, not a
/// container with a petabyte of memory.
const MAX_CEILING: usize = 8;
const MAX_ATTEMPT_SECS: u64 = 3600;
const MAX_CLASS_MEMORY_MIB: u64 = 16 * 1024;
const MAX_NODE_MEMORY_MIB: u64 = 4 * 1024 * 1024;
const MAX_PROCESSES: u32 = 4096;
const MAX_CPU_MILLIS: u32 = 64_000;
const MAX_WORK_MIB: u64 = 64 * 1024;
const MAX_TMP_MIB: u64 = 4096;

/// The prefix of every setting of the script mode. A lake-mode process runs no build or script step and has no token of the
/// control plane (ADR-0079, rule 1), so any of them is a mistake in the deployment, not something to ignore.
pub const SCRIPT_MODE_PREFIX: &str = "SAGAT_RUNNER_";

/// The first variable of the environment that belongs to the script mode, if there is one.
#[must_use]
pub fn script_mode_variable(names: impl IntoIterator<Item = String>) -> Option<String> {
    names
        .into_iter()
        .find(|name| name.starts_with(SCRIPT_MODE_PREFIX))
}

/// What one budget class gives a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub memory_bytes: u64,
    pub processes: u32,
    pub cpu_millis: u32,
    /// The largest file a process may write (not a quota on the attempt directory: the volume's size is that).
    pub work_bytes: u64,
    pub tmp_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct LakeConfig {
    /// The directory of the socket. The sandbox runner owns it (mode 0750), and the lake runner's group may search it.
    pub socket_dir: PathBuf,
    /// The one user the listener serves.
    pub peer_uid: u32,
    /// The group of the lake runner, which owns the files in an attempt directory and lets a container read them.
    pub group: u32,
    /// The work volume, by its name for the engine, and where this process sees it (read only: it only looks).
    pub work_volume: String,
    pub work_root: PathBuf,
    pub image: String,
    /// The label on every container: which lake runner they are for. Required, because the name of a container does not carry
    /// it: two lake deployments on one engine with the same owner would count and sweep each other's containers.
    pub owner: String,
    pub small: Budget,
    /// `None` when the node has no `large` class.
    pub large: Option<Budget>,
    /// Containers at once on the node.
    pub ceiling: usize,
    /// What the reserved budgets of the live containers may add up to.
    pub node_memory_bytes: u64,
    /// The longest an attempt may ask for. A longer one is refused, never cut.
    pub max_attempt: Duration,
    /// Added to an attempt's deadline for the label that ends the container: the time the lake runner has to end it first.
    pub deadline_grace: Duration,
    /// How long an ended container is kept, after its deadline, so that what the engine said about it can still be read.
    pub retain: Duration,
    /// How often the labels are read for what is past its deadline. This is the deadline's only clock (ADR-0079, rule 8), so it
    /// is at most a minute.
    pub sweep_interval: Duration,
    /// How long, after the stop signal, the node answers waits and kills while it refuses new starts.
    pub drain: Duration,
    pub admin_addr: Option<SocketAddr>,
    pub environment: Environment,
}

fn number<T: std::str::FromStr>(get: Lookup<'_>, name: &'static str) -> Result<T, ConfigError>
where
    T::Err: std::fmt::Display,
{
    required(get, name)?
        .trim()
        .parse()
        .map_err(|error: T::Err| ConfigError::Invalid {
            name,
            reason: error.to_string(),
        })
}

fn invalid(name: &'static str, reason: &str) -> ConfigError {
    ConfigError::Invalid {
        name,
        reason: reason.to_owned(),
    }
}

fn seconds(
    get: Lookup<'_>,
    name: &'static str,
    default: u64,
    longest: u64,
) -> Result<Duration, ConfigError> {
    let value: u64 = optional(get, name, default)?;
    if value == 0 || value > longest {
        return Err(invalid(name, &format!("is 1 to {longest} seconds")));
    }
    Ok(Duration::from_secs(value))
}

/// A count of something other than memory: not zero (which is not unlimited), not past `most`.
fn count(get: Lookup<'_>, name: &'static str, default: u32, most: u32) -> Result<u32, ConfigError> {
    let value: u32 = optional(get, name, default)?;
    if value == 0 {
        return Err(invalid(name, "is zero, and zero is not unlimited"));
    }
    if value > most {
        return Err(invalid(name, &format!("is above {most}")));
    }
    Ok(value)
}

/// A size in MiB, as bytes: not zero, not past `most_mib`.
fn mib(
    get: Lookup<'_>,
    name: &'static str,
    default_mib: u64,
    most_mib: u64,
) -> Result<u64, ConfigError> {
    let value: u64 = optional(get, name, default_mib)?;
    if value == 0 {
        return Err(invalid(name, "is zero, and zero is not unlimited"));
    }
    if value > most_mib {
        return Err(invalid(
            name,
            &format!("is above {most_mib} MiB: is it in bytes?"),
        ));
    }
    Ok(value * MIB)
}

/// The five limits of one class, from the variables named `prefix` + `MEMORY_MIB`, `PROCESSES`, `CPU_MILLIS`, `WORK_MIB` and
/// `TMP_MIB`.
fn budget(
    get: Lookup<'_>,
    names: [&'static str; 5],
    default: Budget,
) -> Result<Budget, ConfigError> {
    let [memory, processes, cpu, work, tmp] = names;
    Ok(Budget {
        memory_bytes: mib(
            get,
            memory,
            default.memory_bytes / MIB,
            MAX_CLASS_MEMORY_MIB,
        )?,
        processes: count(get, processes, default.processes, MAX_PROCESSES)?,
        cpu_millis: count(get, cpu, default.cpu_millis, MAX_CPU_MILLIS)?,
        work_bytes: mib(get, work, default.work_bytes / MIB, MAX_WORK_MIB)?,
        tmp_bytes: mib(get, tmp, default.tmp_bytes / MIB, MAX_TMP_MIB)?,
    })
}

impl LakeConfig {
    /// # Errors
    /// [`ConfigError`] naming the first missing or invalid variable. The settings of the script mode are refused by name in
    /// [`script_mode_variable`], which the process runs over its whole environment before this.
    pub fn from_lookup(get: Lookup<'_>) -> Result<Self, ConfigError> {
        let environment: Environment = optional(get, "SAGAT_ENV", Environment::Prod)?;
        let peer_uid: u32 = number(get, "SAGAT_LAKE_RUNNER_UID")?;
        if peer_uid == 0 || peer_uid == OVERFLOW_UID {
            return Err(invalid(
                "SAGAT_LAKE_RUNNER_UID",
                "is root's or the overflow id's, which is never the lake runner",
            ));
        }
        // a lake container runs as a user of this range: if the peer were one of them, a container could use the socket
        if (FIRST_UID..FIRST_UID + MAX_SLOTS).contains(&peer_uid) {
            return Err(invalid(
                "SAGAT_LAKE_RUNNER_UID",
                "is in the range of the users of the lake containers",
            ));
        }
        let group: u32 = number(get, "SAGAT_LAKE_RUNNER_GID")?;
        if group == 0 || group == OVERFLOW_UID {
            return Err(invalid(
                "SAGAT_LAKE_RUNNER_GID",
                "is root's or the overflow id's, which is never the group of a lake container",
            ));
        }
        let work_volume = required(get, "SAGAT_LAKE_WORK_VOLUME")?.trim().to_owned();
        if !volume_name_ok(&work_volume) {
            return Err(invalid(
                "SAGAT_LAKE_WORK_VOLUME",
                "is not a volume name the engine accepts",
            ));
        }
        let image = required(get, "SAGAT_LAKE_IMAGE")?.trim().to_owned();
        if !image_ok(&image) {
            return Err(invalid("SAGAT_LAKE_IMAGE", "is not an image reference"));
        }
        // a tag moves, a digest does not: the host that holds the credentials is the one that was reviewed
        if environment != Environment::Dev && !image.contains("@sha256:") {
            return Err(invalid(
                "SAGAT_LAKE_IMAGE",
                "must name the image by digest (name@sha256:...) outside SAGAT_ENV=dev",
            ));
        }
        let owner = required(get, "SAGAT_LAKE_OWNER")?.trim().to_owned();
        if !owner_ok(&owner) {
            return Err(invalid(
                "SAGAT_LAKE_OWNER",
                "is 1 to 128 letters, digits, '.', '_' and '-'",
            ));
        }
        let ceiling: usize = optional(get, "SAGAT_LAKE_CEILING", 2)?;
        if ceiling == 0 || ceiling > MAX_CEILING {
            return Err(invalid(
                "SAGAT_LAKE_CEILING",
                &format!("is 1 to {MAX_CEILING}, the most lake containers a node runs"),
            ));
        }
        let small_default = Budget {
            memory_bytes: 2048 * MIB,
            processes: 128,
            cpu_millis: 1500,
            work_bytes: 512 * MIB,
            tmp_bytes: 64 * MIB,
        };
        let small = budget(
            get,
            [
                "SAGAT_LAKE_SMALL_MEMORY_MIB",
                "SAGAT_LAKE_SMALL_PROCESSES",
                "SAGAT_LAKE_SMALL_CPU_MILLIS",
                "SAGAT_LAKE_SMALL_WORK_MIB",
                "SAGAT_LAKE_SMALL_TMP_MIB",
            ],
            small_default,
        )?;
        // the class exists only on a node that says how much memory it gives; a write of up to 2 GiB of objects needs a file
        // limit of that size, so its default is not the small class's
        let large = if get("SAGAT_LAKE_LARGE_MEMORY_MIB").is_some_and(|v| !v.trim().is_empty()) {
            let large_default = Budget {
                memory_bytes: number::<u64>(get, "SAGAT_LAKE_LARGE_MEMORY_MIB")?
                    .saturating_mul(MIB),
                work_bytes: 2048 * MIB,
                ..small_default
            };
            Some(budget(
                get,
                [
                    "SAGAT_LAKE_LARGE_MEMORY_MIB",
                    "SAGAT_LAKE_LARGE_PROCESSES",
                    "SAGAT_LAKE_LARGE_CPU_MILLIS",
                    "SAGAT_LAKE_LARGE_WORK_MIB",
                    "SAGAT_LAKE_LARGE_TMP_MIB",
                ],
                large_default,
            )?)
        } else {
            None
        };
        // required: a node says how much memory it has, and nothing is assumed
        let node_memory_mib: u64 = number(get, "SAGAT_LAKE_NODE_MEMORY_MIB")?;
        if node_memory_mib == 0 {
            return Err(invalid(
                "SAGAT_LAKE_NODE_MEMORY_MIB",
                "is zero, and zero is not unlimited",
            ));
        }
        if node_memory_mib > MAX_NODE_MEMORY_MIB {
            return Err(invalid(
                "SAGAT_LAKE_NODE_MEMORY_MIB",
                &format!("is above {MAX_NODE_MEMORY_MIB} MiB: is it in bytes?"),
            ));
        }
        let node_memory_bytes = node_memory_mib * MIB;
        // a class no container of which could ever start is a node that was sized wrongly
        let largest = large.map_or(small.memory_bytes, |large| {
            large.memory_bytes.max(small.memory_bytes)
        });
        if node_memory_bytes < largest {
            return Err(invalid(
                "SAGAT_LAKE_NODE_MEMORY_MIB",
                "is less than the memory of the largest class: no container of it could start",
            ));
        }
        let admin_addr =
            match get("SAGAT_ADMIN_ADDR").as_deref().map(str::trim) {
                None | Some("") => Some(SocketAddr::from(([127, 0, 0, 1], 9092))),
                Some("off") => None,
                Some(other) => Some(other.parse().map_err(|_| {
                    invalid("SAGAT_ADMIN_ADDR", "is not a socket address or 'off'")
                })?),
            };
        Ok(Self {
            socket_dir: PathBuf::from(required(get, "SAGAT_LAKE_SOCKET_DIR")?.trim()),
            peer_uid,
            group,
            work_volume,
            work_root: PathBuf::from(required(get, "SAGAT_LAKE_WORK_ROOT")?.trim()),
            image,
            owner,
            small,
            large,
            ceiling,
            node_memory_bytes,
            max_attempt: seconds(get, "SAGAT_LAKE_MAX_ATTEMPT_SECS", 600, MAX_ATTEMPT_SECS)?,
            deadline_grace: seconds(get, "SAGAT_LAKE_DEADLINE_GRACE_SECS", 30, 3600)?,
            retain: seconds(get, "SAGAT_LAKE_RETAIN_SECS", 300, 86_400)?,
            sweep_interval: seconds(get, "SAGAT_LAKE_SWEEP_SECS", 30, 60)?,
            drain: seconds(get, "SAGAT_LAKE_DRAIN_SECS", 30, 600)?,
            admin_addr,
            environment,
        })
    }

    /// The budget of a class, or `None` when this node does not have it.
    #[must_use]
    pub const fn budget(&self, class: Class) -> Option<Budget> {
        match class {
            Class::Small => Some(self.small),
            Class::Large => self.large,
        }
    }

    #[must_use]
    pub fn socket_path(&self) -> PathBuf {
        self.socket_dir.join(SOCKET_NAME)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn base() -> HashMap<&'static str, &'static str> {
        HashMap::from([
            ("SAGAT_LAKE_SOCKET_DIR", "/run/lake"),
            ("SAGAT_LAKE_RUNNER_UID", "10001"),
            ("SAGAT_LAKE_RUNNER_GID", "10001"),
            ("SAGAT_LAKE_WORK_VOLUME", "sagat-lake-work"),
            ("SAGAT_LAKE_WORK_ROOT", "/work"),
            ("SAGAT_LAKE_IMAGE", "sagat/lake-runtime@sha256:abc"),
            ("SAGAT_LAKE_OWNER", "lake-runner-1"),
            ("SAGAT_LAKE_NODE_MEMORY_MIB", "8192"),
        ])
    }

    fn read(env: &HashMap<&'static str, &'static str>) -> Result<LakeConfig, ConfigError> {
        let lookup = |name: &str| env.get(name).map(|value| (*value).to_owned());
        LakeConfig::from_lookup(&lookup)
    }

    fn with(name: &'static str, value: &'static str) -> HashMap<&'static str, &'static str> {
        let mut env = base();
        env.insert(name, value);
        env
    }

    fn rejected(env: &HashMap<&'static str, &'static str>) -> &'static str {
        match read(env) {
            Err(ConfigError::Invalid { name, .. } | ConfigError::Missing(name)) => name,
            Ok(config) => panic!("accepted: {config:?}"),
        }
    }

    #[test]
    fn the_minimum_is_the_peer_the_volume_the_image_the_owner_and_the_memory_of_the_node() {
        let config = read(&base()).expect("a valid configuration");
        assert_eq!(config.peer_uid, 10_001);
        assert_eq!(config.owner, "lake-runner-1");
        // the defaults of the plan's table: two containers, ten minutes, two GiB
        assert_eq!(config.ceiling, 2);
        assert_eq!(config.max_attempt, Duration::from_secs(600));
        assert_eq!(config.small.memory_bytes, 2048 * MIB);
        assert_eq!(config.large, None);
        assert_eq!(
            config.socket_path(),
            PathBuf::from("/run/lake/lake-sandbox.sock")
        );
        assert_eq!(config.budget(Class::Small), Some(config.small));
        assert_eq!(config.budget(Class::Large), None);
        assert_eq!(config.environment, Environment::Prod);
    }

    #[test]
    fn each_required_setting_is_required() {
        for name in [
            "SAGAT_LAKE_SOCKET_DIR",
            "SAGAT_LAKE_RUNNER_UID",
            "SAGAT_LAKE_RUNNER_GID",
            "SAGAT_LAKE_WORK_VOLUME",
            "SAGAT_LAKE_WORK_ROOT",
            "SAGAT_LAKE_IMAGE",
            "SAGAT_LAKE_OWNER",
            "SAGAT_LAKE_NODE_MEMORY_MIB",
        ] {
            let mut env = base();
            env.remove(name);
            assert_eq!(rejected(&env), name);
            env.insert(name, "  ");
            assert_eq!(rejected(&env), name, "{name} blank");
        }
    }

    /// With no peer user the listener would serve whoever can reach the socket, so there is no default, and the ids that are
    /// never the lake runner are refused: root's, the overflow id, and any user a lake container runs as.
    #[test]
    fn the_peer_is_never_root_nor_the_overflow_id_nor_a_user_of_the_containers_nor_a_guess() {
        for bad in [
            "0",
            "65534",
            "-1",
            "ten",
            "4294967296",
            "",
            "40000",
            "40001",
            "44095",
        ] {
            assert_eq!(
                rejected(&with("SAGAT_LAKE_RUNNER_UID", bad)),
                "SAGAT_LAKE_RUNNER_UID",
                "{bad:?}"
            );
        }
        // the ends of the range of the containers' users are outside it by one
        assert!(read(&with("SAGAT_LAKE_RUNNER_UID", "39999")).is_ok());
        assert!(read(&with("SAGAT_LAKE_RUNNER_UID", "44096")).is_ok());
        for bad in ["0", "65534"] {
            assert_eq!(
                rejected(&with("SAGAT_LAKE_RUNNER_GID", bad)),
                "SAGAT_LAKE_RUNNER_GID"
            );
        }
    }

    /// Lake mode and script mode are never one process: any variable of the script mode in the environment is a refusal, by its
    /// prefix, so that a setting that is not on a list is not ignored.
    #[test]
    fn any_variable_of_the_script_mode_is_found_by_its_prefix() {
        let found = script_mode_variable(
            [
                "PATH",
                "SAGAT_LAKE_IMAGE",
                "SAGAT_RUNNER_CA_FILE",
                "SAGAT_RUNNER_TOKEN",
            ]
            .map(str::to_owned),
        );
        assert_eq!(found.as_deref(), Some("SAGAT_RUNNER_CA_FILE"));
        assert_eq!(
            script_mode_variable(
                ["PATH", "SAGAT_LAKE_IMAGE", "SAGAT_ADMIN_ADDR"].map(str::to_owned)
            ),
            None
        );
        // the prefix is of the whole name, not a part of it
        assert_eq!(
            script_mode_variable(["MY_SAGAT_RUNNER_X", "sagat_runner_x"].map(str::to_owned)),
            None
        );
    }

    #[test]
    fn names_the_engine_or_the_label_filter_would_misread_are_refused() {
        for (name, bad) in [
            ("SAGAT_LAKE_OWNER", "a,b"),
            ("SAGAT_LAKE_OWNER", "a=b"),
            ("SAGAT_LAKE_OWNER", "has space"),
            ("SAGAT_LAKE_WORK_VOLUME", "../x"),
            ("SAGAT_LAKE_WORK_VOLUME", "-x"),
            ("SAGAT_LAKE_IMAGE", "img age"),
        ] {
            assert_eq!(rejected(&with(name, bad)), name, "{bad:?}");
        }
    }

    /// A tag moves and a digest does not; outside a development environment the image is named by digest.
    #[test]
    fn the_image_is_named_by_digest_outside_development() {
        assert_eq!(
            rejected(&with("SAGAT_LAKE_IMAGE", "sagat/lake-runtime:latest")),
            "SAGAT_LAKE_IMAGE"
        );
        let mut dev = with("SAGAT_LAKE_IMAGE", "sagat/lake-runtime:latest");
        dev.insert("SAGAT_ENV", "dev");
        assert!(read(&dev).is_ok());
        let mut staging = with("SAGAT_LAKE_IMAGE", "sagat/lake-runtime:latest");
        staging.insert("SAGAT_ENV", "staging");
        assert_eq!(rejected(&staging), "SAGAT_LAKE_IMAGE");
    }

    #[test]
    fn the_ceiling_is_the_plans_default_and_never_past_its_cap() {
        assert_eq!(
            read(&with("SAGAT_LAKE_CEILING", "8"))
                .expect("the cap")
                .ceiling,
            8
        );
        for bad in ["0", "9", "4096", "-1", "many"] {
            assert_eq!(
                rejected(&with("SAGAT_LAKE_CEILING", bad)),
                "SAGAT_LAKE_CEILING",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_class_is_on_the_node_only_when_it_is_given_memory() {
        let mut env = base();
        env.insert("SAGAT_LAKE_LARGE_MEMORY_MIB", "6144");
        let config = read(&env).expect("a node with a large class");
        let large = config.large.expect("the large class");
        assert_eq!(large.memory_bytes, 6144 * MIB);
        assert_eq!(config.budget(Class::Large), Some(large));
        assert_eq!(large.processes, config.small.processes);
        // an object write of up to 2 GiB needs a file limit that big: the large class does not inherit the small one's
        assert_eq!(large.work_bytes, 2048 * MIB);
        assert!(large.work_bytes > config.small.work_bytes);
        env.insert("SAGAT_LAKE_LARGE_PROCESSES", "256");
        assert_eq!(
            read(&env).expect("set").large.expect("large").processes,
            256
        );
    }

    /// A node that cannot hold one container of its largest class was sized wrongly, and says so at start, not at the first
    /// attempt.
    #[test]
    fn a_node_smaller_than_its_largest_class_is_refused() {
        let mut env = base();
        env.insert("SAGAT_LAKE_NODE_MEMORY_MIB", "4096");
        env.insert("SAGAT_LAKE_LARGE_MEMORY_MIB", "6144");
        assert_eq!(rejected(&env), "SAGAT_LAKE_NODE_MEMORY_MIB");
        env.insert("SAGAT_LAKE_NODE_MEMORY_MIB", "6144");
        assert!(read(&env).is_ok());
        assert_eq!(
            rejected(&with("SAGAT_LAKE_NODE_MEMORY_MIB", "1024")),
            "SAGAT_LAKE_NODE_MEMORY_MIB"
        );
    }

    #[test]
    fn a_limit_of_zero_is_not_unlimited() {
        for name in [
            "SAGAT_LAKE_SMALL_MEMORY_MIB",
            "SAGAT_LAKE_SMALL_PROCESSES",
            "SAGAT_LAKE_SMALL_CPU_MILLIS",
            "SAGAT_LAKE_SMALL_WORK_MIB",
            "SAGAT_LAKE_SMALL_TMP_MIB",
            "SAGAT_LAKE_NODE_MEMORY_MIB",
        ] {
            assert_eq!(rejected(&with(name, "0")), name);
        }
    }

    /// A number written in bytes where MiB is meant would be a container with a petabyte of memory; each limit has a cap.
    #[test]
    fn a_limit_past_its_cap_is_refused_as_the_mistake_of_a_unit() {
        // 2 GiB written in bytes
        for (name, bad) in [
            ("SAGAT_LAKE_SMALL_MEMORY_MIB", "2147483648"),
            ("SAGAT_LAKE_SMALL_MEMORY_MIB", "16385"),
            ("SAGAT_LAKE_SMALL_WORK_MIB", "65537"),
            ("SAGAT_LAKE_SMALL_TMP_MIB", "4097"),
            ("SAGAT_LAKE_SMALL_PROCESSES", "4097"),
            ("SAGAT_LAKE_SMALL_CPU_MILLIS", "64001"),
            ("SAGAT_LAKE_LARGE_MEMORY_MIB", "16385"),
            ("SAGAT_LAKE_NODE_MEMORY_MIB", "4194305"),
        ] {
            let mut env = base();
            env.insert("SAGAT_LAKE_NODE_MEMORY_MIB", "8192");
            env.insert(name, bad);
            assert_eq!(rejected(&env), name, "{name}={bad}");
        }
        // and the cap itself is accepted
        let mut env = base();
        env.insert("SAGAT_LAKE_SMALL_MEMORY_MIB", "16384");
        env.insert("SAGAT_LAKE_NODE_MEMORY_MIB", "16384");
        assert!(read(&env).is_ok());
    }

    #[test]
    fn the_times_are_bounded() {
        for (name, bad) in [
            ("SAGAT_LAKE_MAX_ATTEMPT_SECS", "0"),
            ("SAGAT_LAKE_MAX_ATTEMPT_SECS", "3601"),
            ("SAGAT_LAKE_DEADLINE_GRACE_SECS", "0"),
            ("SAGAT_LAKE_RETAIN_SECS", "0"),
            ("SAGAT_LAKE_SWEEP_SECS", "0"),
            // the deadline's only clock is the sweep: it is not an hour
            ("SAGAT_LAKE_SWEEP_SECS", "61"),
            ("SAGAT_LAKE_DRAIN_SECS", "601"),
        ] {
            assert_eq!(rejected(&with(name, bad)), name, "{name}={bad}");
        }
        assert_eq!(
            read(&with("SAGAT_LAKE_MAX_ATTEMPT_SECS", "3600"))
                .expect("the cap")
                .max_attempt,
            Duration::from_secs(3600)
        );
    }

    #[test]
    fn the_admin_listener_is_local_by_default_and_can_be_off() {
        assert_eq!(
            read(&base()).expect("valid").admin_addr,
            Some(SocketAddr::from(([127, 0, 0, 1], 9092)))
        );
        assert_eq!(
            read(&with("SAGAT_ADMIN_ADDR", "off"))
                .expect("valid")
                .admin_addr,
            None
        );
        assert_eq!(
            rejected(&with("SAGAT_ADMIN_ADDR", "everywhere")),
            "SAGAT_ADMIN_ADDR"
        );
    }
}
