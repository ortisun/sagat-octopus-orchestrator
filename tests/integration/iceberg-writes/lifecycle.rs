//! T035-035 / G035-055: the lifecycle of a lake container by its labels, against a real container engine (ADR-0079, rules 7
//! and 8), and against a scripted one for the answers a real engine will not give on request.
//!
//! The sandbox runner of a lake deployment keeps no state of its own: what is running, in which slot, in which budget class
//! and until when is read from the labels on the containers (`sagat.lake.*`), so that a restarted process finds what the
//! last one left. The cases start real containers from the lake profile with the mount removed and the command replaced,
//! and look at what the API above them is made of: launch, find, wait, end (which reads the out-of-memory flag **before**
//! it removes the container), the sweep by the deadline label, and the answers when the engine cannot be reached, refuses
//! a removal, or cannot inspect.
//!
//! The image is `python:3.13-alpine` unless `SAGAT_LAKE_ISOLATION_IMAGE` names another (the same as `isolation.rs`).

use std::sync::mpsc;
use std::time::{Duration, Instant};

use sagat_infra_sandbox::lake::engine::{EndedContainer, EngineError, LakeEngine, LakeInfo, Wait};
use sagat_infra_sandbox::lake::{AttemptMount, LakeLimits, LakeSpec, container_config};

use crate::support::bounded::{START_LIMIT, within};

const MIB: u64 = 1024 * 1024;
const GROUP: u32 = 10_001;
const STAGE: Duration = Duration::from_secs(60);

fn image() -> String {
    std::env::var("SAGAT_LAKE_ISOLATION_IMAGE").unwrap_or_else(|_| "python:3.13-alpine".to_owned())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn attempt() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// How a case wants the container made.
#[derive(Clone, Copy)]
enum Shape {
    /// As the profile makes it.
    Plain,
    /// With the labels taken away: not a lake container, and `launch` refuses it.
    Unlabelled,
    /// Without the engine's init process, so that a command that is not there fails the **start**: with init the container
    /// starts and the missing command is an exit status.
    CannotStart,
}

/// Makes a lake container as the profile makes it, with the mount taken away (no work volume is needed to look at the
/// lifecycle) and the command replaced, and launches it on `engine`. The `shape` says whether it is a container the
/// profile makes, one without its labels, or one that cannot start.
#[allow(clippy::too_many_arguments)]
async fn launch_on(
    engine: &LakeEngine,
    owner: &str,
    attempt: &str,
    slot: u32,
    deadline: u64,
    memory_mib: u64,
    command: &[&str],
    shape: Shape,
) -> Result<(), EngineError> {
    let mut body = container_config(&LakeSpec {
        image: image(),
        attempt_id: attempt.to_owned(),
        slot,
        group: GROUP,
        mount: AttemptMount {
            volume: "sagat-lake-it-not-mounted".to_owned(),
        },
        limits: LakeLimits {
            memory_bytes: memory_mib * MIB,
            processes: 64,
            cpu_millis: 1000,
            work_bytes: 32 * MIB,
            tmp_bytes: 8 * MIB,
        },
        owner: owner.to_owned(),
        class: "small".to_owned(),
        deadline_epoch_secs: deadline,
    })
    .expect("a valid profile");
    body.host_config
        .as_mut()
        .expect("host configuration")
        .mounts = None;
    body.cmd = Some(command.iter().map(|part| (*part).to_owned()).collect());
    match shape {
        Shape::Plain => {}
        Shape::Unlabelled => body.labels = None,
        Shape::CannotStart => {
            body.host_config.as_mut().expect("host configuration").init = Some(false);
        }
    }
    within("launch", START_LIMIT, engine.launch(body)).await
}

/// An engine and a label of its own, so that one case never sees, ends or sweeps another case's containers. Whatever the
/// case leaves, also when an assertion failed, is ended when the fixture is dropped.
struct Fixture {
    engine: LakeEngine,
    owner: String,
}

impl Fixture {
    async fn new() -> Self {
        let engine =
            LakeEngine::connect().expect("a container engine (DOCKER_HOST, or the local socket)");
        within("pull the image", START_LIMIT, engine.ensure_image(&image()))
            .await
            .expect("the image is on the engine or can be pulled");
        Self {
            engine,
            owner: format!("lifecycle-{}", uuid::Uuid::now_v7()),
        }
    }

    /// Another owner on the same engine: its own label, its own cleanup.
    fn sibling(&self, suffix: &str) -> Self {
        Self {
            engine: self.engine.clone(),
            owner: format!("{}{suffix}", self.owner),
        }
    }

    async fn try_launch(
        &self,
        attempt: &str,
        slot: u32,
        deadline: u64,
        memory_mib: u64,
        command: &[&str],
        shape: Shape,
    ) -> Result<(), EngineError> {
        launch_on(
            &self.engine,
            &self.owner,
            attempt,
            slot,
            deadline,
            memory_mib,
            command,
            shape,
        )
        .await
    }

    async fn launch(
        &self,
        attempt: &str,
        slot: u32,
        deadline: u64,
        memory_mib: u64,
        command: &[&str],
    ) {
        self.try_launch(attempt, slot, deadline, memory_mib, command, Shape::Plain)
            .await
            .expect("the container starts");
    }

    async fn find(&self, attempt: &str) -> Vec<LakeInfo> {
        within("find", STAGE, self.engine.find(&self.owner, attempt))
            .await
            .expect("the engine answers")
    }

    async fn nothing_is_left(&self) {
        let left = within("list", STAGE, self.engine.list(&self.owner))
            .await
            .expect("the engine answers");
        assert!(left.is_empty(), "containers were left behind: {left:?}");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let owner = self.owner.clone();
        // A thread of its own with a runtime of its own: this may run while the case's runtime is going away, after a
        // failed assertion, and a case must not leave containers on the shared engine.
        let _ = std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async {
                let Ok(engine) = LakeEngine::connect() else {
                    return;
                };
                for container in engine.list(&owner).await.unwrap_or_default() {
                    engine.end(&owner, &container.attempt_id).await;
                }
            });
        })
        .join();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launched_container_is_found_by_the_labels_it_was_made_with() {
    let fixture = Fixture::new().await;
    let (first, second) = (attempt(), attempt());
    let deadline = now() + 3600;
    fixture
        .launch(&first, 11, deadline, 128, &["sleep", "60"])
        .await;
    fixture
        .launch(&second, 12, deadline + 5, 128, &["sleep", "60"])
        .await;

    let found = fixture.find(&first).await;
    assert_eq!(found.len(), 1, "{found:?}");
    let info = &found[0];
    assert_eq!(info.attempt_id, first);
    assert_eq!(info.slot, Some(11));
    assert_eq!(info.class.as_deref(), Some("small"));
    assert_eq!(info.deadline_epoch_secs, Some(deadline));
    // the limit the container was made with, which a restarted process adds up
    assert_eq!(info.memory_bytes, Some(128 * MIB));
    assert!(info.running, "{info:?}");

    // the owner's whole list is what a restarted process rebuilds its slots and reservations from
    let all = within("list", STAGE, fixture.engine.list(&fixture.owner))
        .await
        .expect("the engine answers");
    let mut slots: Vec<_> = all.iter().filter_map(|info| info.slot).collect();
    slots.sort_unstable();
    assert_eq!(slots, vec![11, 12], "{all:?}");

    for attempt in [&first, &second] {
        within("end", STAGE, fixture.engine.end(&fixture.owner, attempt)).await;
    }
    fixture.nothing_is_left().await;
}

/// The sub-path mount, which is what keeps a lake container to its own attempt directory, came with API 1.45. The engine
/// these cases run on says what it speaks, and the sandbox runner refuses to start against one that is older.
#[tokio::test(flavor = "multi_thread")]
async fn the_engine_says_which_api_it_speaks_and_it_has_sub_path_mounts() {
    let fixture = Fixture::new().await;
    let version = within("version", STAGE, fixture.engine.api_version())
        .await
        .expect("the engine answers");
    assert!(version >= (1, 45), "{version:?}");
}

/// The container is made and then fails to start (the command is not there and there is no init to hide it), so the cleanup
/// of a failed start is what is under test, not the engine's refusal to make a container from an image it lacks.
#[tokio::test(flavor = "multi_thread")]
async fn a_start_that_fails_leaves_nothing_behind() {
    let fixture = Fixture::new().await;
    let id = attempt();
    let outcome = fixture
        .try_launch(
            &id,
            13,
            now() + 3600,
            128,
            &["/no/such/binary"],
            Shape::CannotStart,
        )
        .await;
    assert_eq!(outcome.err(), Some(EngineError::Unavailable));
    fixture.nothing_is_left().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_launch_of_an_attempt_is_refused_by_the_engine_and_the_first_is_untouched() {
    let fixture = Fixture::new().await;
    let id = attempt();
    fixture
        .launch(&id, 14, now() + 3600, 128, &["sleep", "60"])
        .await;
    let again = fixture
        .try_launch(&id, 15, now() + 3600, 128, &["sleep", "60"], Shape::Plain)
        .await;
    assert_eq!(again.err(), Some(EngineError::Exists));
    let found = fixture.find(&id).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].slot, Some(14), "the first container was replaced");
    assert!(found[0].running, "the first container was ended: {found:?}");
    within("end", STAGE, fixture.engine.end(&fixture.owner, &id)).await;
    fixture.nothing_is_left().await;
}

/// A container without the labels would not be found, counted or swept: it is not made.
#[tokio::test(flavor = "multi_thread")]
async fn a_launch_without_the_labels_of_the_profile_makes_nothing() {
    let fixture = Fixture::new().await;
    let outcome = fixture
        .try_launch(
            &attempt(),
            16,
            now() + 3600,
            128,
            &["sleep", "60"],
            Shape::Unlabelled,
        )
        .await;
    assert_eq!(outcome.err(), Some(EngineError::Invalid));
    fixture.nothing_is_left().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_holds_while_the_container_runs() {
    let fixture = Fixture::new().await;
    let id = attempt();
    // a container that runs until it is ended, so that the answer cannot depend on how fast the machine is
    fixture
        .launch(&id, 17, now() + 3600, 128, &["sleep", "60"])
        .await;

    let began = Instant::now();
    let state = within(
        "wait while running",
        STAGE,
        fixture
            .engine
            .wait(&fixture.owner, &id, Duration::from_millis(700)),
    )
    .await
    .expect("the engine answers");
    assert_eq!(state, Wait::Running);
    assert!(
        began.elapsed() >= Duration::from_millis(600),
        "the wait returned at {:?} while the container ran",
        began.elapsed()
    );

    within("end", STAGE, fixture.engine.end(&fixture.owner, &id)).await;
    fixture.nothing_is_left().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_answers_ended_when_the_container_exits_and_the_exited_container_is_kept() {
    let fixture = Fixture::new().await;
    let id = attempt();
    fixture
        .launch(&id, 18, now() + 3600, 128, &["sh", "-c", "exit 0"])
        .await;
    let state = within(
        "wait for the exit",
        STAGE,
        fixture
            .engine
            .wait(&fixture.owner, &id, Duration::from_secs(30)),
    )
    .await
    .expect("the engine answers");
    assert_eq!(state, Wait::Ended);

    // an exited container is kept until it is ended, so that what the engine said about it can still be read
    let kept = fixture.find(&id).await;
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!(!kept[0].running);

    within("end", STAGE, fixture.engine.end(&fixture.owner, &id)).await;
    fixture.nothing_is_left().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wait_for_an_attempt_with_no_container_answers_ended_at_once() {
    let fixture = Fixture::new().await;
    let began = Instant::now();
    let state = within(
        "wait for nothing",
        STAGE,
        fixture
            .engine
            .wait(&fixture.owner, &attempt(), Duration::from_secs(30)),
    )
    .await
    .expect("the engine answers");
    assert_eq!(state, Wait::Ended);
    assert!(
        began.elapsed() < Duration::from_secs(10),
        "{:?}",
        began.elapsed()
    );

    // a hold that no clock can add to is clamped, not a panic in the process that holds the engine socket
    let state = within(
        "wait for ever",
        STAGE,
        fixture
            .engine
            .wait(&fixture.owner, &attempt(), Duration::MAX),
    )
    .await
    .expect("the engine answers");
    assert_eq!(state, Wait::Ended);
}

#[tokio::test(flavor = "multi_thread")]
async fn end_kills_a_running_container_removes_it_and_a_second_end_has_nothing_to_do() {
    let fixture = Fixture::new().await;
    let id = attempt();
    fixture
        .launch(&id, 19, now() + 3600, 128, &["sleep", "60"])
        .await;

    let first = within("end", STAGE, fixture.engine.end(&fixture.owner, &id)).await;
    assert!(
        first.existed && first.confirmed && !first.memory_killed,
        "{first:?}"
    );
    fixture.nothing_is_left().await;

    // nothing to end, and the engine said so: ended, not "could not tell"
    let second = within("end again", STAGE, fixture.engine.end(&fixture.owner, &id)).await;
    assert!(
        !second.existed && second.confirmed && !second.memory_killed,
        "{second:?}"
    );
}

/// The fact the supervisor needs (`ICEBERG_LIMIT_EXCEEDED`) is on the container until it is removed; removing first would
/// lose it, and a memory kill would be reported as a host that broke.
// (experiment, not for merge: this comment only makes CI select this suite, to see whether develop passes this test)
#[tokio::test(flavor = "multi_thread")]
async fn end_reads_the_memory_flag_before_it_removes_the_container() {
    let fixture = Fixture::new().await;
    let id = attempt();
    let grow = r#"python3 -I -c "x = bytearray(400 * 1024 * 1024); print('alive')""#;
    fixture
        .launch(&id, 20, now() + 3600, 96, &["sh", "-c", grow])
        .await;
    let state = within(
        "wait for the kill",
        STAGE,
        fixture
            .engine
            .wait(&fixture.owner, &id, Duration::from_secs(30)),
    )
    .await
    .expect("the engine answers");
    assert_eq!(state, Wait::Ended);

    let ended = within("end", STAGE, fixture.engine.end(&fixture.owner, &id)).await;
    assert!(ended.existed && ended.confirmed, "{ended:?}");
    assert!(ended.memory_killed, "the memory kill was lost: {ended:?}");
    fixture.nothing_is_left().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_container_that_exits_by_itself_is_not_a_memory_kill() {
    let fixture = Fixture::new().await;
    let id = attempt();
    fixture
        .launch(&id, 21, now() + 3600, 128, &["sh", "-c", "exit 3"])
        .await;
    within(
        "wait for the exit",
        STAGE,
        fixture
            .engine
            .wait(&fixture.owner, &id, Duration::from_secs(30)),
    )
    .await
    .expect("the engine answers");
    let ended = within("end", STAGE, fixture.engine.end(&fixture.owner, &id)).await;
    assert!(
        ended.existed && ended.confirmed && !ended.memory_killed,
        "{ended:?}"
    );
}

/// The deadline is a label the sweep reads, so that it holds across a restart of the process that set it. `now` is a
/// parameter: no case waits for a real deadline.
#[tokio::test(flavor = "multi_thread")]
async fn the_sweep_ends_what_is_past_its_deadline_and_keeps_what_is_not() {
    let fixture = Fixture::new().await;
    let (late, on_time, finished_long_ago, finished_just_now) =
        (attempt(), attempt(), attempt(), attempt());
    let clock = now();
    let retain = Duration::from_secs(300);
    // running and past its deadline: ended at once
    fixture
        .launch(&late, 22, clock - 10, 128, &["sleep", "60"])
        .await;
    // running and before its deadline: left alone
    fixture
        .launch(&on_time, 23, clock + 3600, 128, &["sleep", "60"])
        .await;
    // exited, and past its deadline by more than the time it is kept: removed
    fixture
        .launch(
            &finished_long_ago,
            24,
            clock - 3600,
            128,
            &["sh", "-c", "exit 0"],
        )
        .await;
    // exited, past its deadline by less than the time it is kept: kept, its facts can still be read
    fixture
        .launch(
            &finished_just_now,
            25,
            clock - 10,
            128,
            &["sh", "-c", "exit 0"],
        )
        .await;
    for id in [&finished_long_ago, &finished_just_now] {
        within(
            "wait for the exit",
            STAGE,
            fixture
                .engine
                .wait(&fixture.owner, id, Duration::from_secs(30)),
        )
        .await
        .expect("the engine answers");
    }

    let sweep = within(
        "sweep",
        STAGE,
        fixture.engine.sweep_expired(&fixture.owner, clock, retain),
    )
    .await;
    assert!(sweep.listed, "{sweep:?}");
    assert_eq!(
        (sweep.removed, sweep.failed, sweep.unlabelled),
        (2, 0, 0),
        "the late one and the one finished long ago: {sweep:?}"
    );
    assert!(fixture.find(&late).await.is_empty());
    assert!(fixture.find(&finished_long_ago).await.is_empty());
    assert_eq!(fixture.find(&on_time).await.len(), 1);
    assert_eq!(fixture.find(&finished_just_now).await.len(), 1);

    for id in [&on_time, &finished_just_now] {
        within("end", STAGE, fixture.engine.end(&fixture.owner, id)).await;
    }
    fixture.nothing_is_left().await;
}

/// A container of an owner that merely starts with this owner's name is not this owner's: the labels are matched whole.
#[tokio::test(flavor = "multi_thread")]
async fn an_owner_sees_and_ends_only_its_own_containers_also_when_the_names_share_a_prefix() {
    let fixture = Fixture::new().await;
    let longer = fixture.sibling("-longer");
    let (mine, theirs) = (attempt(), attempt());
    fixture
        .launch(&mine, 26, now() + 3600, 128, &["sleep", "60"])
        .await;
    // past its deadline: any sweep that reached it would end it
    longer
        .launch(&theirs, 27, now() - 10, 128, &["sleep", "60"])
        .await;

    let listed = within("list", STAGE, fixture.engine.list(&fixture.owner))
        .await
        .expect("the engine answers");
    assert_eq!(
        listed
            .iter()
            .map(|info| info.attempt_id.as_str())
            .collect::<Vec<_>>(),
        vec![mine.as_str()],
        "{listed:?}"
    );
    assert!(fixture.find(&theirs).await.is_empty());
    let ended = within("end", STAGE, fixture.engine.end(&fixture.owner, &theirs)).await;
    assert!(!ended.existed, "{ended:?}");
    let sweep = within(
        "sweep",
        STAGE,
        fixture
            .engine
            .sweep_expired(&fixture.owner, now() + 1_000, Duration::ZERO),
    )
    .await;
    assert_eq!(
        (sweep.removed, sweep.failed),
        (0, 0),
        "this owner's own container is not due, and the longer owner's is not its to end: {sweep:?}"
    );
    assert_eq!(longer.find(&theirs).await.len(), 1);
    assert_eq!(fixture.find(&mine).await.len(), 1);

    within("end", STAGE, longer.engine.end(&longer.owner, &theirs)).await;
    within("end", STAGE, fixture.engine.end(&fixture.owner, &mine)).await;
    fixture.nothing_is_left().await;
    longer.nothing_is_left().await;
}

/// "Could not ask" is not "there is nothing": a container that holds the credentials may still be up, and the answer must
/// not say it is gone (ADR-0079, rule 7).
#[tokio::test(flavor = "multi_thread")]
async fn an_engine_that_cannot_be_reached_never_answers_that_something_has_ended() {
    // A socket file nobody listens on any more: what an engine that has died leaves. (The client refuses a path that is
    // not there at all, so the file is made, bound and closed.)
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("engine.sock");
    drop(std::os::unix::net::UnixListener::bind(&path).expect("a socket"));
    let engine =
        LakeEngine::connect_unix(path.to_str().expect("a path")).expect("a client for an address");
    let owner = "lifecycle-unreachable";
    let id = attempt();

    assert_eq!(
        within("find", STAGE, engine.find(owner, &id)).await.err(),
        Some(EngineError::Unavailable)
    );
    assert_eq!(
        within("list", STAGE, engine.list(owner)).await.err(),
        Some(EngineError::Unavailable)
    );
    assert_eq!(
        within(
            "wait",
            STAGE,
            engine.wait(owner, &id, Duration::from_secs(1))
        )
        .await
        .err(),
        Some(EngineError::Unavailable)
    );
    let ended = within("end", STAGE, engine.end(owner, &id)).await;
    assert!(
        !ended.confirmed,
        "an unreachable engine confirmed an end: {ended:?}"
    );
    assert!(!ended.memory_killed);
    let sweep = within(
        "sweep",
        STAGE,
        engine.sweep_expired(owner, now() + 1_000_000, Duration::ZERO),
    )
    .await;
    assert!(
        !sweep.listed && sweep.removed == 0,
        "an unreachable engine looked as if it had been swept: {sweep:?}"
    );
}

// ---- a scripted engine -------------------------------------------------------------------------------------------------
//
// A real engine does not fail on request, and the answers that matter most are the ones it gives when it does: a removal
// it refuses, an inspection it cannot make, a kill that takes long. This speaks just enough of the engine's HTTP API over a
// Unix socket, and each case sets what it answers. Its list **ignores the filters** and always shows the containers, so a
// case that expects nothing back proves the engine was not asked.

/// What the scripted engine answers.
#[derive(Clone, Copy)]
struct Script {
    /// `POST /containers/<id>/start`.
    start: u16,
    /// `POST /containers/<id>/kill`, after `kill_delay`.
    kill: u16,
    kill_delay: Duration,
    /// `GET /containers/<id>/json`: 200 answers a state, anything else is an engine that cannot inspect.
    inspect: u16,
    /// Whether the inspected container says it is still running, however long it is asked.
    inspect_running: bool,
    /// `DELETE /containers/<id>`.
    remove: u16,
    /// The state the list gives the container: `running` or `exited`.
    state: &'static str,
    /// The deadline label of the listed container.
    deadline: &'static str,
    /// A second container of the attempt, whose removal the engine always refuses and which was not killed by memory.
    duplicate: bool,
    /// `GET /version`: the API version the engine says it speaks.
    version: &'static str,
}

impl Script {
    /// A container that was killed by the memory limit and is removed without trouble.
    const fn ok() -> Self {
        Self {
            start: 204,
            kill: 204,
            kill_delay: Duration::ZERO,
            inspect: 200,
            inspect_running: false,
            remove: 204,
            state: "running",
            deadline: "1900000000",
            duplicate: false,
            version: "1.52",
        }
    }
}

const SCRIPTED_OWNER: &str = "scripted";
const SCRIPTED_ATTEMPT: &str = "feed-0001";
const SCRIPTED_ID: &str = "feedface0123";
const SCRIPTED_ID_TWO: &str = "feedface0456";
const SCRIPTED_DEADLINE: u64 = 1_900_000_000;

struct ScriptedEngine {
    engine: LakeEngine,
    /// The id of every container the engine was asked to remove, in order.
    removals: mpsc::Receiver<String>,
    _directory: tempfile::TempDir,
}

fn reply(status: u16, body_when_ok: &str) -> (u16, String) {
    if status >= 400 {
        (status, r#"{"message":"scripted failure"}"#.to_owned())
    } else if status == 204 {
        (204, String::new())
    } else {
        (status, body_when_ok.to_owned())
    }
}

impl ScriptedEngine {
    fn start(script: Script) -> Self {
        use std::io::{Read as _, Write as _};
        use std::os::unix::net::UnixListener;

        let directory = tempfile::tempdir().expect("a directory");
        let path = directory.path().join("engine.sock");
        let listener = UnixListener::bind(&path).expect("a socket");
        let (removed, removals) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let removed = removed.clone();
                std::thread::spawn(move || {
                    // one request per connection: read to the end of the headers, answer, close
                    let mut request = Vec::new();
                    let mut chunk = [0_u8; 1024];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => return,
                            Ok(read) => request.extend_from_slice(&chunk[..read]),
                        }
                        if request.len() > 16 * 1024 {
                            return;
                        }
                    }
                    let text = String::from_utf8_lossy(&request);
                    let line = text.lines().next().unwrap_or_default().to_owned();
                    let mut parts = line.split(' ');
                    let (method, target) = (
                        parts.next().unwrap_or_default(),
                        parts.next().unwrap_or_default(),
                    );
                    // drop the query, and the API version if the client sent one (`/v1.51/containers/json?all=1` and
                    // `/containers/json` are the same route)
                    let route = target.split('?').next().unwrap_or_default();
                    let route = match route
                        .strip_prefix("/v")
                        .and_then(|rest| rest.split_once('/'))
                    {
                        Some((version, rest))
                            if version
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || byte == b'.') =>
                        {
                            format!("/{rest}")
                        }
                        _ => route.to_owned(),
                    };
                    let segments: Vec<&str> = route.trim_matches('/').split('/').collect();
                    let container = |id: &str, state: &str| {
                        format!(
                            r#"{{"Id":"{id}","State":"{state}","Labels":{{"sagat.lake":"1","sagat.lake.owner":"{SCRIPTED_OWNER}","sagat.lake.attempt":"{SCRIPTED_ATTEMPT}","sagat.lake.slot":"7","sagat.lake.class":"small","sagat.lake.deadline":"{}"}}}}"#,
                            script.deadline
                        )
                    };
                    let (status, body) = match (method, segments.as_slice()) {
                        ("GET", ["version"]) => (
                            200,
                            format!(
                                r#"{{"Version":"29.0.0","ApiVersion":"{}"}}"#,
                                script.version
                            ),
                        ),
                        ("GET", ["containers", "json"]) => {
                            let mut listed = vec![container(SCRIPTED_ID, script.state)];
                            if script.duplicate {
                                listed.push(container(SCRIPTED_ID_TWO, script.state));
                            }
                            (200, format!("[{}]", listed.join(",")))
                        }
                        ("POST", ["containers", "create"]) => {
                            (201, r#"{"Id":"cafe0001","Warnings":[]}"#.to_owned())
                        }
                        ("POST", ["containers", _, "start"]) => reply(script.start, ""),
                        ("POST", ["containers", _, "kill"]) => {
                            std::thread::sleep(script.kill_delay);
                            reply(script.kill, "")
                        }
                        ("GET", ["containers", id, "json"]) => reply(
                            script.inspect,
                            &format!(
                                r#"{{"State":{{"Running":{},"OOMKilled":{}}}}}"#,
                                script.inspect_running,
                                *id == SCRIPTED_ID
                            ),
                        ),
                        ("DELETE", ["containers", id]) => {
                            let _ = removed.send((*id).to_owned());
                            if *id == SCRIPTED_ID_TWO {
                                reply(500, "")
                            } else {
                                reply(script.remove, "")
                            }
                        }
                        _ => (404, r#"{"message":"not scripted"}"#.to_owned()),
                    };
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                });
            }
        });
        let engine = LakeEngine::connect_unix(path.to_str().expect("a path"))
            .expect("a client for the scripted engine");
        Self {
            engine,
            removals,
            _directory: directory,
        }
    }

    async fn end(&self) -> EndedContainer {
        within(
            "end",
            STAGE,
            self.engine.end(SCRIPTED_OWNER, SCRIPTED_ATTEMPT),
        )
        .await
    }
}

/// The next container the scripted engine was asked to remove, within `limit`; `None` if none was. Waits on the channel,
/// not on the clock.
async fn next_removal(removals: mpsc::Receiver<String>, limit: Duration) -> Option<String> {
    tokio::task::spawn_blocking(move || removals.recv_timeout(limit).ok())
        .await
        .expect("the waiting thread")
}

/// The report of an ending is only as good as what the engine confirmed. A removal the engine refuses leaves a container
/// that may still hold the credentials: it is not confirmed, and the memory flag that was read before the attempt to remove
/// it is still reported.
#[tokio::test(flavor = "multi_thread")]
async fn a_removal_the_engine_refuses_is_not_confirmed_but_the_memory_kill_it_read_is_reported() {
    let scripted = ScriptedEngine::start(Script {
        remove: 500,
        ..Script::ok()
    });
    let ended = scripted.end().await;
    assert!(ended.existed, "{ended:?}");
    assert!(ended.memory_killed, "{ended:?}");
    assert!(
        !ended.confirmed,
        "a container the engine refused to remove was confirmed gone: {ended:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_container_the_engine_says_is_not_there_when_it_is_removed_is_gone() {
    let scripted = ScriptedEngine::start(Script {
        remove: 404,
        ..Script::ok()
    });
    let ended = scripted.end().await;
    assert!(
        ended.existed && ended.confirmed && ended.memory_killed,
        "{ended:?}"
    );
}

/// An engine that cannot inspect does not stop the container being removed (the credentials come first), and what it could
/// not read is not invented: the memory kill is not reported. The supervisor then sees a host that broke and the step is
/// retried, which a limit that was exceeded would not be (D035-111); that is the price of a flaky engine, and it is stated.
#[tokio::test(flavor = "multi_thread")]
async fn an_engine_that_cannot_inspect_still_removes_and_does_not_invent_a_memory_kill() {
    let scripted = ScriptedEngine::start(Script {
        inspect: 500,
        ..Script::ok()
    });
    let ended = scripted.end().await;
    assert!(ended.existed && ended.confirmed, "{ended:?}");
    assert!(!ended.memory_killed, "{ended:?}");
}

/// A kill the engine fails or does not find changes nothing about the ending: the inspection and the removal decide.
#[tokio::test(flavor = "multi_thread")]
async fn a_kill_the_engine_fails_or_does_not_find_does_not_change_the_ending() {
    for kill in [404, 500] {
        let scripted = ScriptedEngine::start(Script {
            kill,
            ..Script::ok()
        });
        let ended = scripted.end().await;
        assert!(
            ended.existed && ended.confirmed && ended.memory_killed,
            "kill answered {kill}: {ended:?}"
        );
    }
}

/// A container that is still running after the engine's grace is removed by force, and the removal decides; the wait for it
/// does not spin and does not last longer than the grace.
#[tokio::test(flavor = "multi_thread")]
async fn a_container_still_running_after_the_grace_is_removed_by_force_and_the_removal_decides() {
    let began = Instant::now();
    let removed = ScriptedEngine::start(Script {
        inspect_running: true,
        ..Script::ok()
    });
    let ended = removed.end().await;
    assert!(
        ended.existed && ended.confirmed && ended.memory_killed,
        "{ended:?}"
    );
    assert!(
        began.elapsed() >= Duration::from_secs(4),
        "the grace was not waited for: {:?}",
        began.elapsed()
    );
    assert!(
        began.elapsed() < Duration::from_secs(30),
        "{:?}",
        began.elapsed()
    );

    let refused = ScriptedEngine::start(Script {
        inspect_running: true,
        remove: 500,
        ..Script::ok()
    });
    let ended = refused.end().await;
    assert!(
        ended.existed && !ended.confirmed,
        "a container that never stopped and could not be removed was confirmed: {ended:?}"
    );
}

/// Two containers of one attempt (a launch the engine did not refuse, from before the name was the attempt's): every one is
/// ended, the flags are added up, and a single removal the engine refuses leaves the whole ending unconfirmed.
#[tokio::test(flavor = "multi_thread")]
async fn several_containers_of_an_attempt_are_all_ended_and_one_refusal_leaves_it_unconfirmed() {
    let scripted = ScriptedEngine::start(Script {
        duplicate: true,
        ..Script::ok()
    });
    let ended = scripted.end().await;
    assert!(ended.existed, "{ended:?}");
    assert!(
        ended.memory_killed,
        "the first container's flag was lost: {ended:?}"
    );
    assert!(
        !ended.confirmed,
        "the second container could not be removed and the ending was confirmed: {ended:?}"
    );
    let mut asked: Vec<String> = scripted.removals.try_iter().collect();
    asked.sort();
    assert_eq!(
        asked,
        vec![SCRIPTED_ID.to_owned(), SCRIPTED_ID_TWO.to_owned()],
        "every container of the attempt was asked to be removed"
    );
}

/// A start that fails and a removal that fails: the guard is not let go, so the removal is tried again in the background.
/// (Before, `remove` was the only attempt and the container stayed, counted against the slots, until the deadline.)
#[tokio::test(flavor = "multi_thread")]
async fn a_start_that_fails_and_a_removal_the_engine_refuses_is_tried_again() {
    let scripted = ScriptedEngine::start(Script {
        start: 500,
        remove: 500,
        ..Script::ok()
    });
    let outcome = launch_on(
        &scripted.engine,
        SCRIPTED_OWNER,
        SCRIPTED_ATTEMPT,
        7,
        SCRIPTED_DEADLINE,
        128,
        &["sleep", "60"],
        Shape::Plain,
    )
    .await;
    assert_eq!(outcome.err(), Some(EngineError::Unavailable));

    let name = format!("sagat-lake-attempt-{SCRIPTED_ATTEMPT}");
    let removals = scripted.removals;
    let (first, second) = tokio::task::spawn_blocking(move || {
        let limit = Duration::from_secs(10);
        (
            removals.recv_timeout(limit).ok(),
            removals.recv_timeout(limit).ok(),
        )
    })
    .await
    .expect("the waiting thread");
    assert_eq!(first.as_deref(), Some(name.as_str()), "the removal at once");
    assert_eq!(
        second.as_deref(),
        Some(name.as_str()),
        "no second removal: the guard was let go of after a removal the engine refused"
    );
}

/// The lake runner bounds its kill by a time. A caller that gives up must not stop the container being removed halfway,
/// with the credentials still in it.
#[tokio::test(flavor = "multi_thread")]
async fn an_end_the_caller_gave_up_on_is_finished() {
    let scripted = ScriptedEngine::start(Script {
        kill_delay: Duration::from_millis(500),
        ..Script::ok()
    });
    let gave_up = tokio::time::timeout(
        Duration::from_millis(50),
        scripted.engine.end(SCRIPTED_OWNER, SCRIPTED_ATTEMPT),
    )
    .await;
    assert!(gave_up.is_err(), "the end was quicker than the kill");
    let removed = next_removal(scripted.removals, Duration::from_secs(10)).await;
    assert_eq!(
        removed.as_deref(),
        Some(SCRIPTED_ID),
        "the end was abandoned with the caller and the container was not removed"
    );
}

/// What the engine confirmed removed is what the sweep counts, and a removal it refused is a failure, not a success.
#[tokio::test(flavor = "multi_thread")]
async fn the_sweep_counts_what_the_engine_confirmed_and_what_it_refused() {
    let removed = ScriptedEngine::start(Script::ok());
    let sweep = within(
        "sweep",
        STAGE,
        removed
            .engine
            .sweep_expired(SCRIPTED_OWNER, SCRIPTED_DEADLINE + 1, Duration::ZERO),
    )
    .await;
    assert_eq!(
        (sweep.listed, sweep.removed, sweep.failed, sweep.unlabelled),
        (true, 1, 0, 0)
    );

    let refused = ScriptedEngine::start(Script {
        remove: 500,
        ..Script::ok()
    });
    let sweep = within(
        "sweep refused",
        STAGE,
        refused
            .engine
            .sweep_expired(SCRIPTED_OWNER, SCRIPTED_DEADLINE + 1, Duration::ZERO),
    )
    .await;
    assert_eq!(
        (sweep.listed, sweep.removed, sweep.failed),
        (true, 0, 1),
        "a removal the engine refused was counted as done: {sweep:?}"
    );
}

/// The edges of the deadline: a running container is due at its deadline, not a second before; one that ended is due when
/// the time it is kept has passed, not a second before.
#[tokio::test(flavor = "multi_thread")]
async fn the_sweep_acts_at_the_deadline_and_not_a_second_before() {
    let running = ScriptedEngine::start(Script::ok());
    for (clock, expected) in [
        (SCRIPTED_DEADLINE - 1, 0),
        (SCRIPTED_DEADLINE, 1),
        (SCRIPTED_DEADLINE + 1, 1),
    ] {
        let sweep = within(
            "sweep running",
            STAGE,
            running
                .engine
                .sweep_expired(SCRIPTED_OWNER, clock, Duration::from_secs(300)),
        )
        .await;
        assert_eq!(sweep.removed, expected, "running, at {clock}");
    }
    let exited = ScriptedEngine::start(Script {
        state: "exited",
        ..Script::ok()
    });
    for (clock, expected) in [
        (SCRIPTED_DEADLINE, 0),
        (SCRIPTED_DEADLINE + 299, 0),
        (SCRIPTED_DEADLINE + 300, 1),
    ] {
        let sweep = within(
            "sweep exited",
            STAGE,
            exited
                .engine
                .sweep_expired(SCRIPTED_OWNER, clock, Duration::from_secs(300)),
        )
        .await;
        assert_eq!(sweep.removed, expected, "exited, at {clock}");
    }
}

/// A container whose deadline label is not a deadline is not the deadline sweep's to end, and it is counted, not hidden: the
/// age sweep takes it, and an operator sees that there is one.
#[tokio::test(flavor = "multi_thread")]
async fn a_container_with_no_deadline_is_counted_and_left_alone() {
    for bad in ["soon", "0", "4102444801", "18446744073709551615", ""] {
        let scripted = ScriptedEngine::start(Script {
            deadline: bad,
            ..Script::ok()
        });
        let sweep = within(
            "sweep",
            STAGE,
            scripted
                .engine
                .sweep_expired(SCRIPTED_OWNER, u64::MAX, Duration::ZERO),
        )
        .await;
        assert_eq!(
            (sweep.listed, sweep.removed, sweep.unlabelled),
            (true, 0, 1),
            "deadline label {bad:?}: {sweep:?}"
        );
    }
}

/// An owner or an attempt the profile refuses is never put in a filter: the scripted engine lists a container whatever it
/// is asked, so an empty answer means the engine was not asked, and a valid name beside them is the control.
#[tokio::test(flavor = "multi_thread")]
async fn names_the_profile_refuses_never_reach_the_engine() {
    let scripted = ScriptedEngine::start(Script::ok());
    let engine = &scripted.engine;
    // the control: a valid owner and attempt find the container the fake always shows
    assert_eq!(
        engine
            .find(SCRIPTED_OWNER, SCRIPTED_ATTEMPT)
            .await
            .expect("answers")
            .len(),
        1
    );
    assert_eq!(engine.list(SCRIPTED_OWNER).await.expect("answers").len(), 1);

    let owners = [
        "",
        "a,sagat.lake.owner=b",
        "line\nbreak",
        "has space",
        "tab\t",
    ];
    let long_attempt = "a".repeat(61);
    let attempts = [
        "",
        "a,b",
        "feed=1",
        "feed\n1",
        "-feed",
        "FEED",
        "../feed",
        long_attempt.as_str(),
    ];
    for owner in owners {
        assert!(
            engine.list(owner).await.expect("answers").is_empty(),
            "{owner:?}"
        );
        assert!(
            engine
                .find(owner, SCRIPTED_ATTEMPT)
                .await
                .expect("answers")
                .is_empty(),
            "{owner:?}"
        );
        let swept = engine.sweep_expired(owner, u64::MAX, Duration::ZERO).await;
        assert_eq!(
            (swept.removed, swept.failed, swept.unlabelled),
            (0, 0, 0),
            "{owner:?}"
        );
    }
    for attempt in attempts {
        assert!(
            engine
                .find(SCRIPTED_OWNER, attempt)
                .await
                .expect("answers")
                .is_empty(),
            "{attempt:?}"
        );
        assert_eq!(
            engine
                .wait(SCRIPTED_OWNER, attempt, Duration::from_secs(5))
                .await
                .expect("answers"),
            Wait::Ended,
            "{attempt:?}"
        );
        // nothing can be there, and nothing was removed: ended, with no request made
        let ended = engine.end(SCRIPTED_OWNER, attempt).await;
        assert!(
            !ended.existed && ended.confirmed && !ended.memory_killed,
            "{attempt:?}: {ended:?}"
        );
    }
    assert!(
        scripted
            .removals
            .recv_timeout(Duration::from_millis(200))
            .is_err(),
        "a refused name led to a removal"
    );
}

/// The sandbox runner refuses an engine older than the sub-path mount; what the engine says is read as `major.minor` and nothing
/// else is taken for a version.
#[tokio::test(flavor = "multi_thread")]
async fn the_api_version_is_what_the_engine_says_and_nothing_made_up() {
    for (said, expected) in [
        ("1.52", Ok((1, 52))),
        ("1.44", Ok((1, 44))),
        ("1.45", Ok((1, 45))),
    ] {
        let scripted = ScriptedEngine::start(Script {
            version: said,
            ..Script::ok()
        });
        assert_eq!(
            within("version", STAGE, scripted.engine.api_version()).await,
            expected,
            "{said}"
        );
    }
    for garbage in ["", "latest", "1", "1.45.2"] {
        let scripted = ScriptedEngine::start(Script {
            version: garbage,
            ..Script::ok()
        });
        assert_eq!(
            within("version", STAGE, scripted.engine.api_version()).await,
            Err(EngineError::Unavailable),
            "{garbage:?}"
        );
    }
}
