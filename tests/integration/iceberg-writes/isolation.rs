//! T035-015: what the `lake` container profile holds against a real container engine (D035-048, FR-035-023).
//!
//! The profile's configuration is pinned without an engine in `crates/infra-sandbox/tests/lake_profile.rs`. These
//! cases start real containers from it, with only the command replaced by a probe, and look at what a process inside
//! can and cannot do: reach the pod network or the metadata service (IPv4 and IPv6), see the engine socket, read another
//! attempt's directory or a token that belongs to another user, hold a capability or write to the root, run code from
//! `/tmp`, outgrow its memory, its scratch or its work volume, or outlive its deadline.
//!
//! Only the Docker engine and its work volume are covered. The Kubernetes co-location (a pod sharing an `emptyDir`) and
//! the Helm profile are the second half of the spike (T035-029, G035-022). A user namespace of the container's own is a
//! setting of the engine and is not asserted here.
//!
//! The image is `python:3.13-alpine` unless `SAGAT_LAKE_ISOLATION_IMAGE` names another; the probes need `sh`, `awk`,
//! `dd`, `stat` and `python3`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use sagat_infra_sandbox::lake::engine::{EngineError, Exit, LakeContainer, LakeEngine};
use sagat_infra_sandbox::lake::{
    AttemptMount, LakeLimits, LakeSpec, container_config, uid_for_slot,
};

use crate::support::bounded::{START_LIMIT, within};

const MIB: u64 = 1024 * 1024;
const TOKEN_A: &str = "token-of-attempt-a";
const TOKEN_B: &str = "token-of-attempt-b";
const SLOT_A: u32 = 11;
const SLOT_B: u32 = 12;
/// The user and the group of the lake runner (`USER 10001`): what it owns in the work volume, and the group that lets a
/// lake container read it (D035-086).
const RUNNER_UID: u32 = 10_001;
const GROUP: u32 = 10_001;
/// A group that is not the runner's.
const OTHER_GROUP: u32 = 20_000;
const PROBE_WAIT: Duration = Duration::from_secs(60);

fn image() -> String {
    std::env::var("SAGAT_LAKE_ISOLATION_IMAGE").unwrap_or_else(|_| "python:3.13-alpine".to_owned())
}

/// A deadline label far enough away that no sweep of these cases acts on it.
fn far_deadline() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
        + 24 * 3600
}

fn limits() -> LakeLimits {
    LakeLimits {
        memory_bytes: 256 * MIB,
        processes: 64,
        cpu_millis: 1000,
        work_bytes: 32 * MIB,
        tmp_bytes: 8 * MIB,
    }
}

/// Two attempts of two slots in one work volume, prepared as the lake runner would leave them: a directory and a
/// token each, owned by the user of the slot, readable by that user alone.
///
/// A memory-backed volume keeps its content only while some container has it mounted, which in a deployment is the lake
/// runner's own mount for as long as it runs. Here a holder container stands for the lake runner: it mounts the whole
/// volume, makes the directories and stays up while the probes run (found by this test: without it the directories
/// vanish when the preparer exits).
struct Work {
    engine: LakeEngine,
    holder: LakeContainer,
    volume: String,
    /// Its own label, so that one test's sweep never removes another test's containers.
    owner: String,
    attempt_a: String,
    attempt_b: String,
}

impl Work {
    async fn new() -> Self {
        let engine =
            LakeEngine::connect().expect("a container engine (DOCKER_HOST, or the local socket)");
        within(
            "pull the isolation image",
            START_LIMIT,
            engine.ensure_image(&image()),
        )
        .await
        .expect("the isolation image is on the engine or can be pulled");
        let volume = format!("sagat-lake-it-{}", uuid::Uuid::now_v7());
        let owner = format!("isolation-{}", uuid::Uuid::now_v7());
        engine
            .create_work_volume(&volume, 64 * MIB, 0)
            .await
            .expect("a work volume");
        let mut body = container_config(&LakeSpec {
            image: image(),
            attempt_id: "0".to_owned(),
            slot: 0,
            group: GROUP,
            mount: AttemptMount {
                volume: volume.clone(),
            },
            limits: limits(),
            owner: format!("{owner}-holder"), // not the probes' label: their sweep must not take the holder
            class: "small".to_owned(),
            deadline_epoch_secs: far_deadline(),
        })
        .expect("a valid profile");
        body.user = Some("0:0".to_owned());
        body.cmd = Some(vec!["sleep".to_owned(), "3600".to_owned()]);
        let host = body.host_config.as_mut().expect("host configuration");
        host.cap_drop = None; // the holder stands for the lake runner, which must be able to give a file away
        let mount = &mut host.mounts.as_mut().expect("mounts")[0];
        mount.target = Some("/w".to_owned());
        mount
            .volume_options
            .as_mut()
            .expect("volume options")
            .subpath = None;
        let holder = within("start the holder", START_LIMIT, engine.start(body))
            .await
            .expect("the holder starts");
        let work = Self {
            engine,
            holder,
            volume,
            owner,
            attempt_a: uuid::Uuid::now_v7().to_string(),
            attempt_b: uuid::Uuid::now_v7().to_string(),
        };
        work.prepare().await;
        work
    }

    fn spec(&self, attempt: &str, slot: u32, limits: LakeLimits) -> LakeSpec {
        LakeSpec {
            image: image(),
            attempt_id: attempt.to_owned(),
            slot,
            group: GROUP,
            mount: AttemptMount {
                volume: self.volume.clone(),
            },
            limits,
            owner: self.owner.clone(),
            class: "small".to_owned(),
            deadline_epoch_secs: far_deadline(),
        }
    }

    /// The lake runner's part: make the two directories and their tokens, owned by the runner's user and group, the
    /// directory 0770 and the token 0640 (D035-086). The runner is a plain user and cannot give a file to the user of a
    /// slot; the holder is root only because the test volume is created by root.
    async fn prepare(&self) {
        let (a, b) = (&self.attempt_a, &self.attempt_b);
        let script = format!(
            "set -e; mkdir /w/{a} /w/{b}; \
             printf %s {TOKEN_A} > /w/{a}/relay.token; printf %s {TOKEN_B} > /w/{b}/relay.token; \
             chown {RUNNER_UID}:{GROUP} /w/{a} /w/{a}/relay.token /w/{b} /w/{b}/relay.token; \
             chmod 0770 /w/{a} /w/{b}; chmod 0640 /w/{a}/relay.token /w/{b}/relay.token"
        );
        let exit = self
            .holder
            .exec(
                vec!["sh".to_owned(), "-c".to_owned(), script],
                PROBE_WAIT,
                4096,
            )
            .await
            .expect("the preparer runs");
        assert_eq!(exit.code, Some(0), "preparing the work volume: {exit:?}");
    }

    /// Runs `script` in the lake profile of `attempt` and `slot` and returns what it printed as `key=value` lines.
    async fn probe(
        &self,
        attempt: &str,
        slot: u32,
        limits: LakeLimits,
        script: &str,
    ) -> (Exit, HashMap<String, String>) {
        self.probe_in_group(attempt, slot, GROUP, limits, script)
            .await
    }

    /// As [`Self::probe`], with the container in `group` instead of the runner's.
    async fn probe_in_group(
        &self,
        attempt: &str,
        slot: u32,
        group: u32,
        limits: LakeLimits,
        script: &str,
    ) -> (Exit, HashMap<String, String>) {
        let script = script
            .replace("@A@", &self.attempt_a)
            .replace("@B@", &self.attempt_b);
        let mut spec = self.spec(attempt, slot, limits);
        spec.group = group;
        let mut body = container_config(&spec).expect("a valid profile");
        body.cmd = Some(vec!["sh".to_owned(), "-c".to_owned(), script]);
        let exit = self
            .engine
            .run(body, PROBE_WAIT, 64 * 1024)
            .await
            .expect("the probe runs");
        let printed = String::from_utf8_lossy(&exit.stdout)
            .lines()
            .filter_map(|line| {
                line.split_once('=')
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
            })
            .collect();
        (exit, printed)
    }

    async fn finish(self) {
        self.holder.stop().await;
        self.engine.remove_work_volume(&self.volume).await;
    }
}

const SURVEY: &str = r#"
echo "uid=$(id -u)"
echo "gid=$(id -g)"
echo "capeff=$(awk '/^CapEff/ {print $2}' /proc/self/status)"
echo "capbnd=$(awk '/^CapBnd/ {print $2}' /proc/self/status)"
echo "nonewprivs=$(awk '/^NoNewPrivs/ {print $2}' /proc/self/status)"
if touch /var/tmp/x 2>/dev/null; then echo rootwrite=yes; else echo rootwrite=no; fi
if cp /bin/busybox /tmp/true 2>/dev/null && /tmp/true 2>/dev/null; then echo tmpexec=yes; else echo tmpexec=no; fi
if dd if=/dev/zero of=/tmp/full bs=1M count=16 2>/dev/null; then echo tmpfull=no; else echo tmpfull=yes; fi
echo "ifaces=$(ls /sys/class/net | tr '\n' ',')"
for host in 169.254.169.254 fd00:ec2::254 1.1.1.1; do
  if python3 -I -c "import socket,sys; socket.create_connection((sys.argv[1], 80), 1)" "$host" 2>/dev/null; then echo "reach_$host=yes"; else echo "reach_$host=no"; fi
done
if [ -e /var/run/docker.sock ] || [ -e /run/docker.sock ] || [ -e /var/run/secrets ]; then echo enginesock=yes; else echo enginesock=no; fi
echo "work=$(ls /work | tr '\n' ',')"
echo "ownfile=$(cat /work/@A@/relay.token 2>&1)"
echo "otherfile=$(cat /work/@B@/relay.token 2>&1)"
echo "memmax=$(cat /sys/fs/cgroup/memory.max)"
echo "swapmax=$(cat /sys/fs/cgroup/memory.swap.max)"
echo "pidsmax=$(cat /sys/fs/cgroup/pids.max)"
echo "core=$(ulimit -c)"
echo "nofile=$(ulimit -n)"
"#;

#[tokio::test(flavor = "multi_thread")]
async fn an_attempt_container_has_the_user_the_root_the_capabilities_and_the_limits_of_its_profile()
{
    let work = Work::new().await;
    let (exit, got) = work
        .probe(&work.attempt_a.clone(), SLOT_A, limits(), SURVEY)
        .await;
    assert_eq!(exit.code, Some(0), "{exit:?}");
    let uid = uid_for_slot(SLOT_A).expect("slot").to_string();
    assert_eq!(
        (got["uid"].as_str(), got["gid"].as_str()),
        (uid.as_str(), GROUP.to_string().as_str()),
        "the user of the slot, in the group of the runner"
    );
    assert_eq!(
        got["capeff"], "0000000000000000",
        "an effective capability survived"
    );
    assert_eq!(
        got["capbnd"], "0000000000000000",
        "a capability is still in the bounding set"
    );
    assert_eq!(got["nonewprivs"], "1");
    assert_eq!(
        got["rootwrite"], "no",
        "a world-writable path of the root is writable"
    );
    assert_eq!(got["tmpexec"], "no", "code ran from /tmp");
    assert_eq!(got["tmpfull"], "yes", "/tmp took more than its size");
    assert_eq!(got["memmax"], (256 * MIB).to_string());
    assert_eq!(got["swapmax"], "0", "swap is not off");
    assert_eq!(got["pidsmax"], "64");
    assert_eq!(got["core"], "0");
    assert_eq!(got["nofile"], "1024");
    work.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_attempt_container_reaches_neither_the_network_nor_the_metadata_service_nor_the_engine()
{
    let work = Work::new().await;
    let (exit, got) = work
        .probe(&work.attempt_a.clone(), SLOT_A, limits(), SURVEY)
        .await;
    assert_eq!(exit.code, Some(0), "{exit:?}");
    assert_eq!(got["ifaces"], "lo,", "an interface besides loopback exists");
    for target in ["169.254.169.254", "fd00:ec2::254", "1.1.1.1"] {
        assert_eq!(
            got[&format!("reach_{target}")],
            "no",
            "{target} was reached"
        );
    }
    assert_eq!(
        got["enginesock"], "no",
        "an engine socket or a service account is visible"
    );
    work.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_attempt_sees_its_own_directory_and_not_another_attempts() {
    let work = Work::new().await;
    let (exit, got) = work
        .probe(&work.attempt_a.clone(), SLOT_A, limits(), SURVEY)
        .await;
    assert_eq!(exit.code, Some(0), "{exit:?}");
    assert_eq!(
        got["work"],
        format!("{},", work.attempt_a),
        "the work directory lists more than the attempt's own"
    );
    assert_eq!(
        got["ownfile"], TOKEN_A,
        "the positive control: the attempt reads its own token"
    );
    assert!(
        got["otherfile"].contains("No such file"),
        "another attempt's token: {}",
        got["otherfile"]
    );
    assert!(!got["otherfile"].contains(TOKEN_B));
    work.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_group_of_the_runner_reads_what_the_runner_left_in_the_attempt_directory() {
    let work = Work::new().await;
    // The container is in the attempt's mount and in another group: the directory is 0770 and the token 0640, so nothing
    // is open to "others".
    let (exit, got) = work
        .probe_in_group(
            &work.attempt_a.clone(),
            SLOT_A,
            OTHER_GROUP,
            limits(),
            SURVEY,
        )
        .await;
    assert_eq!(exit.code, Some(0), "{exit:?}");
    assert!(
        got["ownfile"].contains("Permission denied") || got["ownfile"].contains("Can't open"),
        "a container outside the runner's group read the token: {}",
        got["ownfile"]
    );
    assert!(!got["ownfile"].contains(TOKEN_A));
    work.finish().await;
}

/// What the decision costs (D035-086, the owner's choice of a shared group over a capability for the runner). The user id of
/// a slot is a process identity, not a boundary between attempts: if a container were given another attempt's directory,
/// the group would let it read the token. What keeps two attempts apart is the mount (`an_attempt_sees_its_own_directory_and_
/// not_another_attempts`) and the token on the relay. This test fails the day someone makes the user a boundary again, so
/// that the contract is amended with it.
#[tokio::test(flavor = "multi_thread")]
async fn the_user_of_a_slot_is_not_a_boundary_between_attempts() {
    let work = Work::new().await;
    let (exit, got) = work
        .probe(&work.attempt_a.clone(), SLOT_B, limits(), SURVEY)
        .await;
    assert_eq!(exit.code, Some(0), "{exit:?}");
    assert_eq!(
        got["ownfile"], TOKEN_A,
        "a container of another slot in the runner's group, given this directory, reads the token"
    );
    work.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_work_volume_and_the_file_size_bound_what_an_attempt_can_write() {
    let work = Work::new().await;
    let script = r#"
dd if=/dev/zero of=/work/@A@/big bs=1M count=40 2>/dev/null || echo "big=failed"
echo "biggest=$(stat -c %s /work/@A@/big)"
rm -f /work/@A@/big
for i in 1 2 3; do dd if=/dev/zero of=/work/@A@/f$i bs=1M count=30 2>/dev/null || echo "f$i=failed"; done
echo "total=$(du -sk /work/@A@ | cut -f1)"
"#;
    let (exit, got) = work
        .probe(&work.attempt_a.clone(), SLOT_A, limits(), script)
        .await;
    assert_eq!(exit.code, Some(0), "{exit:?}");
    assert_eq!(
        got.get("f1"),
        None,
        "30 MiB fits the volume and the file limit"
    );
    assert_eq!(
        got.get("f2"),
        None,
        "60 MiB in two files fits a 64 MiB volume"
    );
    assert_eq!(
        got.get("f3").map(String::as_str),
        Some("failed"),
        "a third 30 MiB file must not fit"
    );
    assert_eq!(
        got["big"], "failed",
        "a 40 MiB file fits the volume, so only the file-size limit can stop it"
    );
    let biggest: u64 = got["biggest"].parse().expect("a size");
    assert!(biggest <= 32 * MIB, "a file grew to {biggest} bytes");
    let total: u64 = got["total"].parse().expect("a size in KiB");
    assert!(
        total <= 64 * 1024,
        "the attempt wrote {total} KiB into a 64 MiB volume"
    );
    work.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_process_that_outgrows_its_memory_is_killed_by_the_engine() {
    let work = Work::new().await;
    let mut small = limits();
    small.memory_bytes = 96 * MIB;
    let script = r#"python3 -I -c "x = bytearray(400 * 1024 * 1024); print('alive=yes')""#;
    let (exit, got) = work
        .probe(&work.attempt_a.clone(), SLOT_A, small, script)
        .await;
    assert!(
        !got.contains_key("alive"),
        "the process was not stopped: {exit:?}"
    );
    assert!(exit.code == Some(137) || exit.oom_killed, "{exit:?}");
    assert!(!exit.timed_out);
    work.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_process_that_outstays_its_deadline_is_killed_and_the_container_is_gone() {
    let work = Work::new().await;
    let mut body = container_config(&work.spec(&work.attempt_a.clone(), SLOT_A, limits()))
        .expect("a valid profile");
    body.cmd = Some(vec![
        "sh".to_owned(),
        "-c".to_owned(),
        "sleep 120".to_owned(),
    ]);
    let began = Instant::now();
    let exit = work
        .engine
        .run(body, Duration::from_secs(2), 1024)
        .await
        .expect("the probe runs");
    assert!(exit.timed_out, "{exit:?}");
    assert_eq!(exit.code, Some(137), "{exit:?}");
    assert!(
        began.elapsed() < Duration::from_secs(30),
        "the kill took {:?}",
        began.elapsed()
    );
    assert_eq!(
        work.engine.sweep(&work.owner, Duration::ZERO).await,
        0,
        "the killed container was not removed"
    );
    work.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_image_the_engine_does_not_have_is_an_unavailable_engine_and_leaves_nothing_behind() {
    let work = Work::new().await;
    let mut spec = work.spec(&work.attempt_a.clone(), SLOT_A, limits());
    spec.image = "sagat-no-such-image:0".to_owned();
    let body = container_config(&spec).expect("a valid profile");
    let outcome = work.engine.run(body, Duration::from_secs(5), 1024).await;
    assert_eq!(outcome.err(), Some(EngineError::Unavailable));
    assert_eq!(work.engine.sweep(&work.owner, Duration::ZERO).await, 0);
    work.finish().await;
}
