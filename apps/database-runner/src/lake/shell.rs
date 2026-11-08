//! What the serve loop of the lake profile needs from the runner's lake runtime (feature 035, T035-058, D035-143): how many slots are
//! free, how long a drain waits, and how a lease is run. The production implementation is the sandbox at its socket, the system's
//! resolver and a TCP connector; a test supplies a scripted sandbox. A trait object so that the runner is not generic over them.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use sagat_infra_lake::relay::{SystemResolver, TcpConnector};
use sagat_infra_lake::sandbox_api::client::SocketSandbox;
use tokio_util::sync::CancellationToken;

use super::attempt::{self, Task};
use super::blobs::LeaseBlobs;
use super::driver::{Driver, SlotPool};
use super::settings::Settings;
use crate::{RunnerError, rpc::Rpc};

/// How long the relay waits to connect to a catalog or a store.
const CONNECT: Duration = Duration::from_secs(10);

/// How long a drain lets the attempts that are running finish before it gives their leases up.
pub(crate) const DRAIN_GRACE: Duration = Duration::from_secs(30);

/// How often the work root is swept (the sandbox's own sweep runs at least once a minute, ADR-0079).
pub(crate) const SWEEP_EVERY: Duration = Duration::from_secs(60);

pub(crate) type Launched = Pin<Box<dyn Future<Output = Result<(), RunnerError>> + Send>>;

pub(crate) trait Launch: Send + Sync + 'static {
    /// How many attempts the runner can hold at once: its slots.
    fn slots(&self) -> usize;

    /// The slots nobody holds, counting the ones held back for a container that may still be running.
    fn free(&self) -> usize;

    /// How long a drain waits for the attempts that are running before it gives their leases up.
    fn drain_grace(&self) -> Duration {
        DRAIN_GRACE
    }

    /// Where the attempt directories are made, to be swept of the ones no attempt owns any more. `None` for a runtime with no
    /// directories of its own.
    fn work_root(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// How often the work root is swept.
    fn sweep_every(&self) -> Duration {
        SWEEP_EVERY
    }

    /// Runs one lease to its report. `drain` is cancelled when the runner gives up the leases it holds.
    fn launch(&self, rpc: Rpc, task: Task, drain: CancellationToken) -> Launched;
}

/// The lake runtime of a runner: the sandbox at its socket, the system's resolver and a TCP connector.
pub(crate) struct Production {
    sandbox: SocketSandbox,
    settings: Settings,
    development: bool,
    slots: Arc<SlotPool>,
}

impl Production {
    /// # Errors
    /// [`RunnerError::Configuration`] when the work root is not a directory: the attempts could not be made, and the sweep would find
    /// nothing to be sure of.
    pub(crate) fn new(settings: Settings, development: bool) -> Result<Self, RunnerError> {
        if !std::fs::metadata(&settings.work_root).is_ok_and(|root| root.is_dir()) {
            return Err(RunnerError::Configuration);
        }
        // the process at the socket must run as the sandbox's user: a socket replaced by another user's is not the sandbox
        let sandbox = SocketSandbox::new(settings.sandbox_socket.clone())
            .expecting_server(settings.sandbox_uid);
        let slots = SlotPool::new(settings.slots);
        Ok(Self {
            sandbox,
            settings,
            development,
            slots,
        })
    }
}

impl Launch for Production {
    fn slots(&self) -> usize {
        self.settings.slots as usize
    }

    fn free(&self) -> usize {
        self.slots().saturating_sub(self.slots.in_use())
    }

    fn work_root(&self) -> Option<std::path::PathBuf> {
        Some(self.settings.work_root.clone())
    }

    fn launch(&self, rpc: Rpc, task: Task, drain: CancellationToken) -> Launched {
        // a driver for each lease, because the blobs are read under the lease; the slots are the runner's
        let driver = Driver::with_slots(
            self.sandbox.clone(),
            LeaseBlobs::new(rpc.clone(), task.lease.clone()),
            SystemResolver,
            TcpConnector::new(CONNECT),
            self.settings.clone(),
            self.development,
            Arc::clone(&self.slots),
        );
        Box::pin(attempt::run(rpc, task, driver, drain))
    }
}
