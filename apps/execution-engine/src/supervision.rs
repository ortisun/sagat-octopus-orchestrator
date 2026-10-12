use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use sagat_control_plane::RunError;
use tokio::sync::watch;
use tokio::task::{Id, JoinSet};

use crate::loops::Health;

enum TaskOutcome {
    Returned(Result<(), RunError>),
    Panicked,
    Cancelled,
}

struct TaskExit {
    name: &'static str,
    outcome: TaskOutcome,
}

/// Critical process tasks and the handles needed to stop them at a finite shutdown deadline.
pub(super) struct CriticalTasks {
    tasks: JoinSet<Result<(), RunError>>,
    names: HashMap<Id, &'static str>,
}

impl CriticalTasks {
    pub(super) fn new() -> Self {
        Self {
            tasks: JoinSet::new(),
            names: HashMap::new(),
        }
    }

    pub(super) fn spawn<F>(&mut self, name: &'static str, task: F)
    where
        F: Future<Output = Result<(), RunError>> + Send + 'static,
    {
        let handle = self.tasks.spawn(task);
        self.names.insert(handle.id(), name);
    }

    pub(super) fn spawn_loop<F>(&mut self, name: &'static str, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.spawn(name, async move {
            task.await;
            Ok(())
        });
    }

    async fn next(&mut self) -> Option<TaskExit> {
        match self.tasks.join_next_with_id().await {
            Some(Ok((id, result))) => Some(TaskExit {
                name: self.names.remove(&id).unwrap_or("unnamed critical task"),
                outcome: TaskOutcome::Returned(result),
            }),
            Some(Err(error)) => Some(TaskExit {
                name: self
                    .names
                    .remove(&error.id())
                    .unwrap_or("unnamed critical task"),
                outcome: if error.is_panic() {
                    TaskOutcome::Panicked
                } else {
                    TaskOutcome::Cancelled
                },
            }),
            None => None,
        }
    }

    async fn drain(&mut self, grace: Duration) -> Result<(), RunError> {
        let completed = tokio::time::timeout(grace, async {
            let mut first_failure = None;
            while let Some(exit) = self.next().await {
                if first_failure.is_none() {
                    first_failure = cleanup_failure(exit);
                }
            }
            first_failure
        })
        .await;

        match completed {
            Ok(Some(error)) => Err(error),
            Ok(None) => Ok(()),
            Err(_) => {
                let mut names = self.names.values().copied().collect::<Vec<_>>();
                names.sort_unstable();
                let names = names.join(", ");
                self.tasks.abort_all();
                self.tasks.detach_all();
                self.names.clear();
                Err(RunError::Unhealthy(format!(
                    "critical tasks did not stop within the shutdown grace: {names}"
                )))
            }
        }
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.names.is_empty() && self.tasks.is_empty()
    }
}

/// Watches every critical task. A requested shutdown drains cleanly; every other task exit makes
/// probes fail, stops the whole process, and becomes a runtime error for a non-zero exit status.
pub(super) async fn supervise<S>(
    tasks: &mut CriticalTasks,
    stop_tx: watch::Sender<bool>,
    stop_rx: watch::Receiver<bool>,
    health: Arc<Health>,
    shutdown: S,
    grace: Duration,
) -> Result<(), RunError>
where
    S: Future<Output = ()>,
{
    tokio::pin!(shutdown);
    let exit = tokio::select! {
        biased;
        () = &mut shutdown => None,
        exit = tasks.next() => Some(exit),
    };
    let unexpected = exit.map(|exit| match exit {
        Some(exit) => {
            record_failure(&health, exit.name);
            unexpected_failure(exit)
        }
        None => {
            record_failure(&health, "task supervisor");
            RunError::Unhealthy("critical task supervision ended unexpectedly".into())
        }
    });

    let _ = stop_tx.send(true);
    drop(stop_rx);

    let cleanup = tasks.drain(grace).await;
    match (unexpected, cleanup) {
        (Some(error), _) => Err(error),
        (None, Err(error)) => {
            record_failure(&health, "shutdown cleanup");
            Err(error)
        }
        (None, Ok(())) => Ok(()),
    }
}

fn record_failure(health: &Health, task: &'static str) {
    health.fail_process();
    metrics::counter!("engine_critical_task_failures_total", "task" => task).increment(1);
    tracing::error!(
        task,
        runbook = "docs/runbooks/execution-engine.md#critical-task-failure",
        "a critical execution-engine task stopped; shutting down the process"
    );
}

fn unexpected_failure(exit: TaskExit) -> RunError {
    match exit.outcome {
        TaskOutcome::Returned(Ok(())) => RunError::Unhealthy(format!(
            "critical task '{}' stopped unexpectedly",
            exit.name
        )),
        TaskOutcome::Returned(Err(error)) => error,
        TaskOutcome::Panicked => {
            RunError::Unhealthy(format!("critical task '{}' panicked", exit.name))
        }
        TaskOutcome::Cancelled => RunError::Unhealthy(format!(
            "critical task '{}' was cancelled unexpectedly",
            exit.name
        )),
    }
}

fn cleanup_failure(exit: TaskExit) -> Option<RunError> {
    match exit.outcome {
        TaskOutcome::Returned(Ok(())) => None,
        TaskOutcome::Returned(Err(error)) => Some(error),
        TaskOutcome::Panicked => Some(RunError::Unhealthy(format!(
            "critical task '{}' panicked during shutdown",
            exit.name
        ))),
        TaskOutcome::Cancelled => Some(RunError::Unhealthy(format!(
            "critical task '{}' was cancelled during shutdown",
            exit.name
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use sagat_control_plane::{RunError, stopped};
    use tokio::sync::watch;

    use super::{CriticalTasks, supervise};
    use crate::loops::Health;

    #[tokio::test]
    async fn an_unexpected_loop_exit_makes_the_process_unhealthy_and_stops_its_siblings() {
        let (stop_tx, stop_rx) = watch::channel(false);
        let health = Arc::new(Health::new(Duration::from_secs(30)));
        let sibling_stopped = Arc::new(AtomicBool::new(false));
        let mut tasks = CriticalTasks::new();
        tasks.spawn("event dispatcher", async { Ok(()) });
        tasks.spawn("scheduler", {
            let sibling_stopped = Arc::clone(&sibling_stopped);
            let stop_rx = stop_rx.clone();
            async move {
                stopped(stop_rx).await;
                sibling_stopped.store(true, Ordering::SeqCst);
                Ok(())
            }
        });

        let result = supervise(
            &mut tasks,
            stop_tx,
            stop_rx,
            Arc::clone(&health),
            pending(),
            Duration::from_secs(1),
        )
        .await;

        let error = result.expect_err("an unexpected loop exit is fatal");
        assert_eq!(error.exit_code(), 1, "runtime failures exit non-zero");
        assert!(matches!(
            error,
            RunError::Unhealthy(message) if message.contains("event dispatcher")
        ));
        assert!(!health.process_alive());
        assert!(!health.scheduler_alive());
        assert!(!health.keeper_alive());
        assert!(sibling_stopped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_panicking_loop_is_named_and_causes_a_runtime_failure() {
        let (stop_tx, stop_rx) = watch::channel(false);
        let health = Arc::new(Health::new(Duration::from_secs(30)));
        let mut tasks = CriticalTasks::new();
        tasks.spawn("telemetry projector", async {
            panic!("projector defect");
            #[allow(unreachable_code)]
            Ok(())
        });

        let result = supervise(
            &mut tasks,
            stop_tx,
            stop_rx,
            Arc::clone(&health),
            pending(),
            Duration::from_secs(1),
        )
        .await;

        assert!(matches!(
            result,
            Err(RunError::Unhealthy(message))
                if message.contains("telemetry projector") && message.contains("panicked")
        ));
        assert!(!health.process_alive());
    }

    #[tokio::test]
    async fn requested_shutdown_waits_for_clean_loop_cleanup() {
        let (stop_tx, stop_rx) = watch::channel(false);
        let health = Arc::new(Health::new(Duration::from_secs(30)));
        let cleaned = Arc::new(AtomicBool::new(false));
        let mut tasks = CriticalTasks::new();
        tasks.spawn("shard keeper", {
            let cleaned = Arc::clone(&cleaned);
            let stop_rx = stop_rx.clone();
            async move {
                stopped(stop_rx).await;
                cleaned.store(true, Ordering::SeqCst);
                Ok(())
            }
        });

        let result = supervise(
            &mut tasks,
            stop_tx,
            stop_rx,
            Arc::clone(&health),
            std::future::ready(()),
            Duration::from_secs(1),
        )
        .await;

        assert!(result.is_ok(), "{result:?}");
        assert!(health.process_alive(), "a requested stop is not a failure");
        assert!(cleaned.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cleanup_that_ignores_shutdown_is_aborted_at_the_deadline() {
        let (stop_tx, stop_rx) = watch::channel(false);
        let health = Arc::new(Health::new(Duration::from_secs(30)));
        let mut tasks = CriticalTasks::new();
        tasks.spawn("stuck monitor", async {
            pending::<()>().await;
            Ok(())
        });

        let result = supervise(
            &mut tasks,
            stop_tx,
            stop_rx,
            Arc::clone(&health),
            std::future::ready(()),
            Duration::from_millis(20),
        )
        .await;

        assert!(matches!(
            result,
            Err(RunError::Unhealthy(message))
                if message.contains("did not stop") && message.contains("stuck monitor")
        ));
        assert!(!health.process_alive());
        assert!(tasks.is_empty(), "the stuck task was aborted and reaped");
    }
}
