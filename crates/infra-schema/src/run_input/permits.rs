//! The permits of the run-input validator (design 043, D043-050, D043-068, D043-071): [`RUN_INPUT_PERMITS`] per process,
//! shared among tenants by a work-conserving fair queue, awaited for at most [`RUN_INPUT_PERMIT_WAIT`].
//!
//! Under one mutex the pool keeps the permits each tenant holds and a FIFO queue of waiters. A free permit goes to the
//! waiter whose tenant holds the fewest (FIFO among equals), so a tenant alone on the process uses all four, and a
//! tenant that arrives while another holds them is served first as permits come back; nothing held is taken away. With
//! four permits that rule is also the cap of ADR-0090 decision 4: a tenant holds more than two only while no other
//! tenant waits (a waiter of a tenant holding two or more is chosen only when every waiter's tenant holds as many, and
//! two tenants holding two each leave no permit free). A waiter's guard leaves the queue, or returns the permit it was
//! granted, when its future is dropped; a tenant's entry leaves the map when it holds nothing.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use sagat_domain::identity::TenantId;
use tokio::sync::oneshot;

use super::RUN_INPUT_PERMITS;

/// How long a validation waits for a permit before it is refused as transient (`Busy`).
pub const RUN_INPUT_PERMIT_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
struct State {
    free: usize,
    held: HashMap<TenantId, usize>,
    waiters: VecDeque<Waiter>,
    next: u64,
    /// The most permits held at once since the pool was made, for the tests of the bound.
    peak: usize,
}

#[derive(Debug)]
struct Waiter {
    id: u64,
    tenant: TenantId,
    grant: oneshot::Sender<()>,
}

/// The pool of permits of one process.
#[derive(Debug, Clone)]
pub struct Permits {
    state: Arc<Mutex<State>>,
    total: usize,
    wait: Duration,
}

impl Default for Permits {
    fn default() -> Self {
        Self::new(RUN_INPUT_PERMITS, RUN_INPUT_PERMIT_WAIT)
    }
}

/// A permit, returned to the pool when dropped.
#[derive(Debug)]
pub struct Permit {
    pool: Permits,
    tenant: TenantId,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.pool.release(self.tenant);
    }
}

/// The permit was not granted within the wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

/// The waiter's place in the queue: leaves the queue, or gives back what it was granted, when its future is dropped.
struct Waiting<'a> {
    pool: &'a Permits,
    id: u64,
    tenant: TenantId,
    armed: bool,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let granted = {
            let mut state = self.pool.lock();
            match state.waiters.iter().position(|waiter| waiter.id == self.id) {
                Some(at) => {
                    state.waiters.remove(at);
                    false
                }
                None => true,
            }
        };
        if granted {
            // the grant was sent but never taken: the permit goes back
            self.pool.release(self.tenant);
        }
    }
}

impl Permits {
    /// A pool of `total` permits whose waiters wait at most `wait`.
    #[must_use]
    pub fn new(total: usize, wait: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                free: total,
                ..State::default()
            })),
            total,
            wait,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // a panic under this lock leaves counts that are still consistent: every change is one statement
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A permit for `tenant`, within the pool's wait.
    ///
    /// # Errors
    /// [`Busy`] when none was granted within the wait.
    pub async fn acquire(&self, tenant: TenantId) -> Result<Permit, Busy> {
        let (id, receiver) = {
            let mut state = self.lock();
            if state.free > 0 && state.waiters.is_empty() {
                state.free -= 1;
                *state.held.entry(tenant).or_default() += 1;
                state.peak = state.peak.max(self.total - state.free);
                return Ok(Permit {
                    pool: self.clone(),
                    tenant,
                });
            }
            let id = state.next;
            state.next += 1;
            let (grant, receiver) = oneshot::channel();
            state.waiters.push_back(Waiter { id, tenant, grant });
            (id, receiver)
        };
        let mut waiting = Waiting {
            pool: self,
            id,
            tenant,
            armed: true,
        };
        match tokio::time::timeout(self.wait, receiver).await {
            Ok(Ok(())) => {
                waiting.armed = false;
                Ok(Permit {
                    pool: self.clone(),
                    tenant,
                })
            }
            // timed out (or the pool went away): the guard leaves the queue or returns a grant that raced the clock
            _ => Err(Busy),
        }
    }

    fn release(&self, tenant: TenantId) {
        let mut state = self.lock();
        if let Some(held) = state.held.get_mut(&tenant) {
            *held = held.saturating_sub(1);
            if *held == 0 {
                state.held.remove(&tenant);
            }
        }
        state.free += 1;
        Self::dispatch(&mut state, self.total);
    }

    /// Grants the free permits to the waiters whose tenants hold the fewest, FIFO among equals.
    fn dispatch(state: &mut State, total: usize) {
        while state.free > 0 && !state.waiters.is_empty() {
            let mut chosen = 0;
            let mut fewest = usize::MAX;
            for (at, waiter) in state.waiters.iter().enumerate() {
                let held = state.held.get(&waiter.tenant).copied().unwrap_or(0);
                if held < fewest {
                    fewest = held;
                    chosen = at;
                }
            }
            let Some(waiter) = state.waiters.remove(chosen) else {
                return;
            };
            state.free -= 1;
            *state.held.entry(waiter.tenant).or_default() += 1;
            if waiter.grant.send(()).is_err() {
                // its future is gone and its guard could not find it in the queue any more: take it back here
                state.free += 1;
                if let Some(held) = state.held.get_mut(&waiter.tenant) {
                    *held -= 1;
                    if *held == 0 {
                        state.held.remove(&waiter.tenant);
                    }
                }
                continue;
            }
            state.peak = state.peak.max(total - state.free);
        }
    }

    /// The permits `tenant` holds now.
    #[must_use]
    pub fn held_by(&self, tenant: TenantId) -> usize {
        self.lock().held.get(&tenant).copied().unwrap_or(0)
    }

    /// The waiters in the queue now.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.lock().waiters.len()
    }

    /// The tenants with an entry in the map (holding a permit) now.
    #[must_use]
    pub fn tenants(&self) -> usize {
        self.lock().held.len()
    }

    /// The most permits held at once since the pool was made.
    #[must_use]
    pub fn peak(&self) -> usize {
        self.lock().peak
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    async fn settle() {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    /// A tenant alone on the process holds all four permits at four concurrent validations (work-conserving), and the
    /// fifth waits (T043-21).
    #[tokio::test]
    async fn a_tenant_alone_holds_every_permit_and_no_more() {
        let pool = Permits::new(4, Duration::from_secs(30));
        let tenant = TenantId::generate();
        let mut held = Vec::new();
        for _ in 0..4 {
            held.push(pool.acquire(tenant).await.unwrap());
        }
        assert_eq!(pool.held_by(tenant), 4);
        let pool2 = pool.clone();
        let fifth = tokio::spawn(async move { pool2.acquire(tenant).await.map(|_| ()) });
        settle().await;
        assert_eq!(pool.waiting(), 1);
        assert_eq!(pool.peak(), 4);
        drop(held.pop());
        fifth.await.unwrap().unwrap();
        assert_eq!(pool.peak(), 4, "never more than four at once");
    }

    /// A second tenant that arrives while the first holds every permit is served before the first tenant's next
    /// waiter, and nothing the first holds is taken away (T043-21, D043-068).
    #[tokio::test]
    async fn a_newcomer_is_served_before_the_holders_next_waiter() {
        let pool = Permits::new(4, Duration::from_secs(30));
        let first = TenantId::generate();
        let second = TenantId::generate();
        let mut held: Vec<Permit> = Vec::new();
        for _ in 0..4 {
            held.push(pool.acquire(first).await.unwrap());
        }
        let (order_tx, mut order_rx) = tokio::sync::mpsc::unbounded_channel();
        let spawn = |tenant: TenantId, label: &'static str| {
            let pool = pool.clone();
            let order = order_tx.clone();
            tokio::spawn(async move {
                let permit = pool.acquire(tenant).await.unwrap();
                order.send(label).unwrap();
                // hold it until the test ends the task
                std::future::pending::<()>().await;
                drop(permit);
            })
        };
        let first_next = spawn(first, "first");
        settle().await;
        let second_waiter = spawn(second, "second");
        settle().await;
        assert_eq!(pool.waiting(), 2);
        assert_eq!(pool.held_by(first), 4, "nothing held is taken away");
        drop(held.pop());
        settle().await;
        assert_eq!(order_rx.recv().await, Some("second"));
        assert_eq!(pool.held_by(second), 1);
        drop(held.pop());
        settle().await;
        assert_eq!(order_rx.recv().await, Some("first"));
        first_next.abort();
        second_waiter.abort();
    }

    /// A future dropped while it waits leaves the queue; one dropped after its grant returns the permit (T043-21).
    #[tokio::test]
    async fn a_dropped_waiter_leaves_the_queue_and_a_dropped_grant_returns_its_permit() {
        let pool = Permits::new(1, Duration::from_secs(30));
        let tenant = TenantId::generate();
        let held = pool.acquire(tenant).await.unwrap();
        {
            let waiting = pool.acquire(tenant);
            tokio::pin!(waiting);
            assert!(futures_poll_once(waiting.as_mut()).await.is_none());
            assert_eq!(pool.waiting(), 1);
        }
        assert_eq!(pool.waiting(), 0, "the dropped waiter left the queue");

        // granted, then dropped before it was polled again: the permit comes back
        let waiting = pool.acquire(tenant);
        let mut waiting = Box::pin(waiting);
        assert!(futures_poll_once(waiting.as_mut()).await.is_none());
        drop(held);
        assert_eq!(pool.held_by(tenant), 1, "granted to the waiter");
        drop(waiting);
        assert_eq!(pool.held_by(tenant), 0, "the dropped grant came back");
        assert_eq!(
            pool.tenants(),
            0,
            "an entry that holds nothing leaves the map"
        );
        pool.acquire(tenant).await.unwrap();
    }

    /// No permit within the wait is `Busy`, and the waiter is gone from the queue.
    #[tokio::test]
    async fn no_permit_within_the_wait_is_busy() {
        let pool = Permits::new(1, Duration::from_millis(20));
        let tenant = TenantId::generate();
        let _held = pool.acquire(tenant).await.unwrap();
        assert_eq!(pool.acquire(TenantId::generate()).await.unwrap_err(), Busy);
        assert_eq!(pool.waiting(), 0);
    }

    async fn futures_poll_once<F: std::future::Future + Unpin>(future: F) -> Option<F::Output> {
        let mut future = future;
        std::future::poll_fn(|cx| {
            std::task::Poll::Ready(match std::pin::Pin::new(&mut future).poll(cx) {
                std::task::Poll::Ready(output) => Some(output),
                std::task::Poll::Pending => None,
            })
        })
        .await
    }
}
