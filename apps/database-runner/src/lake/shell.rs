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
    pub(crate) fn new(settings: Settings, development: bool) -> Self {
        // the process at the socket must run as the sandbox's user: a socket replaced by another user's is not the sandbox
        let sandbox = SocketSandbox::new(settings.sandbox_socket.clone())
            .expecting_server(settings.sandbox_uid);
        let slots = SlotPool::new(settings.slots);
        Self {
            sandbox,
            settings,
            development,
            slots,
        }
    }
}

impl Launch for Production {
    fn slots(&self) -> usize {
        self.settings.slots as usize
    }

    fn free(&self) -> usize {
        self.slots().saturating_sub(self.slots.in_use())
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
