//! The sweep of attempt directories that nobody removed (feature 035, T035-035, D035-123 slice D, D035-128).
//!
//! The driver keeps an attempt's directory when the container may still be running ([`Finished::left`]), and an attempt that is
//! dropped or aborted can leave one behind (G035-071). Each holds the relay token and the author's records, on a volume the
//! container shares, so they do not stay. The sweep removes **by age**: a directory that has not changed for longer than the
//! longest attempt can last (the step's longest timeout, the time the slot is held back after it, and a margin) cannot belong to
//! a running attempt, because its container ended by its deadline label and nothing else touches it. A live attempt is young
//! because its directory is made when it starts and its files change as it runs.
//!
//! What it touches is narrow: a **real directory** (never a link, never followed) in the work root whose name is an attempt id as
//! ADR-0079 has it. Anything else in the work root is left alone and counted. A directory that cannot be removed (the container's
//! user left in it a directory the runner cannot enter: G035-060, O035-006) is counted as failed and tried again by the next
//! pass; what the runner cannot enter, it cannot remove.
//!
//! [`Finished::left`]: super::driver::Finished::left

use std::path::Path;
use std::time::{Duration, SystemTime};

use sagat_infra_lake::sandbox_api::attempt_id_ok;
use tokio::task::spawn_blocking;
use tokio_util::sync::CancellationToken;

/// The longest a step may take: the largest `timeoutMs` the attempt input accepts (one hour).
const LONGEST_STEP: Duration = Duration::from_secs(3600);
/// How long a slot is held back after an attempt's deadline (`driver::QUARANTINE`), which is also past the container's own end.
const HELD_BACK: Duration = Duration::from_secs(90);
/// What no clock or filesystem disagreement should reach.
const MARGIN: Duration = Duration::from_secs(300);

/// A directory this old (its last change) belongs to no running attempt.
pub const SWEEP_AFTER: Duration =
    Duration::from_secs(LONGEST_STEP.as_secs() + HELD_BACK.as_secs() + MARGIN.as_secs());

/// What one pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Swept {
    /// Attempt directories removed.
    pub removed: usize,
    /// Attempt directories that could not be removed; the next pass tries again.
    pub failed: usize,
    /// Attempt directories not old enough to be nobody's.
    pub young: usize,
    /// Entries that are not attempt directories, left alone: a file, a link, a name that is not an attempt id.
    pub foreign: usize,
}

/// One pass over `work_root`. `now` and `older_than` are arguments so that a test needs no clock.
///
/// # Errors
/// The work root could not be read; nothing was touched.
pub fn sweep(work_root: &Path, now: SystemTime, older_than: Duration) -> std::io::Result<Swept> {
    let mut swept = Swept::default();
    for entry in std::fs::read_dir(work_root)? {
        let Ok(entry) = entry else {
            swept.foreign += 1;
            continue;
        };
        let is_attempt_name = entry.file_name().to_str().is_some_and(attempt_id_ok);
        // `symlink_metadata`: a link is not followed, and is not an attempt directory
        let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
            swept.foreign += 1;
            continue;
        };
        if !is_attempt_name || !metadata.file_type().is_dir() {
            swept.foreign += 1;
            continue;
        }
        let old = metadata
            .modified()
            .ok()
            .and_then(|changed| now.duration_since(changed).ok())
            .is_some_and(|age| age > older_than);
        if !old {
            swept.young += 1;
            continue;
        }
        match std::fs::remove_dir_all(entry.path()) {
            Ok(()) => swept.removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => swept.removed += 1,
            Err(_) => swept.failed += 1,
        }
    }
    Ok(swept)
}

/// Sweeps every `every` until `stop`, calling `report` with what each pass did. The removal runs on a blocking thread: a directory
/// of many files is not removed on the runtime's.
pub async fn keep_sweeping(
    work_root: std::path::PathBuf,
    every: Duration,
    older_than: Duration,
    stop: CancellationToken,
    report: impl Fn(std::io::Result<Swept>) + Send + 'static,
) {
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(every) => {}
        }
        let root = work_root.clone();
        let pass = spawn_blocking(move || sweep(&root, SystemTime::now(), older_than)).await;
        if let Ok(pass) = pass {
            report(pass);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    const OLD: &str = "0192f5aa-1111-4000-8000-000000000001";
    const OTHER: &str = "0192f5aa-1111-4000-8000-000000000002";
    const YOUNG: &str = "0192f5aa-1111-4000-8000-000000000003";

    /// Makes `<root>/<name>/in/records` with the given last change on the directory.
    fn attempt(root: &Path, name: &str, changed: SystemTime) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("in")).unwrap();
        std::fs::write(dir.join("in/records"), b"x").unwrap();
        std::fs::write(dir.join("relay.token"), b"t").unwrap();
        set_modified(&dir, changed);
    }

    fn set_modified(path: &Path, at: SystemTime) {
        std::fs::File::open(path).unwrap().set_modified(at).unwrap();
    }

    fn hours_ago(hours: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(hours * 3600)
    }

    #[test]
    fn a_directory_past_the_longest_attempt_is_removed_and_a_young_one_is_not() {
        let root = tempfile::tempdir().unwrap();
        attempt(root.path(), OLD, hours_ago(3));
        attempt(root.path(), OTHER, hours_ago(2));
        attempt(root.path(), YOUNG, SystemTime::now());

        let swept = sweep(root.path(), SystemTime::now(), SWEEP_AFTER).unwrap();
        assert_eq!(
            swept,
            Swept {
                removed: 2,
                failed: 0,
                young: 1,
                foreign: 0
            }
        );
        assert!(!root.path().join(OLD).exists());
        assert!(!root.path().join(OTHER).exists());
        assert!(root.path().join(YOUNG).join("relay.token").is_file());
    }

    #[test]
    fn the_age_is_measured_from_the_last_change_and_the_edge_is_young() {
        let root = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let threshold = Duration::from_secs(1000);
        attempt(root.path(), OLD, now - threshold);
        attempt(root.path(), OTHER, now - threshold - Duration::from_secs(1));
        let swept = sweep(root.path(), now, threshold).unwrap();
        assert_eq!(
            swept.young, 1,
            "exactly as old as the threshold is not older than it"
        );
        assert_eq!(swept.removed, 1);
        assert!(root.path().join(OLD).exists());
        assert!(!root.path().join(OTHER).exists());
    }

    #[test]
    fn the_threshold_is_past_the_longest_step_the_hold_back_and_a_margin() {
        assert_eq!(SWEEP_AFTER, Duration::from_secs(3600 + 90 + 300));
        assert!(
            SWEEP_AFTER > Duration::from_secs(3600),
            "a running attempt of the longest step is not swept"
        );
    }

    #[test]
    fn only_attempt_directories_are_touched_and_a_link_is_never_followed() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("precious"), b"keep").unwrap();
        // an old link with an attempt's name, to a directory outside
        std::os::unix::fs::symlink(outside.path(), root.path().join(OLD)).unwrap();
        // an old file with an attempt's name, an old directory with another name, a hidden file
        std::fs::write(root.path().join(OTHER), b"not a directory").unwrap();
        std::fs::create_dir(root.path().join("lost+found")).unwrap();
        set_modified(&root.path().join("lost+found"), hours_ago(5));
        std::fs::write(root.path().join(".lock"), b"").unwrap();

        let swept = sweep(root.path(), SystemTime::now(), SWEEP_AFTER).unwrap();
        assert_eq!(
            swept,
            Swept {
                removed: 0,
                failed: 0,
                young: 0,
                foreign: 4
            }
        );
        assert!(
            outside.path().join("precious").is_file(),
            "the target of the link is intact"
        );
        assert!(root.path().join("lost+found").is_dir());
    }

    #[test]
    fn a_directory_the_runner_cannot_enter_is_counted_and_left_for_the_next_pass() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        attempt(root.path(), OLD, hours_ago(3));
        // what a container's user can leave in the shared directory: a subdirectory only it can enter
        let locked = root.path().join(OLD).join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::write(locked.join("inner"), b"x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        set_modified(&root.path().join(OLD), hours_ago(3));
        // a process allowed to enter it anyway (root) removes it, and then there is nothing to count
        let can_enter = std::fs::read_dir(&locked).is_ok();

        let swept = sweep(root.path(), SystemTime::now(), SWEEP_AFTER).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).ok();
        if can_enter {
            assert_eq!(swept.removed, 1);
        } else {
            assert_eq!(swept.failed, 1, "{swept:?}");
            assert!(root.path().join(OLD).exists());
            // and once it can be entered, the next pass takes it
            let again = sweep(root.path(), SystemTime::now(), Duration::ZERO);
            assert_eq!(again.unwrap().removed, 1);
        }
    }

    #[test]
    fn a_work_root_that_cannot_be_read_is_an_error_and_nothing_is_touched() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("nothing-here");
        assert!(sweep(&missing, SystemTime::now(), SWEEP_AFTER).is_err());
    }

    #[tokio::test]
    async fn the_sweeper_reports_each_pass_removes_what_is_old_and_stops_when_told() {
        let root = tempfile::tempdir().unwrap();
        attempt(root.path(), OLD, hours_ago(3));
        let passes: Arc<Mutex<Vec<Swept>>> = Arc::default();
        let stop = CancellationToken::new();
        let sweeper = tokio::spawn(keep_sweeping(
            root.path().to_path_buf(),
            Duration::from_millis(30),
            SWEEP_AFTER,
            stop.clone(),
            {
                let passes = Arc::clone(&passes);
                move |pass| passes.lock().unwrap().push(pass.unwrap())
            },
        ));
        let began = std::time::Instant::now();
        while passes.lock().unwrap().is_empty() {
            assert!(
                began.elapsed() < Duration::from_secs(10),
                "no pass was reported"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!root.path().join(OLD).exists());
        assert_eq!(passes.lock().unwrap()[0].removed, 1);

        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), sweeper)
            .await
            .expect("the sweeper stops when told")
            .unwrap();
    }
}
