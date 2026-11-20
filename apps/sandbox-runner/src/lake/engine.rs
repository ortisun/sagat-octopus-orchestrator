//! The container engine, as lake mode sees it: the operations of `LakeEngine` behind a port, so that admission is tested against
//! an engine that can be made to wait, to refuse and to fail, and the real one is a thin adapter.
//!
//! Lake mode keeps no state of its own about the containers: what is running, in which slot, in which class and until when is
//! read from the labels on the containers (ADR-0079, rule 7). This is the only place it asks.

use std::future::Future;
use std::time::Duration;

use sagat_infra_sandbox::lake::engine::{
    EndedContainer, EngineError, LakeEngine, LakeInfo, Sweep, Wait,
};
use sagat_infra_sandbox::lake::{LakeSpec, container_config};

/// What admission needs from the engine.
pub trait Engine: Send + Sync + 'static {
    /// Makes and starts the container of `spec`, named after its attempt: a second launch of the attempt is
    /// [`EngineError::Exists`].
    fn launch(&self, spec: LakeSpec) -> impl Future<Output = Result<(), EngineError>> + Send;

    /// The containers of `owner`, ended or not.
    fn list(&self, owner: &str) -> impl Future<Output = Result<Vec<LakeInfo>, EngineError>> + Send;

    /// Holds for at most `hold` while the attempt's container runs.
    fn wait(
        &self,
        owner: &str,
        attempt_id: &str,
        hold: Duration,
    ) -> impl Future<Output = Result<Wait, EngineError>> + Send;

    /// Ends the attempt's container, reading the memory flag before it is removed, and says what the engine confirmed.
    fn end(&self, owner: &str, attempt_id: &str) -> impl Future<Output = EndedContainer> + Send;

    /// Ends what is past its deadline label.
    fn sweep_expired(
        &self,
        owner: &str,
        now_epoch_secs: u64,
        retain: Duration,
    ) -> impl Future<Output = Sweep> + Send;

    /// The API version the engine speaks. A lake container is confined to its attempt directory by the engine's resolution of
    /// a sub-path mount, which came with 1.45: an older engine is not one to start on.
    fn api_version(&self) -> impl Future<Output = Result<(u32, u32), EngineError>> + Send;

    /// Removes what is older than `age` by its creation, whatever its labels say: the backstop for a label that is wrong or
    /// missing.
    fn sweep(&self, owner: &str, age: Duration) -> impl Future<Output = usize> + Send;
}

/// The real engine.
pub struct DockerEngine {
    engine: LakeEngine,
}

impl DockerEngine {
    /// Connects to the engine the environment names (`DOCKER_HOST`, or the local socket).
    ///
    /// # Errors
    /// The engine's address cannot be used.
    pub fn connect() -> Result<Self, EngineError> {
        Ok(Self {
            engine: LakeEngine::connect()?,
        })
    }
}

impl Engine for DockerEngine {
    async fn launch(&self, spec: LakeSpec) -> Result<(), EngineError> {
        // a spec the profile refuses is a defect of whoever built it, and no container is made from it
        let body = container_config(&spec).map_err(|_| EngineError::Invalid)?;
        self.engine.launch(body).await
    }

    async fn list(&self, owner: &str) -> Result<Vec<LakeInfo>, EngineError> {
        self.engine.list(owner).await
    }

    async fn wait(
        &self,
        owner: &str,
        attempt_id: &str,
        hold: Duration,
    ) -> Result<Wait, EngineError> {
        self.engine.wait(owner, attempt_id, hold).await
    }

    async fn end(&self, owner: &str, attempt_id: &str) -> EndedContainer {
        self.engine.end(owner, attempt_id).await
    }

    async fn sweep_expired(&self, owner: &str, now_epoch_secs: u64, retain: Duration) -> Sweep {
        self.engine
            .sweep_expired(owner, now_epoch_secs, retain)
            .await
    }

    async fn sweep(&self, owner: &str, age: Duration) -> usize {
        self.engine.sweep(owner, age).await
    }

    async fn api_version(&self) -> Result<(u32, u32), EngineError> {
        self.engine.api_version().await
    }
}

/// An engine that is a list in memory, for the cases of admission and of the API: it can be made to wait inside a launch, to
/// fail, to refuse a removal, and it says what was asked of it.
#[cfg(test)]
pub mod fake {
    use std::collections::HashSet;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use tokio::sync::Notify;

    use super::*;

    /// How long a case lets a gate hold a call before it gives up on it.
    const GATE_LIMIT: Duration = Duration::from_secs(5);

    struct Container {
        spec: LakeSpec,
        running: bool,
        /// Whether the container carries the label of the memory it was made with.
        memory_label: bool,
    }

    #[derive(Default)]
    struct State {
        containers: Vec<Container>,
        memory_killed: HashSet<String>,
        list_fails: bool,
        launch_fails: Option<EngineError>,
        end_refuses: bool,
        launch_panics: bool,
        ended: Vec<String>,
        version: Option<(u32, u32)>,
    }

    #[derive(Default)]
    pub struct Fake {
        state: Mutex<State>,
        pub launches: AtomicUsize,
        /// When set, a launch signals `entered` and waits for `release` before it makes the container.
        gate: Mutex<bool>,
        /// The same for a wait.
        wait_gate: Mutex<bool>,
        pub entered: Notify,
        pub release: Notify,
    }

    impl Fake {
        #[must_use]
        pub fn new() -> Self {
            Self::default()
        }

        fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }

        /// Makes the next launches wait inside the engine until [`Self::release`] is notified.
        pub fn gate_launches(&self) {
            *self
                .gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        }

        /// Makes the next waits block inside the engine until [`Self::release`] is notified.
        pub fn gate_waits(&self) {
            *self
                .wait_gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        }

        pub fn ungate_waits(&self) {
            *self
                .wait_gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
            self.release.notify_waiters();
        }

        pub fn ungate_launches(&self) {
            *self
                .gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
            self.release.notify_waiters();
        }

        /// A container that is already there, as a previous process left it.
        pub fn seed(&self, spec: LakeSpec, running: bool) {
            self.state().containers.push(Container {
                spec,
                running,
                memory_label: true,
            });
        }

        /// A container from before the label existed: the engine cannot say what memory it was given.
        pub fn seed_without_memory_label(&self, spec: LakeSpec, running: bool) {
            self.state().containers.push(Container {
                spec,
                running,
                memory_label: false,
            });
        }

        pub fn set_list_fails(&self, fails: bool) {
            self.state().list_fails = fails;
        }

        pub fn set_launch_fails(&self, error: Option<EngineError>) {
            self.state().launch_fails = error;
        }

        pub fn set_launch_panics(&self, panics: bool) {
            self.state().launch_panics = panics;
        }

        pub fn set_api_version(&self, version: Option<(u32, u32)>) {
            self.state().version = version;
        }

        pub fn set_end_refuses(&self, refuses: bool) {
            self.state().end_refuses = refuses;
        }

        /// The memory limit killed this attempt's container.
        pub fn mark_memory_killed(&self, attempt_id: &str) {
            self.state().memory_killed.insert(attempt_id.to_owned());
        }

        /// Marks the container as ended by itself (it stays, as the engine keeps it).
        pub fn finish(&self, attempt_id: &str) {
            for container in &mut self.state().containers {
                if container.spec.attempt_id == attempt_id {
                    container.running = false;
                }
            }
        }

        #[must_use]
        pub fn specs(&self) -> Vec<LakeSpec> {
            self.state()
                .containers
                .iter()
                .map(|c| c.spec.clone())
                .collect()
        }

        /// Waits until a gated call is inside the engine. It does not wait for ever: a call that never gets there is a case that
        /// failed.
        pub async fn wait_entered(&self) {
            tokio::time::timeout(GATE_LIMIT, self.entered.notified())
                .await
                .expect("a call to be inside the engine");
        }

        #[must_use]
        pub fn launches_made(&self) -> usize {
            self.launches.load(Ordering::SeqCst)
        }

        #[must_use]
        pub fn alive(&self) -> usize {
            self.state().containers.iter().filter(|c| c.running).count()
        }

        #[must_use]
        pub fn ended(&self) -> Vec<String> {
            self.state().ended.clone()
        }
    }

    fn info(container: &Container) -> LakeInfo {
        LakeInfo {
            name: format!("id-{}", container.spec.attempt_id),
            attempt_id: container.spec.attempt_id.clone(),
            slot: Some(container.spec.slot),
            class: Some(container.spec.class.clone()),
            deadline_epoch_secs: Some(container.spec.deadline_epoch_secs),
            memory_bytes: container
                .memory_label
                .then_some(container.spec.limits.memory_bytes),
            running: container.running,
        }
    }

    impl Engine for Fake {
        #[allow(clippy::panic)]
        async fn launch(&self, spec: LakeSpec) -> Result<(), EngineError> {
            self.launches.fetch_add(1, Ordering::SeqCst);
            let gated = *self
                .gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if gated {
                let released = self.release.notified();
                tokio::pin!(released);
                // register as a waiter before telling the test that the launch is inside, or the release could be missed
                released.as_mut().enable();
                self.entered.notify_one();
                // a gate that nobody opens is a protection that is missing: let the call through, and the case fails on
                // its answer instead of hanging the suite
                let _ = tokio::time::timeout(GATE_LIMIT, released).await;
            }
            let mut state = self.state();
            assert!(!state.launch_panics, "a launch that panics");
            if let Some(error) = state.launch_fails {
                return Err(error);
            }
            if state
                .containers
                .iter()
                .any(|c| c.spec.attempt_id == spec.attempt_id)
            {
                return Err(EngineError::Exists);
            }
            state.containers.push(Container {
                spec,
                running: true,
                memory_label: true,
            });
            Ok(())
        }

        async fn list(&self, owner: &str) -> Result<Vec<LakeInfo>, EngineError> {
            let state = self.state();
            if state.list_fails {
                return Err(EngineError::Unavailable);
            }
            Ok(state
                .containers
                .iter()
                .filter(|c| c.spec.owner == owner)
                .map(info)
                .collect())
        }

        async fn wait(
            &self,
            owner: &str,
            attempt_id: &str,
            _hold: Duration,
        ) -> Result<Wait, EngineError> {
            let gated = *self
                .wait_gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if gated {
                let released = self.release.notified();
                tokio::pin!(released);
                released.as_mut().enable();
                self.entered.notify_one();
                // a gate that nobody opens is a protection that is missing: let the call through, and the case fails on
                // its answer instead of hanging the suite
                let _ = tokio::time::timeout(GATE_LIMIT, released).await;
            }
            let state = self.state();
            if state.list_fails {
                return Err(EngineError::Unavailable);
            }
            let running = state
                .containers
                .iter()
                .any(|c| c.spec.owner == owner && c.spec.attempt_id == attempt_id && c.running);
            Ok(if running { Wait::Running } else { Wait::Ended })
        }

        async fn end(&self, owner: &str, attempt_id: &str) -> EndedContainer {
            let mut state = self.state();
            if state.list_fails {
                return EndedContainer {
                    existed: false,
                    memory_killed: false,
                    confirmed: false,
                };
            }
            let existed = state
                .containers
                .iter()
                .any(|c| c.spec.owner == owner && c.spec.attempt_id == attempt_id);
            // there is no flag to read where there is no container
            let memory_killed = existed && state.memory_killed.contains(attempt_id);
            state.ended.push(attempt_id.to_owned());
            if state.end_refuses {
                return EndedContainer {
                    existed,
                    memory_killed,
                    confirmed: false,
                };
            }
            state
                .containers
                .retain(|c| !(c.spec.owner == owner && c.spec.attempt_id == attempt_id));
            state.memory_killed.remove(attempt_id);
            EndedContainer {
                existed,
                memory_killed,
                confirmed: true,
            }
        }

        async fn sweep_expired(&self, owner: &str, now_epoch_secs: u64, retain: Duration) -> Sweep {
            let mut state = self.state();
            if state.list_fails {
                return Sweep::default();
            }
            let before = state.containers.len();
            state.containers.retain(|c| {
                if c.spec.owner != owner {
                    return true;
                }
                let due = if c.running {
                    now_epoch_secs >= c.spec.deadline_epoch_secs
                } else {
                    now_epoch_secs >= c.spec.deadline_epoch_secs.saturating_add(retain.as_secs())
                };
                !due
            });
            Sweep {
                listed: true,
                removed: before - state.containers.len(),
                ..Sweep::default()
            }
        }

        async fn sweep(&self, _owner: &str, _age: Duration) -> usize {
            0
        }

        async fn api_version(&self) -> Result<(u32, u32), EngineError> {
            Ok(self.state().version.unwrap_or((1, 52)))
        }
    }
}
