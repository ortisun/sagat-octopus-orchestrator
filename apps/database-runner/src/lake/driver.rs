//! One lake attempt, from a lease's input to what is reported (feature 035, T035-035, D035-123 slice C, D035-126, D035-127).
//!
//! The driver is the join: it reads the lease input ([`AttemptInput`]), takes a slot, prepares the attempt
//! ([`prepare`](super::prepare::prepare)), builds the relay for the user of the slot, runs the attempt behind the sandbox port
//! ([`run_attempt`]) and says what to report. It does not talk to the control plane: the connection it is given is already
//! resolved (two-phase, fenced and audited by the server), the signals it is given are the lease's cancellation and loss, and
//! the report it returns is for the caller to send. That keeps what is decided here (which code, whether the directory may be
//! removed, whether the slot is free again) testable against a scripted host on the real relay socket.
//!
//! **What this decides.**
//!
//! - *Time.* The attempt ends at `now + the step's timeout` (the strictest of the step's and the connection's, [`effective`]),
//!   **staging included**: a blob that stalls cannot hold a slot and the credentials past it. The lease's own end is not a bound,
//!   because a lease is renewed by its heartbeats; a lease that is lost is the `lease_lost` signal, and a cancellation is
//!   `cancelled`, and both end the staging as they end the attempt.
//! - *Before anything is made:* an input that is not an attempt input, no free slot and a slot with no user fail with their
//!   code, and say why in [`Finished::cause`].
//! - *The class:* an attempt whose staged input is above `largeAboveBytes` asks for the `large` class, when the settings name one.
//! - *After the attempt:* the request, which holds the credentials, is dropped (zeroised) the moment the attempt is over. The
//!   directory is removed **only if the sandbox confirmed the container gone**; if it did not, the container may still have it
//!   mounted and its credentials are good until its deadline, so the directory is kept for the sweep and the caller is told
//!   where it is. A removal that fails is tried once more before that.
//! - *The slot* is free again when the container is known to be gone. If it is not, the slot is **held back** until the
//!   container's deadline has passed and the sandbox's sweep has had its turn: its user may still be in use, and the sandbox
//!   would refuse the slot to an unrelated attempt.
//! - *A dropped future.* If the caller drops [`Driver::run`] while the container may be running, the container is not killed
//!   (that needs an `await`; the backstop is the deadline label the sandbox put on it), but the directory is kept and the slot
//!   is held back exactly as above.
//! - *What the host reports* is believed only as far as it matches the request: a receipt for another effect key, another ref
//!   or another mode is `LAKE_HOST_PROTOCOL`, not a commit of this step.
//! - *Refusals of the sandbox* that no retry changes (a deadline above the node's ceiling, a class the node does not have, an
//!   attempt directory it does not accept) are the step's configuration against the node's: `DATABASE_CONFIGURATION_INVALID`,
//!   not the retryable `LAKE_SANDBOX_UNAVAILABLE`. A full node and a slot in use stay retryable.
//!
//! A start refused for the slot is **not** retried here with another one: the code is retryable, the step is retried by the
//! server, and the next attempt takes the next slot (D035-126).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use sagat_domain::database::DatabaseFailure;
use sagat_infra_lake::attempt::{AttemptOutcome, LakeSandbox, Plan, Signals, run_attempt};
use sagat_infra_lake::control::{ControlConfig, ControlFailure};
use sagat_infra_lake::protocol::{
    AttemptInput, NodeLimits, ProbeInput, ProbeReport, Receipt, WriteMode,
};
use sagat_infra_lake::relay::{Connector, Relay, RelayDenials, Resolver};
use sagat_infra_lake::sandbox_api::{Class, Reason, uid_for_slot};
use sagat_infra_lake::staging::AttemptDir;
use sagat_infra_lake::supervisor::{
    AttemptFacts, Disposition, Ended, Failure, ProbeDisposition, dispose_probe,
};
use sagat_telemetry::lake::LakeSandboxExit;
use tokio::time::Instant;

use super::connection::Converted;
use super::prepare::{
    Attempt, Blobs, PrepareError, Prepared, effective, host_check_timeout_ms, prepare,
    prepare_probe, probe_timeout_ms,
};
use super::settings::Settings;

/// After an attempt's deadline, how long a slot whose container may still be running is held back. The container ends by its
/// own label (the deadline and the sandbox's grace) at the sandbox's sweep, which runs at least once a minute (ADR-0079), so a
/// minute and a half is past both.
const QUARANTINE: Duration = Duration::from_secs(90);

/// The slots of the lake runner: which of the containers' users are in use. A slot is taken for the whole attempt and is
/// handed back when the guard is dropped. Taken in turn, not always the lowest: the user of the container that has just
/// ended is not the user of the next one.
#[derive(Debug)]
pub struct SlotPool {
    count: u32,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    taken: BTreeSet<u32>,
    next: u32,
    /// How many of the taken slots are a probe's (D035-150).
    probes: usize,
}

impl SlotPool {
    #[must_use]
    pub fn new(count: u32) -> Arc<Self> {
        Arc::new(Self {
            count,
            state: Mutex::new(State::default()),
        })
    }

    /// The next free slot, or `None` when every one is in use.
    #[must_use]
    pub fn take(self: &Arc<Self>) -> Option<SlotGuard> {
        self.take_slot(None)
    }

    /// The next free slot for a probe, or `None` when every one is in use **or probes already hold `cap` of them**. A probe is
    /// an attempt that starts a container for a connection a tenant chose, and the server limits how many are in flight for a
    /// tenant and for a namespace, not how many a node holds: the cap is what keeps slots for the writes.
    #[must_use]
    pub fn take_probe(self: &Arc<Self>, cap: usize) -> Option<SlotGuard> {
        self.take_slot(Some(cap))
    }

    fn take_slot(self: &Arc<Self>, probe_cap: Option<usize>) -> Option<SlotGuard> {
        let mut state = self.state.lock().ok()?;
        if probe_cap.is_some_and(|cap| state.probes >= cap) {
            return None;
        }
        let slot = (0..self.count)
            .map(|offset| (state.next + offset) % self.count)
            .find(|slot| !state.taken.contains(slot))?;
        state.taken.insert(slot);
        state.next = (slot + 1) % self.count;
        let probe = probe_cap.is_some();
        if probe {
            state.probes += 1;
        }
        Some(SlotGuard {
            pool: Arc::clone(self),
            slot,
            probe,
        })
    }

    /// How many slots probes may hold at once: half of them, and one when there is only one. A write always finds a slot that
    /// probes could not take.
    #[must_use]
    pub fn probe_cap(&self) -> usize {
        (self.count as usize / 2).max(1)
    }

    /// How many are in use.
    #[must_use]
    pub fn in_use(&self) -> usize {
        self.state.lock().map_or(0, |state| state.taken.len())
    }
}

/// A slot taken from the pool.
#[derive(Debug)]
pub struct SlotGuard {
    pool: Arc<SlotPool>,
    slot: u32,
    /// The slot was taken for a probe, and counts as one until it is given back.
    probe: bool,
}

impl SlotGuard {
    #[must_use]
    pub const fn number(&self) -> u32 {
        self.slot
    }

    /// Keeps the slot taken until `until`, then gives it back. For a container that may still be running as this slot's user:
    /// the sandbox would refuse the slot as in use, and that refusal would be charged to an unrelated attempt.
    fn quarantine(self, until: Instant) {
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    tokio::time::sleep_until(until).await;
                    drop(self);
                });
            }
            // nothing to wait in: the slot stays taken for the life of the process, which is the safe side of it
            Err(_) => std::mem::forget(self),
        }
    }
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.pool.state.lock() {
            state.taken.remove(&self.slot);
            if self.probe {
                state.probes = state.probes.saturating_sub(1);
            }
        }
    }
}

/// What to tell the control plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// `CompleteStep` with this receipt, rebuilt from the host's typed fields.
    Complete(Box<Receipt>),
    /// `CompleteStep` with the result of a probe (D035-150), the connection and the revision of which are the probe's own. It is
    /// not a receipt: nothing was written. A connection that does not work is in it, as a check that failed.
    Probe(Box<ProbeReport>),
    /// `FailStep`.
    Fail(Failure),
    /// Nothing: the lease is gone.
    Abandon,
}

/// Why an attempt never reached a container. For the runner's own logs and metrics: the code of the failure is the contract's,
/// and several causes share one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// The lease input is not an attempt input.
    Input,
    /// Cancelled, the lease lost, or the step's time used up, while the sources were being staged.
    Over,
    /// Every slot is in use.
    NoSlot,
    /// The slot has no user (it is outside the range).
    NoUser,
    /// The attempt could not be prepared.
    Prepare(PrepareError),
    /// The relay could not be built.
    Relay,
}

/// How an attempt ended, for the report and for the runner's own books.
#[derive(Debug)]
pub struct Finished {
    pub report: Report,
    /// The attempt was a probe **that was run**, which writes nothing: whatever it did, it is not counted as a write. It is
    /// `false` for an attempt that ended before a container was asked for, whatever the attempt was: [`cause`](Self::cause) says
    /// that one ended early, and it is what keeps such an attempt out of the metrics of the writes too.
    pub probe: bool,
    /// Why no container was asked for, when none was.
    pub cause: Option<Cause>,
    /// The request line was written to the host. Before that, nothing can have been written to the table.
    pub request_sent: bool,
    /// The sandbox confirmed the container gone (`true` when there was none).
    pub container_ended: bool,
    /// What the relay refused.
    pub relay_denials: RelayDenials,
    /// Why the sandbox said no, when it answered and said no.
    pub sandbox_refusal: Option<Reason>,
    /// How the container ended, as the closed reason of the metrics; `None` when none was asked for.
    pub exit: Option<LakeSandboxExit>,
    /// Where the attempt directory was left, when it could not be removed: because the container was not confirmed gone, or
    /// because something in it could not be removed. The caller counts it and the sweep takes it.
    pub left: Option<PathBuf>,
}

impl Finished {
    fn before(report: Report, cause: Cause) -> Self {
        Self {
            report,
            probe: false,
            cause: Some(cause),
            request_sent: false,
            container_ended: true,
            relay_denials: RelayDenials::default(),
            sandbox_refusal: None,
            exit: None,
            left: None,
        }
    }

    fn failed(code: DatabaseFailure, cause: Cause) -> Self {
        Self::before(
            Report::Fail(Failure {
                code,
                column: None,
                reason: None,
            }),
            cause,
        )
    }

    fn abandoned(cause: Cause) -> Self {
        Self::before(Report::Abandon, cause)
    }
}

/// What the request said, to hold the host's receipt to it.
struct Expected {
    effect_key: String,
    reference: String,
}

impl Expected {
    /// A receipt the host may report for this request: an Iceberg commit, of the effect key and the ref that were asked, in the
    /// mode that was asked. Anything else is a host that does not keep the protocol.
    fn accepts(&self, receipt: &Receipt) -> bool {
        match receipt {
            Receipt::Iceberg(committed) => {
                committed.effect_key.as_str() == self.effect_key
                    && committed.reference.as_str() == self.reference
                    && committed.mode == WriteMode::Append
            }
            Receipt::ObjectStorage(_) => false,
        }
    }
}

/// What an attempt is held to once the host has answered: the request of a write, or the lease of a probe.
enum Kind {
    Write(Expected),
    Probe(ProbeInput),
}

/// A host that answered with something the request did not ask for does not keep the protocol.
const fn host_broke_the_protocol() -> Failure {
    Failure {
        code: DatabaseFailure::LakeHostProtocol,
        column: None,
        reason: None,
    }
}

/// A refusal of the sandbox that no retry changes is the configuration, and is reported as such.
fn by_refusal(failure: Failure, refusal: Option<Reason>) -> Failure {
    match (failure.code, refusal) {
        (
            DatabaseFailure::LakeSandboxUnavailable,
            Some(Reason::Deadline | Reason::Class | Reason::Directory),
        ) => Failure {
            code: DatabaseFailure::Configuration,
            ..failure
        },
        _ => failure,
    }
}

/// What an attempt holds from the moment its container may be running until it is settled. **Dropped without being settled**
/// (the caller dropped the future) it leaves the directory where it is and holds the slot back: the container is not killed by
/// a drop, and it ends at the deadline the sandbox put on it.
struct Running {
    directory: AttemptDir,
    slot: Option<SlotGuard>,
    deadline: Instant,
    quarantine: Duration,
    settled: bool,
}

impl Running {
    /// The attempt is over and the sandbox has said whether the container is gone. Returns where the directory was left.
    fn settle(mut self, container_ended: bool) -> Option<PathBuf> {
        self.settled = true;
        let left = if container_ended {
            // once more if it fails: the handle is still ours, and what stays is the sweep's
            match self.directory.remove().or_else(|_| self.directory.remove()) {
                Ok(()) => None,
                Err(_) => Some(self.directory.keep()),
            }
        } else {
            Some(self.directory.keep())
        };
        if let Some(slot) = self.slot.take() {
            if container_ended {
                drop(slot);
            } else {
                slot.quarantine(self.deadline + self.quarantine);
            }
        }
        left
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        self.directory.keep();
        if let Some(slot) = self.slot.take() {
            slot.quarantine(self.deadline + self.quarantine);
        }
    }
}

/// What an admitted attempt is to do: a write, or a probe of a connection (D035-150).
enum Work {
    Write(AttemptInput),
    Probe(ProbeInput),
}

/// An attempt that has a readable input and a slot, and has not asked for its connection yet (D035-143).
pub struct Admitted {
    work: Work,
    slot: SlotGuard,
    peer_uid: u32,
    /// When it was admitted: the time of a probe is counted from here, and not from when its connection arrived.
    at: Instant,
}

impl Admitted {
    /// The slot it holds.
    #[must_use]
    pub const fn slot(&self) -> u32 {
        self.slot.number()
    }

    /// The connection and the revision a probe was leased for, which the connection it is given must be; `None` for a write.
    #[must_use]
    pub fn leased_probe(&self) -> Option<(String, u32)> {
        match &self.work {
            Work::Probe(input) => Some((input.connection().to_owned(), input.revision())),
            Work::Write(_) => None,
        }
    }
}

/// The lake driver of one runner.
pub struct Driver<S, B, R, C> {
    sandbox: S,
    blobs: B,
    resolver: R,
    connector: C,
    settings: Settings,
    development: bool,
    slots: Arc<SlotPool>,
    /// The user of the container in a slot: [`uid_for_slot`] in production, the test's own user in a test (the scripted host
    /// is in the test's process, so the kernel reports that user as the peer).
    peer_for_slot: fn(u32) -> Option<u32>,
    quarantine: Duration,
}

impl<S, B, R, C> Driver<S, B, R, C>
where
    S: LakeSandbox,
    B: Blobs,
    R: Resolver + Clone,
    C: Connector + Clone,
{
    /// A driver for a production runner: the containers' users are `40000 + slot`, and the pool is as large as the settings say.
    #[must_use]
    pub fn new(
        sandbox: S,
        blobs: B,
        resolver: R,
        connector: C,
        settings: Settings,
        development: bool,
    ) -> Self {
        let slots = SlotPool::new(settings.slots);
        Self::with_slots(
            sandbox,
            blobs,
            resolver,
            connector,
            settings,
            development,
            slots,
        )
    }

    /// A driver that takes its slots from `slots`, which other drivers may share: the blobs are read under one lease, so the shell
    /// makes a driver for each lease, and the slots belong to the runner (D035-143).
    #[must_use]
    pub fn with_slots(
        sandbox: S,
        blobs: B,
        resolver: R,
        connector: C,
        settings: Settings,
        development: bool,
        slots: Arc<SlotPool>,
    ) -> Self {
        Self {
            sandbox,
            blobs,
            resolver,
            connector,
            settings,
            development,
            slots,
            peer_for_slot: uid_for_slot,
            quarantine: QUARANTINE,
        }
    }

    /// A driver whose containers' users are the test's own, and whose held-back slots are free at once (the scripted host is in the
    /// test's process, so the kernel reports that user as the peer).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn for_tests(mut self, peer_for_slot: fn(u32) -> Option<u32>) -> Self {
        self.peer_for_slot = peer_for_slot;
        self.quarantine = Duration::ZERO;
        self
    }

    /// The slots, for the runner's metrics.
    #[must_use]
    pub const fn slots(&self) -> &Arc<SlotPool> {
        &self.slots
    }

    /// What an attempt needs before its credentials are worth disclosing: a lease input it can read, a free slot, and a user for
    /// the slot. The shell asks the control plane for the connection (which discloses and audits the secrets) only after this, so
    /// a full node or a bad input costs no disclosure. The slot is held until the returned value is run or dropped.
    ///
    /// # Errors
    /// The [`Finished`] to report: the lease input is not an attempt input, every slot is in use, or the slot has no user.
    pub fn admit(&self, input_json: &[u8]) -> Result<Admitted, Finished> {
        let Ok(input) = AttemptInput::read(input_json) else {
            return Err(Finished::failed(
                DatabaseFailure::Configuration,
                Cause::Input,
            ));
        };
        let (slot, peer_uid) = self.take_slot(false)?;
        Ok(Admitted {
            work: Work::Write(input),
            slot,
            peer_uid,
            at: Instant::now(),
        })
    }

    /// [`admit`](Self::admit) for the lease of a probe (D035-150): the input is read by the reader of a probe lease, and a probe
    /// takes a slot like any attempt, because it starts a container, **but not one of the half that the writes keep**
    /// ([`SlotPool::probe_cap`]).
    ///
    /// # Errors
    /// The [`Finished`] to report: the lease input is not a probe input, every slot is in use or probes hold their share of
    /// them, or the slot has no user.
    pub fn admit_probe(&self, input_json: &[u8]) -> Result<Admitted, Finished> {
        let Ok(input) = ProbeInput::read(input_json) else {
            return Err(Finished::failed(
                DatabaseFailure::Configuration,
                Cause::Input,
            ));
        };
        let (slot, peer_uid) = self.take_slot(true)?;
        Ok(Admitted {
            work: Work::Probe(input),
            slot,
            peer_uid,
            at: Instant::now(),
        })
    }

    /// A free slot and the user of its container, or the failure to report.
    fn take_slot(&self, probe: bool) -> Result<(SlotGuard, u32), Finished> {
        let taken = if probe {
            self.slots.take_probe(self.slots.probe_cap())
        } else {
            self.slots.take()
        };
        let Some(slot) = taken else {
            return Err(Finished::failed(
                DatabaseFailure::LakeSandboxUnavailable,
                Cause::NoSlot,
            ));
        };
        let Some(peer_uid) = (self.peer_for_slot)(slot.number()) else {
            return Err(Finished::failed(
                DatabaseFailure::Configuration,
                Cause::NoUser,
            ));
        };
        Ok((slot, peer_uid))
    }

    /// Runs one attempt (see the module). `input_json` is the lease's input and `connection` its resolved connection, which is
    /// consumed. `on_heartbeat` is called once for each beat the host sent in time and must not block.
    pub async fn run(
        &self,
        input_json: &[u8],
        connection: Converted,
        signals: &Signals,
        on_heartbeat: impl FnMut() + Send,
    ) -> Finished {
        match self.admit(input_json) {
            Ok(admitted) => {
                self.run_admitted(admitted, connection, signals, on_heartbeat)
                    .await
            }
            Err(finished) => finished,
        }
    }

    /// Runs an attempt that was admitted, with the connection the control plane resolved for it.
    pub async fn run_admitted(
        &self,
        admitted: Admitted,
        connection: Converted,
        signals: &Signals,
        on_heartbeat: impl FnMut() + Send,
    ) -> Finished {
        let Admitted {
            work,
            slot,
            peer_uid,
            at,
        } = admitted;
        // opaque and random: it names a directory and a container and says nothing of the run or of when (ADR-0079, rule 10)
        let attempt_id = uuid::Uuid::new_v4().to_string();

        match work {
            Work::Write(input) => {
                let expected = Expected {
                    effect_key: input.effect_key().to_owned(),
                    reference: input.reference().to_owned(),
                };
                let limits = effective(input.limits(), connection.limits, &self.settings);

                // the step's time is the attempt's, staging included
                let deadline = Instant::now() + limits.timeout;
                let deadline_at = SystemTime::now() + limits.timeout;

                let prepared = tokio::select! {
                    biased;
                    () = signals.lease_lost.cancelled() => return Finished::abandoned(Cause::Over),
                    () = signals.cancelled.cancelled() => {
                        return Finished::failed(DatabaseFailure::Cancelled, Cause::Over);
                    }
                    () = tokio::time::sleep_until(deadline) => {
                        return Finished::failed(DatabaseFailure::Timeout, Cause::Over);
                    }
                    prepared = prepare(
                        input,
                        connection,
                        &self.blobs,
                        &self.settings,
                        Attempt {
                            id: &attempt_id,
                            peer_uid,
                            deadline_at,
                            development: self.development,
                        },
                    ) => prepared,
                };
                self.run_prepared(
                    prepared,
                    slot,
                    deadline,
                    signals,
                    on_heartbeat,
                    Kind::Write(expected),
                )
                .await
            }
            Work::Probe(input) => {
                // the lease names the time of one check; the attempt has three of them and a margin, and the connection may
                // lower it (nothing raises it)
                let limits = effective(
                    NodeLimits {
                        max_record_bytes: 1,
                        max_records: 1,
                        timeout_ms: probe_timeout_ms(input.check_timeout_ms()),
                    },
                    connection.limits,
                    &self.settings,
                );
                // counted from the admission: the step has its time from the moment it was leased, and a connection that took
                // long to resolve has used some of it. What is left is what the host is told it has, a third for each check.
                let deadline = at + limits.timeout;
                let left = deadline.saturating_duration_since(Instant::now());
                let deadline_at = SystemTime::now() + left;
                let check_timeout_ms = host_check_timeout_ms(input.check_timeout_ms(), left);

                // a probe stages nothing, so its preparation does not wait; the signals are still looked at first
                let prepared = tokio::select! {
                    biased;
                    () = signals.lease_lost.cancelled() => return Finished::abandoned(Cause::Over),
                    () = signals.cancelled.cancelled() => {
                        return Finished::failed(DatabaseFailure::Cancelled, Cause::Over);
                    }
                    () = tokio::time::sleep_until(deadline) => {
                        return Finished::failed(DatabaseFailure::Timeout, Cause::Over);
                    }
                    prepared = async {
                        prepare_probe(
                            &input,
                            check_timeout_ms,
                            connection,
                            &self.settings,
                            Attempt {
                                id: &attempt_id,
                                peer_uid,
                                deadline_at,
                                development: self.development,
                            },
                        )
                    } => prepared,
                };
                self.run_prepared(
                    prepared,
                    slot,
                    deadline,
                    signals,
                    on_heartbeat,
                    Kind::Probe(input),
                )
                .await
            }
        }
    }

    /// What is the same for what is asked of the host, once it is prepared: the relay, the class, the container, and how the
    /// attempt ended. What differs is how the host's answer is held to the request: a write's receipt must be for the effect key
    /// and the ref that were asked, and a probe's result for the connection and the revision that were leased.
    async fn run_prepared(
        &self,
        prepared: Result<Prepared, PrepareError>,
        slot: SlotGuard,
        deadline: Instant,
        signals: &Signals,
        on_heartbeat: impl FnMut() + Send,
        kind: Kind,
    ) -> Finished {
        let Prepared {
            directory,
            request,
            relay,
            staged_bytes,
        } = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return Finished::failed(error.code(), Cause::Prepare(error)),
        };
        let Ok(relay) = Relay::new(relay, self.resolver.clone(), self.connector.clone()) else {
            return Finished::failed(DatabaseFailure::Configuration, Cause::Relay);
        };
        let class = if self
            .settings
            .large_above_bytes
            .is_some_and(|above| staged_bytes > above)
        {
            Class::Large
        } else {
            Class::Small
        };

        let plan = Plan {
            request: &request,
            deadline,
            slot: slot.number(),
            class,
            host_start: self.settings.host_start,
            control: ControlConfig::default(),
            kill_limit: self.settings.kill_limit,
        };
        let running = Running {
            directory,
            slot: Some(slot),
            deadline,
            quarantine: self.quarantine,
            settled: false,
        };
        let outcome = run_attempt(
            &self.sandbox,
            &running.directory,
            relay,
            &plan,
            signals,
            on_heartbeat,
        )
        .await;
        // the request holds the credentials: it goes as soon as the attempt is over
        drop(request);
        let exit = exit_of(&outcome);
        let left = running.settle(outcome.container_ended);

        let probe = matches!(kind, Kind::Probe(_));
        let report = match kind {
            Kind::Write(expected) => match outcome.disposition {
                Disposition::Complete(receipt) if expected.accepts(&receipt) => {
                    Report::Complete(receipt)
                }
                Disposition::Complete(_) => Report::Fail(host_broke_the_protocol()),
                Disposition::Fail(failure) => {
                    Report::Fail(by_refusal(failure, outcome.sandbox_refusal))
                }
                Disposition::Abandon => Report::Abandon,
            },
            Kind::Probe(input) => {
                // a probe that was killed ends otherwise than a write does: nothing it did can be unknown, and no write failed to fit
                let facts = AttemptFacts {
                    memory_killed: outcome.memory_killed,
                    request_sent: outcome.request_sent,
                };
                match dispose_probe(outcome.ended, facts) {
                    ProbeDisposition::Complete(result) if result.answers(&input) => {
                        Report::Probe(result)
                    }
                    // the result of another probe is not this one's, and the server would refuse it
                    ProbeDisposition::Complete(_) => Report::Fail(host_broke_the_protocol()),
                    ProbeDisposition::Fail(failure) => {
                        Report::Fail(by_refusal(failure, outcome.sandbox_refusal))
                    }
                    ProbeDisposition::Abandon => Report::Abandon,
                }
            }
        };
        Finished {
            report,
            probe,
            cause: None,
            request_sent: outcome.request_sent,
            container_ended: outcome.container_ended,
            relay_denials: outcome.relay_denials,
            sandbox_refusal: outcome.sandbox_refusal,
            exit: Some(exit),
            left,
        }
    }
}

/// How the container ended, as one of the closed reasons of the metrics. A memory kill is that whatever else the attempt did.
fn exit_of(outcome: &AttemptOutcome) -> LakeSandboxExit {
    if outcome.memory_killed {
        return LakeSandboxExit::MemoryKilled;
    }
    match &outcome.ended {
        Ended::Conversation(Ok(_)) => LakeSandboxExit::Completed,
        Ended::Conversation(Err(ControlFailure::Silent)) => LakeSandboxExit::HeartbeatMissed,
        Ended::Conversation(Err(ControlFailure::Protocol(_))) => LakeSandboxExit::ProtocolError,
        Ended::SandboxUnavailable | Ended::HostNeverConnected => LakeSandboxExit::StartFailed,
        Ended::DeadlinePassed => LakeSandboxExit::Deadline,
        Ended::Cancelled => LakeSandboxExit::Cancelled,
        Ended::LeaseLost => LakeSandboxExit::LeaseLost,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use sagat_infra_lake::attempt::SandboxError;
    use serde_json::{Value, json};

    use super::super::doubles::*;
    use super::super::fixtures::{converted, input_json, message, my_uid, settings};
    use super::*;

    fn driver(world: &World, script: Script) -> Driver<Scripted, NoBlobs, NoDns, NoNetwork> {
        Driver {
            sandbox: Scripted::new(world.work.path(), script),
            blobs: NoBlobs,
            resolver: NoDns,
            connector: NoNetwork,
            settings: settings(world.work.path()),
            development: false,
            slots: SlotPool::new(4),
            peer_for_slot: |_| Some(my_uid()),
            quarantine: Duration::ZERO,
        }
    }

    fn driver_with<X: Blobs>(
        world: &World,
        script: Script,
        blobs: X,
    ) -> Driver<Scripted, X, NoDns, NoNetwork> {
        Driver {
            sandbox: Scripted::new(world.work.path(), script),
            blobs,
            resolver: NoDns,
            connector: NoNetwork,
            settings: settings(world.work.path()),
            development: false,
            slots: SlotPool::new(4),
            peer_for_slot: |_| Some(my_uid()),
            quarantine: Duration::ZERO,
        }
    }

    async fn run(
        driver: &Driver<Scripted, NoBlobs, NoDns, NoNetwork>,
        input: &[u8],
        signals: &Signals,
    ) -> Finished {
        driver.run(input, converted(), signals, || {}).await
    }

    fn failed_with(finished: &Finished) -> DatabaseFailure {
        match &finished.report {
            Report::Fail(failure) => failure.code,
            other => panic!("not a failure: {other:?}"),
        }
    }

    // -- the pool ------------------------------------------------------------------------------------------------------

    #[test]
    fn slots_are_taken_in_turn_and_come_back_when_the_guard_goes() {
        let pool = SlotPool::new(3);
        let first = pool.take().unwrap();
        assert_eq!(first.number(), 0);
        drop(first);
        // the user of the container that has just ended is not the user of the next one
        assert_eq!(pool.take().unwrap().number(), 1);
        assert_eq!(pool.in_use(), 0, "the guard of that take was dropped");

        let (a, b, c) = (
            pool.take().unwrap(),
            pool.take().unwrap(),
            pool.take().unwrap(),
        );
        assert_eq!(
            [a.number(), b.number(), c.number()]
                .iter()
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        assert!(pool.take().is_none(), "all three are in use");
        assert_eq!(pool.in_use(), 3);
        drop(b);
        assert!(pool.take().is_some());
    }

    #[test]
    fn a_production_driver_gives_the_container_the_user_of_its_slot() {
        let world = world();
        let production = Driver::new(
            Scripted::new(world.work.path(), Script::NeverConnects),
            NoBlobs,
            NoDns,
            NoNetwork,
            settings(world.work.path()),
            false,
        );
        assert_eq!((production.peer_for_slot)(3), Some(40_003));
        assert_eq!((production.peer_for_slot)(4096), None);
        assert_eq!(production.slots.in_use(), 0);
    }

    // -- one attempt ---------------------------------------------------------------------------------------------------

    #[tokio::test]
    async fn an_attempt_that_commits_reports_the_receipt_and_leaves_nothing_behind() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let finished = run(&driver, &input_json(|_| {}), &signals()).await;

        match &finished.report {
            Report::Complete(receipt) => assert!(format!("{receipt:?}").contains("Iceberg")),
            other => panic!("not complete: {other:?}"),
        }
        assert!(!finished.probe, "a write is not a probe");
        assert!(finished.request_sent);
        assert!(finished.container_ended);
        assert_eq!(finished.left, None);
        assert!(
            entries(world.work.path()).is_empty(),
            "the directory is gone"
        );
        assert_eq!(driver.slots.in_use(), 0, "the slot is free again");
        assert_eq!(driver.sandbox.kills.load(Ordering::SeqCst), 1);

        // what the host was sent is the request of the attempt
        let request: Value =
            serde_json::from_slice(&driver.sandbox.request.lock().unwrap()).unwrap();
        assert_eq!(request["effectKey"], "run-7f3a/load/1");
        assert_eq!(request["table"]["name"], "orders");
        // and the sandbox was asked for the slot, the class and the id of the attempt, and for nothing else
        let asked = driver.sandbox.asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].slot, 0);
        assert_eq!(asked[0].class, Class::Small);
        assert!(uuid::Uuid::parse_str(&asked[0].attempt_id).is_ok());
    }

    #[tokio::test]
    async fn a_failure_of_the_host_is_reported_with_its_code_and_column() {
        let world = world();
        let driver = driver(
            &world,
            answer(failure_line("ICEBERG_SCHEMA_MISMATCH", "amount")),
        );
        let finished = run(&driver, &input_json(|_| {}), &signals()).await;
        let Report::Fail(failure) = &finished.report else {
            panic!("not a failure: {:?}", finished.report);
        };
        assert_eq!(failure.code, DatabaseFailure::IcebergSchemaMismatch);
        assert_eq!(failure.column.as_deref(), Some("amount"));
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(driver.slots.in_use(), 0);
    }

    #[tokio::test]
    async fn the_heartbeats_of_the_host_reach_the_caller() {
        let world = world();
        let driver = driver(
            &world,
            Script::Answer {
                beats: 1,
                line: receipt_line(),
            },
        );
        let beats = AtomicUsize::new(0);
        let finished = driver
            .run(&input_json(|_| {}), converted(), &signals(), || {
                beats.fetch_add(1, Ordering::SeqCst);
            })
            .await;
        assert!(matches!(finished.report, Report::Complete(_)));
        assert_eq!(beats.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_input_that_is_not_an_attempt_input_fails_before_anything_is_taken_or_made() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let finished = run(&driver, b"{\"not\":\"an input\"}", &signals()).await;
        assert_eq!(failed_with(&finished), DatabaseFailure::Configuration);
        assert_eq!(finished.cause, Some(Cause::Input));
        assert!(driver.sandbox.asked().is_empty());
        assert_eq!(driver.slots.in_use(), 0);
        assert!(entries(world.work.path()).is_empty());
    }

    #[tokio::test]
    async fn with_no_free_slot_the_attempt_fails_retryably_and_the_sandbox_is_not_asked() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let _all: Vec<_> = (0..4).map(|_| driver.slots.take().unwrap()).collect();
        let finished = run(&driver, &input_json(|_| {}), &signals()).await;
        assert_eq!(
            failed_with(&finished),
            DatabaseFailure::LakeSandboxUnavailable
        );
        assert_eq!(finished.cause, Some(Cause::NoSlot));
        assert!(DatabaseFailure::LakeSandboxUnavailable.retryable());
        assert!(driver.sandbox.asked().is_empty());
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(
            driver.slots.in_use(),
            4,
            "the slots that were taken are still taken"
        );
    }

    // -- admission, before the connection is asked for (D035-143) -------------------------------------------------------

    #[test]
    fn an_attempt_is_admitted_before_its_credentials_are_asked_for_and_holds_its_slot_until_it_is_dropped()
     {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let admitted = driver.admit(&input_json(|_| {})).expect("admitted");
        assert_eq!(driver.slots.in_use(), 1, "the slot is held from admission");
        assert!(driver.sandbox.asked().is_empty());
        drop(admitted);
        assert_eq!(
            driver.slots.in_use(),
            0,
            "an attempt whose connection could not be resolved gives its slot back"
        );
    }

    #[test]
    fn an_input_that_is_not_one_and_a_full_node_are_refused_at_admission_with_nothing_taken() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let refused = driver
            .admit(b"{\"not\":\"an input\"}")
            .err()
            .expect("refused");
        assert_eq!(refused.cause, Some(Cause::Input));
        assert_eq!(driver.slots.in_use(), 0);
        let _all: Vec<_> = (0..4).map(|_| driver.slots.take().unwrap()).collect();
        let full = driver.admit(&input_json(|_| {})).err().expect("refused");
        assert_eq!(full.cause, Some(Cause::NoSlot));
        assert_eq!(failed_with(&full), DatabaseFailure::LakeSandboxUnavailable);
        assert_eq!(driver.slots.in_use(), 4, "nothing more was taken");
    }

    #[test]
    fn the_drivers_of_two_leases_share_the_slots_of_the_runner() {
        let world = world();
        let first = driver(&world, answer(receipt_line()));
        let mut second = driver(&world, answer(receipt_line()));
        second.slots = Arc::clone(&first.slots);
        let _held: Vec<_> = (0..4)
            .map(|_| first.admit(&input_json(|_| {})).expect("admitted"))
            .collect();
        assert_eq!(
            second
                .admit(&input_json(|_| {}))
                .err()
                .expect("refused")
                .cause,
            Some(Cause::NoSlot),
            "the second lease finds the slots the first took"
        );
    }

    #[tokio::test]
    async fn an_admitted_attempt_runs_as_one_that_was_not_split() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let admitted = driver.admit(&input_json(|_| {})).expect("admitted");
        let finished = driver
            .run_admitted(admitted, converted(), &signals(), || {})
            .await;
        assert!(matches!(finished.report, Report::Complete(_)));
        assert!(finished.cause.is_none() && finished.request_sent);
        assert_eq!(driver.slots.in_use(), 0, "and the slot is back");
    }

    // -- how the container ended, for the metrics (D035-143) ----------------------------------------------------------

    #[tokio::test]
    async fn how_the_container_ended_is_one_of_the_closed_reasons() {
        use sagat_telemetry::lake::LakeSandboxExit as Exit;
        let world = world();
        let make = |script: Script| driver(&world, script);
        // the host answered, with a receipt or with a failure: the conversation ran to its end
        let attempt = make(answer(receipt_line()));
        let finished = run(&attempt, &input_json(|_| {}), &signals()).await;
        assert_eq!(finished.exit, Some(Exit::Completed));
        let attempt = make(answer(failure_line("ICEBERG_SCHEMA_MISMATCH", "amount")));
        let finished = run(&attempt, &input_json(|_| {}), &signals()).await;
        assert_eq!(finished.exit, Some(Exit::Completed));
        // a host that never connects
        let mut attempt = make(Script::NeverConnects);
        attempt.settings.host_start = Duration::from_millis(100);
        let finished = run(&attempt, &input_json(|_| {}), &signals()).await;
        assert_eq!(finished.exit, Some(Exit::StartFailed));
        // a cancel and a lost lease, once the host has the request
        for (cancelled, expected) in [(true, Exit::Cancelled), (false, Exit::LeaseLost)] {
            let attempt = make(Script::Silent);
            let signals = signals();
            let ends = if cancelled {
                signals.cancelled.clone()
            } else {
                signals.lease_lost.clone()
            };
            let requested = std::sync::Arc::clone(&attempt.sandbox.requested);
            let ending = tokio::spawn(async move {
                requested.notified().await;
                ends.cancel();
            });
            let finished = run(&attempt, &input_json(|_| {}), &signals).await;
            ending.await.unwrap();
            assert_eq!(finished.exit, Some(expected));
        }
        // the memory limit killed the container: that is the reason, whatever ended the attempt
        let mut attempt = make(Script::Silent);
        attempt.sandbox.memory_killed = true;
        let killed = signals();
        let ends = killed.cancelled.clone();
        let requested = std::sync::Arc::clone(&attempt.sandbox.requested);
        let ending = tokio::spawn(async move {
            requested.notified().await;
            ends.cancel();
        });
        let finished = run(&attempt, &input_json(|_| {}), &killed).await;
        ending.await.unwrap();
        assert_eq!(finished.exit, Some(Exit::MemoryKilled));
        // nothing was started: no container, no reason
        let attempt = make(answer(receipt_line()));
        let _all: Vec<_> = (0..4).map(|_| attempt.slots.take().unwrap()).collect();
        assert_eq!(
            run(&attempt, &input_json(|_| {}), &signals()).await.exit,
            None
        );
    }

    #[tokio::test]
    async fn a_slot_with_no_user_fails_as_a_configuration_and_gives_the_slot_back() {
        let world = world();
        let mut driver = driver(&world, answer(receipt_line()));
        driver.peer_for_slot = |_| None;
        let finished = run(&driver, &input_json(|_| {}), &signals()).await;
        assert_eq!(failed_with(&finished), DatabaseFailure::Configuration);
        assert_eq!(finished.cause, Some(Cause::NoUser));
        assert_eq!(driver.slots.in_use(), 0);
        assert!(driver.sandbox.asked().is_empty());
    }

    #[tokio::test]
    async fn a_sandbox_that_cannot_start_the_container_is_unavailable_and_nothing_stays() {
        let world = world();
        let mut driver = driver(&world, answer(receipt_line()));
        driver.sandbox.start_error = Some(SandboxError::Unavailable);
        let finished = run(&driver, &input_json(|_| {}), &signals()).await;
        assert_eq!(
            failed_with(&finished),
            DatabaseFailure::LakeSandboxUnavailable
        );
        assert!(
            !finished.request_sent,
            "the host was never given the credentials"
        );
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(driver.slots.in_use(), 0);
    }

    #[tokio::test]
    async fn a_refusal_of_the_sandbox_keeps_its_reason_and_is_the_same_code() {
        let world = world();
        let mut driver = driver(&world, answer(receipt_line()));
        driver.sandbox.start_error = Some(SandboxError::Refused(Reason::Ceiling));
        let finished = run(&driver, &input_json(|_| {}), &signals()).await;
        assert_eq!(
            failed_with(&finished),
            DatabaseFailure::LakeSandboxUnavailable
        );
        assert_eq!(finished.sandbox_refusal, Some(Reason::Ceiling));
    }

    #[tokio::test]
    async fn a_container_the_sandbox_did_not_confirm_gone_leaves_its_directory_and_holds_its_slot_back()
     {
        let world = world();
        let mut driver = driver(&world, answer(receipt_line()));
        driver.sandbox.kill_unconfirmed = true;
        // a step of a second, so that the deadline the slot is held until is soon
        let input = input_json(|d| d["limits"]["timeoutMs"] = json!(1000));
        let finished = run(&driver, &input, &signals()).await;
        assert!(!finished.container_ended);
        let left = finished.left.expect("the directory is left where it is");
        assert!(
            left.join("relay.token").is_file(),
            "the container may still be using it"
        );
        assert_eq!(entries(world.work.path()).len(), 1);
        // the container may still be running as the slot's user: the slot is not free
        assert_eq!(driver.slots.in_use(), 1, "the slot is held back");
        // and it comes back once the deadline has passed
        tokio::time::sleep(Duration::from_millis(1300)).await;
        assert_eq!(
            driver.slots.in_use(),
            0,
            "the slot comes back after the deadline"
        );
    }

    #[tokio::test]
    async fn an_attempt_cancelled_before_it_starts_starts_nothing_and_leaves_nothing() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let signals = signals();
        signals.cancelled.cancel();
        let finished = run(&driver, &input_json(|_| {}), &signals).await;
        assert_eq!(failed_with(&finished), DatabaseFailure::Cancelled);
        assert!(driver.sandbox.asked().is_empty());
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(driver.slots.in_use(), 0);
    }

    #[tokio::test]
    async fn a_lost_lease_reports_nothing() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let signals = signals();
        signals.lease_lost.cancel();
        let finished = run(&driver, &input_json(|_| {}), &signals).await;
        assert_eq!(finished.report, Report::Abandon);
        assert!(entries(world.work.path()).is_empty());
    }

    #[tokio::test]
    async fn the_container_gets_the_strictest_timeout_of_the_step_and_the_connection() {
        let world = world();
        // the step says two seconds
        let driver_a = driver(&world, answer(receipt_line()));
        run(
            &driver_a,
            &input_json(|d| d["limits"]["timeoutMs"] = json!(2000)),
            &signals(),
        )
        .await;
        let asked = driver_a.sandbox.asked();
        assert!(
            asked[0].deadline <= Duration::from_secs(2),
            "{:?}",
            asked[0].deadline
        );
        assert!(
            asked[0].deadline > Duration::from_millis(1000),
            "{:?}",
            asked[0].deadline
        );

        // the connection says one and a half, and is stricter
        let driver_b = driver(&world, answer(receipt_line()));
        let mut message = message();
        message.limits = Some(sagat_api_grpc::pb::LakeLimits {
            max_record_bytes: 0,
            max_records: 0,
            timeout_ms: 1500,
        });
        let connection = super::super::connection::convert(&mut message).unwrap();
        let input = input_json(|d| d["limits"]["timeoutMs"] = json!(3_600_000));
        driver_b.run(&input, connection, &signals(), || {}).await;
        let asked = driver_b.sandbox.asked();
        assert!(
            asked[0].deadline <= Duration::from_millis(1500),
            "{:?}",
            asked[0].deadline
        );
        assert!(
            asked[0].deadline > Duration::from_millis(500),
            "{:?}",
            asked[0].deadline
        );
    }

    #[tokio::test]
    async fn an_attempt_above_the_threshold_asks_for_the_large_class_and_one_below_does_not() {
        let world = world();
        let records = json!({"inline": [{"a": "xxxxxxxxxxxxxxxxxxxx"}]});
        let mut with_large = driver(&world, answer(receipt_line()));
        with_large.settings.large_above_bytes = Some(10);
        run(
            &with_large,
            &input_json(|d| d["records"] = records.clone()),
            &signals(),
        )
        .await;
        assert_eq!(with_large.sandbox.asked()[0].class, Class::Large);

        let mut roomy = driver(&world, answer(receipt_line()));
        roomy.settings.large_above_bytes = Some(10_000);
        run(
            &roomy,
            &input_json(|d| d["records"] = records.clone()),
            &signals(),
        )
        .await;
        assert_eq!(roomy.sandbox.asked()[0].class, Class::Small);

        let none = driver(&world, answer(receipt_line()));
        run(
            &none,
            &input_json(|d| d["records"] = records.clone()),
            &signals(),
        )
        .await;
        assert_eq!(
            none.sandbox.asked()[0].class,
            Class::Small,
            "no threshold, no large class"
        );
    }

    #[tokio::test]
    async fn the_threshold_itself_is_still_the_small_class_and_one_byte_over_is_not() {
        let world = world();
        let record = json!({"a": "xxxxxxxxxxxxxxxxxxxx"});
        // a record is one line: its compact text and a newline
        let staged = serde_json::to_vec(&record).unwrap().len() as u64 + 1;
        let input = input_json(|d| d["records"] = json!({"inline": [record.clone()]}));
        for (threshold, class) in [(staged, Class::Small), (staged - 1, Class::Large)] {
            let mut driver = driver(&world, answer(receipt_line()));
            driver.settings.large_above_bytes = Some(threshold);
            run(&driver, &input, &signals()).await;
            assert_eq!(
                driver.sandbox.asked()[0].class,
                class,
                "threshold {threshold}"
            );
        }
    }

    #[tokio::test]
    async fn a_connection_that_does_not_allow_the_mode_fails_before_the_sandbox_is_asked() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let mut message = message();
        message.allowed_modes = vec!["overwrite".into()];
        let finished = driver
            .run(
                &input_json(|_| {}),
                super::super::connection::convert(&mut message).unwrap(),
                &signals(),
                || {},
            )
            .await;
        assert_eq!(failed_with(&finished), DatabaseFailure::Configuration);
        assert_eq!(finished.cause, Some(Cause::Prepare(PrepareError::Mode)));
        assert!(driver.sandbox.asked().is_empty());
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(driver.slots.in_use(), 0);
    }

    // -- what the reviews of the stack found ------------------------------------------------------------------------

    /// Runs `driver` over a stalled blob until `then` has happened, and says what it returned and how long it took.
    async fn over_a_stalled_blob(
        signals: &Signals,
        timeout_ms: u64,
        then: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> (
        Finished,
        Duration,
        Driver<Scripted, Stalled, NoDns, NoNetwork>,
        World,
    ) {
        let world = world();
        let driver = driver_with(&world, answer(receipt_line()), Stalled);
        let input = input_json(|d| {
            by_blob(d);
            d["limits"]["timeoutMs"] = json!(timeout_ms);
        });
        tokio::spawn(then);
        let began = std::time::Instant::now();
        let finished = tokio::time::timeout(
            Duration::from_secs(10),
            driver.run(&input, converted(), signals, || {}),
        )
        .await
        .expect("the driver gave up on a source that never answers");
        (finished, began.elapsed(), driver, world)
    }

    #[tokio::test]
    async fn a_cancellation_ends_the_staging_of_a_blob_that_never_answers() {
        let signals = signals();
        let cancel = signals.cancelled.clone();
        let (finished, took, driver, world) = over_a_stalled_blob(&signals, 600_000, async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            cancel.cancel();
        })
        .await;
        assert_eq!(failed_with(&finished), DatabaseFailure::Cancelled);
        assert_eq!(finished.cause, Some(Cause::Over));
        assert!(took < Duration::from_secs(5), "{took:?}");
        assert!(
            entries(world.work.path()).is_empty(),
            "nothing is left of the staging"
        );
        assert_eq!(driver.slots.in_use(), 0);
        assert!(driver.sandbox.asked().is_empty());
    }

    #[tokio::test]
    async fn a_lost_lease_ends_the_staging_and_reports_nothing() {
        let signals = signals();
        let lost = signals.lease_lost.clone();
        let (finished, _, driver, world) = over_a_stalled_blob(&signals, 600_000, async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            lost.cancel();
        })
        .await;
        assert_eq!(finished.report, Report::Abandon);
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(driver.slots.in_use(), 0);
    }

    #[tokio::test]
    async fn the_time_of_the_step_includes_the_staging() {
        let signals = signals();
        let (finished, took, driver, world) = over_a_stalled_blob(&signals, 1000, async {}).await;
        assert_eq!(failed_with(&finished), DatabaseFailure::Timeout);
        assert_eq!(finished.cause, Some(Cause::Over));
        assert!(
            took >= Duration::from_millis(900) && took < Duration::from_secs(5),
            "{took:?}"
        );
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(driver.slots.in_use(), 0);
        assert!(
            driver.sandbox.asked().is_empty(),
            "no container for a step that used its time staging"
        );
    }

    #[tokio::test]
    async fn a_refusal_that_no_retry_changes_is_a_configuration_and_a_full_node_is_still_retryable()
    {
        for (reason, code) in [
            (Reason::Deadline, DatabaseFailure::Configuration),
            (Reason::Class, DatabaseFailure::Configuration),
            (Reason::Directory, DatabaseFailure::Configuration),
            (Reason::Ceiling, DatabaseFailure::LakeSandboxUnavailable),
            (Reason::Slot, DatabaseFailure::LakeSandboxUnavailable),
        ] {
            let world = world();
            let mut driver = driver(&world, answer(receipt_line()));
            driver.sandbox.start_error = Some(SandboxError::Refused(reason));
            let finished = run(&driver, &input_json(|_| {}), &signals()).await;
            assert_eq!(failed_with(&finished), code, "{reason:?}");
            assert_eq!(finished.sandbox_refusal, Some(reason), "the reason is kept");
            assert_eq!(
                code.retryable(),
                code == DatabaseFailure::LakeSandboxUnavailable,
                "{reason:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_dropped_run_keeps_the_directory_and_holds_the_slot_until_the_deadline() {
        let world = world();
        let driver = driver(&world, Script::NeverConnects);
        // a step of a second: the host never connects, and the caller gives up on the future after a third of it
        let input = input_json(|d| d["limits"]["timeoutMs"] = json!(1000));
        let outcome = tokio::time::timeout(
            Duration::from_millis(300),
            driver.run(&input, converted(), &signals(), || {}),
        )
        .await;
        assert!(outcome.is_err(), "the future was dropped, not finished");

        // the container was asked for and is not killed by a drop: it ends by its deadline
        assert_eq!(driver.sandbox.asked().len(), 1);
        assert_eq!(driver.sandbox.kills.load(Ordering::SeqCst), 0);
        // so its directory stays, and its slot is not free
        assert_eq!(entries(world.work.path()).len(), 1, "the directory is kept");
        assert_eq!(driver.slots.in_use(), 1, "the slot is held back");
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert_eq!(
            driver.slots.in_use(),
            0,
            "and comes back after the deadline"
        );
        assert_eq!(
            entries(world.work.path()).len(),
            1,
            "the directory is the sweep's"
        );
    }

    #[tokio::test]
    async fn a_receipt_that_is_not_for_this_request_is_not_a_commit_of_this_step() {
        let object_storage: Value = serde_json::from_str(
            &std::fs::read_to_string(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
                    "../../tests/contract/iceberg-writes/fixtures/receipts/object-storage-already-present.json",
                ),
            )
            .unwrap(),
        )
        .unwrap();
        let cases: Vec<(&str, Vec<u8>)> = vec![
            (
                "another effect key",
                receipt_line_with(|r| r["effectKey"] = json!("run-other/load/1")),
            ),
            (
                "another ref",
                receipt_line_with(|r| r["ref"] = json!("staging")),
            ),
            (
                "another mode",
                receipt_line_with(|r| r["mode"] = json!("overwrite")),
            ),
            (
                "another kind of receipt",
                receipt_line_with(|r| *r = object_storage.clone()),
            ),
        ];
        for (name, line) in cases {
            let world = world();
            let driver = driver(&world, answer(line));
            let finished = run(&driver, &input_json(|_| {}), &signals()).await;
            assert_eq!(
                failed_with(&finished),
                DatabaseFailure::LakeHostProtocol,
                "{name}"
            );
            assert!(finished.request_sent, "{name}: the host had the request");
            assert!(entries(world.work.path()).is_empty(), "{name}");
        }

        // and the receipt that is for it is taken, so that the cases above are the check and not the setup
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let finished = run(&driver, &input_json(|_| {}), &signals()).await;
        assert!(matches!(finished.report, Report::Complete(_)));
    }

    #[tokio::test]
    async fn a_host_that_is_not_the_user_of_the_slot_is_not_served_and_never_gets_the_request() {
        let world = world();
        let mut driver = driver(&world, answer(receipt_line()));
        // the relay is built for the user the slot has; the host here is another
        driver.peer_for_slot = |_| Some(my_uid().wrapping_add(1));
        driver.settings.host_start = Duration::from_millis(600);
        let finished = run(&driver, &input_json(|_| {}), &signals()).await;
        assert!(
            !finished.request_sent,
            "the credentials never left the runner"
        );
        assert!(driver.sandbox.request.lock().unwrap().is_empty());
        assert_eq!(failed_with(&finished), DatabaseFailure::LakeHostProtocol);
        assert!(entries(world.work.path()).is_empty());
    }

    #[tokio::test]
    async fn the_host_is_told_when_the_step_ends_and_the_attempt_is_named_at_random() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));
        let before = SystemTime::now();
        let input = input_json(|d| d["limits"]["timeoutMs"] = json!(2000));
        run(&driver, &input, &signals()).await;
        let request: Value =
            serde_json::from_slice(&driver.sandbox.request.lock().unwrap()).unwrap();
        let told = time::OffsetDateTime::parse(
            request["limits"]["deadlineAt"].as_str().unwrap(),
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
        let told = SystemTime::from(told);
        let early = before + Duration::from_millis(1500);
        let late = before + Duration::from_millis(2500);
        assert!(
            told >= early && told <= late,
            "deadlineAt is two seconds from the start of the attempt"
        );

        let id = &driver.sandbox.asked()[0].attempt_id;
        let id = uuid::Uuid::parse_str(id).unwrap();
        assert_eq!(id.get_version_num(), 4, "random, not time-ordered");
    }

    // -- the probe (D035-150, slice S6b) ---------------------------------------------------------------------------------

    use super::super::fixtures::probe_input_json;

    fn probe_lease() -> Vec<u8> {
        probe_input_json(|_| {})
    }

    /// Admits a probe and runs it, as the shell does for a lease.
    async fn probe(
        driver: &Driver<Scripted, NoBlobs, NoDns, NoNetwork>,
        input: &[u8],
        signals: &Signals,
    ) -> Finished {
        match driver.admit_probe(input) {
            Ok(admitted) => {
                driver
                    .run_admitted(admitted, converted(), signals, || {})
                    .await
            }
            Err(finished) => finished,
        }
    }

    #[tokio::test]
    async fn a_probe_the_host_answers_reports_its_result_and_leaves_nothing_behind() {
        let world = world();
        let driver = driver(&world, answer(probe_line()));

        let finished = probe(&driver, &probe_lease(), &signals()).await;

        let Report::Probe(report) = &finished.report else {
            panic!("not a probe report: {:?}", finished.report);
        };
        assert!(
            report.answers(&sagat_infra_lake::protocol::ProbeInput::read(&probe_lease()).unwrap())
        );
        assert!(
            finished.probe,
            "it says it was a probe, so that it is not counted as a write"
        );
        assert!(finished.request_sent);
        assert!(finished.container_ended);
        assert_eq!(finished.left, None);
        assert!(
            entries(world.work.path()).is_empty(),
            "the directory is gone"
        );
        assert_eq!(driver.slots.in_use(), 0, "the slot is free again");
        assert_eq!(driver.sandbox.kills.load(Ordering::SeqCst), 1);

        // what the host was sent is the request of a probe: no effect key, the connection and what to run
        let request: Value =
            serde_json::from_slice(&driver.sandbox.request.lock().unwrap()).unwrap();
        assert_eq!(request["operation"], "probe");
        assert!(request.get("effectKey").is_none());
        assert_eq!(
            request["probe"]["connection"],
            json!({"name": "lake-main", "revision": 3})
        );
        // the host is told the time that fits the attempt (the lease names 7 s a check, and three of them do not fit)
        let check = request["probe"]["checkTimeoutMs"].as_u64().unwrap();
        assert!((6_300..=6_333).contains(&check), "{check}");
        // a probe stages nothing and needs no budget of a write: the small class, and the slot the sandbox's user is
        let asked = driver.sandbox.asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].slot, 0);
        assert_eq!(asked[0].class, Class::Small);
    }

    #[tokio::test]
    async fn a_result_for_another_connection_or_another_revision_is_not_the_result_of_this_probe() {
        for edit in [
            (|receipt: &mut Value| receipt["connection"]["name"] = json!("other-lake"))
                as fn(&mut Value),
            |receipt: &mut Value| receipt["connection"]["revision"] = json!(4),
        ] {
            let world = world();
            let driver = driver(&world, answer(probe_line_with(edit)));

            let finished = probe(&driver, &probe_lease(), &signals()).await;

            assert_eq!(failed_with(&finished), DatabaseFailure::LakeHostProtocol);
            assert!(finished.probe, "a probe that failed is still not a write");
            assert!(entries(world.work.path()).is_empty());
            assert_eq!(driver.slots.in_use(), 0);
        }
    }

    #[tokio::test]
    async fn a_probe_the_host_cannot_run_is_reported_with_the_code_it_gave() {
        let world = world();
        let driver = driver(
            &world,
            answer(plain_failure_line("DATABASE_CREDENTIAL_UNAVAILABLE")),
        );

        let finished = probe(&driver, &probe_lease(), &signals()).await;

        assert_eq!(failed_with(&finished), DatabaseFailure::Credential);
        assert!(finished.probe, "a probe that failed is still not a write");
    }

    #[tokio::test]
    async fn a_code_of_a_write_from_the_host_of_a_probe_is_a_protocol_failure() {
        // the host holds the secrets and reads what a hostile catalog says: it cannot make a probe fail as a commit
        // conflict or as an unknown outcome, which the control plane would retry or take as a write that may have landed
        for code in [
            "ICEBERG_COMMIT_CONFLICT",
            "OUTCOME_UNKNOWN",
            "ICEBERG_SCHEMA_MISMATCH",
        ] {
            let world = world();
            let driver = driver(&world, answer(plain_failure_line(code)));

            let finished = probe(&driver, &probe_lease(), &signals()).await;

            assert_eq!(
                failed_with(&finished),
                DatabaseFailure::LakeHostProtocol,
                "{code}"
            );
        }
    }

    #[tokio::test]
    async fn the_receipt_of_a_write_is_not_an_answer_to_a_probe() {
        let world = world();
        let driver = driver(&world, answer(receipt_line()));

        let finished = probe(&driver, &probe_lease(), &signals()).await;

        assert_eq!(failed_with(&finished), DatabaseFailure::LakeHostProtocol);
    }

    #[tokio::test]
    async fn a_host_killed_by_the_memory_limit_is_a_worker_error_and_not_a_write_that_does_not_fit()
    {
        let world = world();
        // the host reads the request and closes without a result, and the sandbox says the memory limit killed it
        let mut driver = driver(&world, answer(Vec::new()));
        driver.sandbox.memory_killed = true;

        let finished = probe(&driver, &probe_lease(), &signals()).await;

        assert_eq!(failed_with(&finished), DatabaseFailure::WorkerError);
        assert_eq!(finished.exit, Some(LakeSandboxExit::MemoryKilled));
        assert!(
            finished.probe,
            "a probe that was killed is still not a write"
        );
    }

    #[tokio::test]
    async fn a_probe_cancelled_after_the_host_has_the_request_is_cancelled_and_not_an_unknown_outcome()
     {
        let world = world();
        // the host reads the request and never answers
        let driver = driver(&world, Script::Silent);
        let signals = signals();
        let requested = std::sync::Arc::clone(&driver.sandbox.requested);
        let lease = probe_lease();

        let (finished, ()) = tokio::join!(probe(&driver, &lease, &signals), async {
            requested.notified().await;
            signals.cancelled.cancel();
        });

        // a write that was sent may have committed and is unknown; a probe changes nothing, so nothing of it is unknown
        assert!(finished.request_sent);
        assert_eq!(failed_with(&finished), DatabaseFailure::Cancelled);
        assert!(finished.probe);
        assert_eq!(driver.slots.in_use(), 0);
    }

    #[tokio::test]
    async fn a_lease_that_is_not_a_probe_lease_is_refused_before_anything_is_taken_or_made() {
        let world = world();
        let driver = driver(&world, answer(probe_line()));

        // the lease of a write, and one of the lease of a probe with a member it does not name
        for input in [
            input_json(|_| {}),
            probe_input_json(|d| d["table"] = json!("orders")),
            b"not json".to_vec(),
        ] {
            let finished = probe(&driver, &input, &signals()).await;
            assert_eq!(failed_with(&finished), DatabaseFailure::Configuration);
            assert_eq!(finished.cause, Some(Cause::Input));
        }
        assert_eq!(driver.slots.in_use(), 0);
        assert!(
            driver.sandbox.asked().is_empty(),
            "the sandbox was not asked"
        );
        assert!(entries(world.work.path()).is_empty());
    }

    #[tokio::test]
    async fn a_probe_lease_is_refused_by_the_reader_of_a_write() {
        let world = world();
        let driver = driver(&world, answer(probe_line()));

        let finished = run(&driver, &probe_lease(), &signals()).await;

        assert_eq!(failed_with(&finished), DatabaseFailure::Configuration);
        assert_eq!(finished.cause, Some(Cause::Input));
    }

    #[tokio::test]
    async fn with_no_free_slot_a_probe_fails_retryably_and_the_sandbox_is_not_asked() {
        let world = world();
        let driver = driver(&world, answer(probe_line()));
        let held: Vec<_> = (0..4).map(|_| driver.slots.take().unwrap()).collect();

        let finished = probe(&driver, &probe_lease(), &signals()).await;

        assert_eq!(
            failed_with(&finished),
            DatabaseFailure::LakeSandboxUnavailable
        );
        assert_eq!(finished.cause, Some(Cause::NoSlot));
        assert!(driver.sandbox.asked().is_empty());
        drop(held);
    }

    #[test]
    fn a_probe_holds_a_slot_from_its_admission_until_it_is_dropped() {
        let world = world();
        let driver = driver(&world, answer(probe_line()));

        let admitted = driver.admit_probe(&probe_lease()).unwrap();
        assert_eq!(driver.slots.in_use(), 1);
        assert_eq!(admitted.slot(), 0);
        drop(admitted);

        assert_eq!(driver.slots.in_use(), 0);
    }

    #[tokio::test]
    async fn a_probe_cancelled_before_it_starts_is_cancelled_and_one_whose_lease_is_lost_reports_nothing()
     {
        let world = world();
        let driver = driver(&world, answer(probe_line()));
        let cancelled = signals();
        cancelled.cancelled.cancel();
        let lost = signals();
        lost.lease_lost.cancel();

        let finished = probe(&driver, &probe_lease(), &cancelled).await;
        assert_eq!(failed_with(&finished), DatabaseFailure::Cancelled);
        let finished = probe(&driver, &probe_lease(), &lost).await;
        assert_eq!(finished.report, Report::Abandon);

        assert!(
            driver.sandbox.asked().is_empty(),
            "no container was started"
        );
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(driver.slots.in_use(), 0);
    }

    /// The time the container was given, and the time one check was given in the request the host read.
    fn times_of(driver: &Driver<Scripted, NoBlobs, NoDns, NoNetwork>) -> (Duration, u64) {
        let request: Value =
            serde_json::from_slice(&driver.sandbox.request.lock().unwrap()).unwrap();
        (
            driver.sandbox.asked()[0].deadline,
            request["probe"]["checkTimeoutMs"].as_u64().unwrap(),
        )
    }

    #[tokio::test]
    async fn the_container_of_a_probe_gets_the_time_of_its_checks_and_the_host_the_time_that_fits()
    {
        // the lease names 7 s for a check: three of them and the margin are more than the ceiling of 22 s, so the container
        // has 22 s and each check the third of what is left after the margin, which is what fits (and not 7 s)
        let world = world();
        let driver_a = driver(&world, answer(probe_line()));
        probe(&driver_a, &probe_lease(), &signals()).await;
        let (deadline, check) = times_of(&driver_a);
        assert!(
            deadline <= Duration::from_secs(22) && deadline > Duration::from_secs(21),
            "{deadline:?}"
        );
        assert!((6_300..=6_333).contains(&check), "{check}");

        // a lease that names less (2 s a check) is given what it names: three of them and the margin are 9 s
        let driver_b = driver(&world, answer(probe_line()));
        let lease = probe_input_json(|d| d["checkTimeoutMs"] = json!(2000));
        probe(&driver_b, &lease, &signals()).await;
        let (deadline, check) = times_of(&driver_b);
        assert!(
            deadline <= Duration::from_secs(9) && deadline > Duration::from_secs(8),
            "{deadline:?}"
        );
        assert!((1_990..=2_000).contains(&check), "{check}");
    }

    #[tokio::test]
    async fn a_connection_that_allows_less_lowers_the_container_and_the_checks_with_it() {
        // a connection that allows a second and a half is stricter than anything the lease says; the host is still told a
        // time it reads, so that its answer is the typed timeout of a check and not the end of the attempt
        let world = world();
        let driver = driver(&world, answer(probe_line()));
        let mut message = message();
        message.limits = Some(sagat_api_grpc::pb::LakeLimits {
            max_record_bytes: 0,
            max_records: 0,
            timeout_ms: 1500,
        });
        let connection = super::super::connection::convert(&mut message).unwrap();
        let admitted = driver.admit_probe(&probe_lease()).unwrap();

        driver
            .run_admitted(admitted, connection, &signals(), || {})
            .await;

        let (deadline, check) = times_of(&driver);
        assert!(
            deadline <= Duration::from_millis(1500) && deadline > Duration::from_millis(500),
            "{deadline:?}"
        );
        assert_eq!(check, 100);
    }

    #[tokio::test]
    async fn the_time_of_a_probe_is_counted_from_its_admission_and_not_from_when_its_connection_arrived()
     {
        let world = world();
        let driver = driver(&world, answer(probe_line()));
        // a connection that allows 50 ms, and one that took longer than that to resolve: the step has its time from the moment
        // it was leased, and a connection that arrives late has used some of it
        let mut message = message();
        message.limits = Some(sagat_api_grpc::pb::LakeLimits {
            max_record_bytes: 0,
            max_records: 0,
            timeout_ms: 50,
        });
        let connection = super::super::connection::convert(&mut message).unwrap();
        let admitted = driver.admit_probe(&probe_lease()).unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;

        let finished = driver
            .run_admitted(admitted, connection, &signals(), || {})
            .await;

        assert_eq!(failed_with(&finished), DatabaseFailure::Timeout);
        assert_eq!(finished.cause, Some(Cause::Over));
        assert!(
            driver.sandbox.asked().is_empty(),
            "no container was started"
        );
        assert!(entries(world.work.path()).is_empty());
        assert_eq!(driver.slots.in_use(), 0);
    }

    #[test]
    fn probes_may_hold_half_of_the_slots_and_a_write_always_finds_one() {
        // a tenant can keep probes going against a catalog that answers slowly: the server limits how many are in flight for
        // a tenant, not how many a node holds, so the node keeps slots for the writes
        let world = world();
        let driver = driver(&world, answer(probe_line()));
        let first = driver.admit_probe(&probe_lease()).unwrap();
        let second = driver.admit_probe(&probe_lease()).unwrap();

        let refused = driver.admit_probe(&probe_lease()).err().unwrap();

        assert_eq!(
            failed_with(&refused),
            DatabaseFailure::LakeSandboxUnavailable
        );
        assert_eq!(refused.cause, Some(Cause::NoSlot));
        assert_eq!(
            driver.slots.in_use(),
            2,
            "two slots were free and were not taken"
        );
        // a write is not refused for it
        let write = driver.admit(&input_json(|_| {})).unwrap();
        let another_write = driver.admit(&input_json(|_| {})).unwrap();
        assert_eq!(driver.slots.in_use(), 4);
        // and a probe is admitted again once one of the two is over
        drop((write, another_write));
        drop(first);
        let third = driver.admit_probe(&probe_lease()).unwrap();
        // the two that hold probes now are the second and the third, and a fourth is refused again
        assert!(driver.admit_probe(&probe_lease()).is_err());
        drop(second);
        assert_eq!(
            driver.slots.in_use(),
            1,
            "only the probe admitted last is held"
        );
        drop(third);
        assert_eq!(driver.slots.in_use(), 0);
    }

    #[test]
    fn a_node_of_one_slot_still_runs_a_probe() {
        let world = world();
        let mut driver = driver(&world, answer(probe_line()));
        driver.slots = SlotPool::new(1);

        let admitted = driver.admit_probe(&probe_lease());

        assert!(
            admitted.is_ok(),
            "half of one slot is not none: a probe may use it"
        );
        assert_eq!(driver.slots.in_use(), 1);
    }

    #[test]
    fn the_slot_of_a_probe_is_counted_as_a_probes_until_it_is_given_back() {
        let pool = SlotPool::new(4);
        let a = pool.take_probe(2).unwrap();
        let b = pool.take_probe(2).unwrap();
        assert!(pool.take_probe(2).is_none(), "two are held by probes");
        let write = pool.take().unwrap();
        drop(a);
        let c = pool.take_probe(2).unwrap();
        assert!(pool.take_probe(2).is_none(), "b and c hold the two");
        drop((b, c, write));
        assert_eq!(pool.in_use(), 0);
        // a pool of three slots lets probes have one, and of one slot has them use it
        assert_eq!(SlotPool::new(3).probe_cap(), 1);
        assert_eq!(SlotPool::new(4).probe_cap(), 2);
        assert_eq!(SlotPool::new(1).probe_cap(), 1);
        assert_eq!(SlotPool::new(8).probe_cap(), 4);
    }
}
