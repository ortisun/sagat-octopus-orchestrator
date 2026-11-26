# Research register: design 054, buffered signals

The registers of [design 054](../../docs/designs/054-buffered-signals.md): decisions (Decision 054-NNN), gaps (Gap 054-NNN) and
owner questions (Owner question 054-NNN). Append-only: an entry is never renumbered or deleted; a revised entry says so in place.
Every owner question below was decided under the owner's delegation (Owner decision 2026-11-20-6, widened by Owner decision
2026-11-24-4), each weighed over its options for the most architectural and business advantage, and is reported to the owner. The
coordinating session decided that buffered signals are design 054 and that superseding Architecture decision 0050 decision 4 (and
Architecture decision 0030 decision 17) in part is within that delegation, Architecture decision 0111 to be accepted if design 054 is
approved.

## What the code does today (2026-11-26, branch `docs/temporal-parity-readiness` at `5d2d7833`)

- A signal is delivered by `deliver_signal` (`crates/application/src/runs.rs`) to every root `waitEvent` step that waits for its
  name (`state.steps()` yields root steps only), a copy each, the payload becoming each step's output; with none, the answer is
  `409` "the run is not waiting for that signal". The chain path (`signal` in `crates/application/src/chain_progress.rs`) repeats
  the same selection on the chain's head. A body wait is never a signal's target.
- The payload is JSON of at most 256 KiB (`MAX_PAYLOAD_BYTES`), and a delivery that would take the run's retained step outputs over
  4 MiB (`MAX_RUN_OUTPUT_BYTES`, `crates/application/src/execution.rs`) is refused `429 QUOTA_EXCEEDED`; 15 source files name
  the constant.
- Signal-with-Start delivers after the initial reduction in the creation transaction and rolls back with
  `SIGNAL_NOT_ACCEPTED_AT_START` when nothing waits (`runs.rs`); reset reapply ends the reapply of signals at the first one nobody
  waits for (`runs/reset.rs`), its candidates being `wait.satisfied` events of `via: signal` (`runs/reapply.rs`).
- A root `waitEvent` enters `WAITING` through the planner's `Evaluation::Correlation` (`crates/domain/src/execution/plan.rs`), which
  `Engine::run` resolves in its loop (`crates/application/src/execution.rs`).
- Feature 004's look-back: an event is admitted to `inbox_events` (payload inline, purged at the end of its deduplication window,
  60 s to 30 days); a wait records `steps.event_since` (its entry less `lookbackMs`, at most 7 days); the `late_waits` scan of
  `crates/scheduler/src/events.rs` delivers an undelivered matching event in a later transaction, recording `event_deliveries` once
  per consumer (`wait:{run}/{node}`), after re-checking under the run lock that the step still waits.
- Every run transaction, a path transaction included, locks the run's row `FOR UPDATE` after its chain (`lock_path_revision`,
  `crates/infra-postgres/src/execution.rs`).
- The continuation's handoff (`continue_as_new`, `crates/infra-postgres/src/chain_progress.rs`) locks the chain, then the run,
  requires the run's version to be the chain's, and inserts the successor.
- The IR's root object is closed (`additionalProperties: false` in `specs/001-platform-foundation/contracts/workflow.schema.json`).
- Architecture decision 0030 decision 17, Architecture decision 0050 decision 4 and feature 004's spec say no signal is buffered;
  feature 018 lists buffered signals as out of scope.

## Decisions

- **Decision 054-001 — An inbox of the run, read before the look-back** (Owner question 054-001). Options: (a) admit a signal as an
  `inbox_events` row correlated to the run and read through feature 004's look-back; (b) a table of the run's pending signals,
  consumed at a wait's entry; (c) (b), and route events through it too. Chosen (b), delegate (Owner decision 2026-11-24-4): (a)
  differs from a signal on lifetime (a window, then gone without a record), consumers (a copy to every wait that looks back), timing
  (a later scan), scope (namespace rows) and accounting (outside the run's bytes and `payloads`), each of which a buffered signal
  needs the other way (design 054, Technical Plan 5); (c) would change events, which are published to everyone who listens. When a
  wait declares both, the inbox is consumed at its entry, before any event; the look-back acts only on a wait still waiting, and
  then the first commit wins. A signal is addressed to this run and was accepted for it; an event's look-back is a best effort.
- **Decision 054-002 — Opt in per version and per name** (Owner question 054-002; carries Decision 045-008). Options: (a) buffer
  every signal of every run; (b) opt in. Chosen (b), delegate (Owner decision 2026-11-24-4): (a) changes the answer of `signalRun`
  for runs in flight and for callers who rely on the `409` (principle IV); (b) gives the capability to every version that asks.
- **Decision 054-003 — One consumer per signal of a declared name** (Owner question 054-003). Options: (a) today's copies when steps
  wait, one consumer when buffered; (b) one consumer always, the first waiter; (c) copies always, each pending signal consumed by
  every later wait. Chosen (b), delegate (Owner decision 2026-11-24-4): under (a) the number of waits a signal satisfies depends on
  whether it arrived before or after them, which no author can reason about; (c) makes a queue impossible and a pending signal
  unbounded in effect. (b) is a queue, as a Temporal channel is; an author who wants a copy each leaves the name undeclared. The
  first waiter is the one with the earliest entry time, a tie going to the lower node id. *Revised again 2026-11-26 after the stage 8
  zoom-out (its Nit 4):* the waiting step with the lowest node id, the order in which the planner enters waits, so one rule decides
  live delivery and consumption and no entry time is derived. *Revised 2026-11-26 after the stage 5
  zoom-out (its Nit 6):* the entry time is derived by the domain from the wait's deadline less its node's `timeoutMs` (both already
  in its state and definition), not from feature 004's `event_since`. A root graph is acyclic, so two waits of one name wait at
  once only in parallel branches.
- **Decision 054-004 — The table holds pending rows only** (Owner question 054-004). Options: (a) the first draft's rows with a state
  (`PENDING`, `CONSUMED`, `MOVED`, `DISCARDED`) and a partial index of pending rows; (b) a row exists while its signal is pending,
  each settlement deletes it and is journaled. Chosen (b), delegate (Owner decision 2026-11-24-4): under (a) the table grows with a
  long run's whole history and duplicates what the journal records; under (b) the table is the live backlog (at most 1,000 rows per
  run), its index needs no predicate, and the oldest pending row of a name is the first entry of its range. The read of "only the
  oldest pending row of its name through an index" that the earlier reviews asked for holds.
- **Decision 054-005 — Declared bounds checked at publication** (Owner question 054-005). Options: (a) three runtime bounds (per
  name, 1,000 per run, the run's bytes); (b) `maxPending` and `maxBytes` per name, publication refusing a sum of `maxPending` above
  1,000 or of `maxPending × maxBytes` above 4 MiB, so that at run time only the name's count and the run's shared bytes refuse.
  Chosen (b), delegate (Owner decision 2026-11-24-4): an author can state the bound (with `maxBytes` of 256 KiB, at most 16 pending
  signals in all, 4,096 KiB divided by 256 KiB), the declarations bound what a sender can pin in a run, and only step outputs that
  already used the room make a signal within the bounds refused. `maxBytes` defaults to 64 KiB, as design 044's signals do.
  *Revised 2026-11-26 after the stage 8 zoom-out (its Majors 1 and 2):* the byte sum is capped at 2 MiB, half of a run's retained
  bytes, not 4 MiB: with 4 MiB, senders acting within the declared bounds could take the room of the run's step outputs and make a
  later step fail `RUN_DATA_LIMIT`; with 2 MiB the outputs always keep the other half, and the declarations also fit under design
  044's headroom of 3 MiB. With `maxBytes` of 256 KiB a version declares at most 8 pending signals in all.
- **Decision 054-006 — Continue-as-new moves what the successor declares** (Owner question 054-006; carries Decision 045-007).
  Options: (a) discard; (b) refuse the continuation while any is pending; (c) move the names the successor's version declares,
  discard the rest, journaled. Chosen (c), delegate (Owner decision 2026-11-24-4): (a) loses business messages, (b) lets a sender
  block a run; (c) keeps every message a successor can consume without user code draining a channel.
- **Decision 054-007 — The end of a run discards and counts** (Owner question 054-007). Options: (a) refuse to end while signals
  are pending; (b) discard them with a journaled count; (c) keep them readable after the end. Chosen (b), delegate (Owner decision
  2026-11-24-4): (a) lets a sender keep a run alive; (c) promises a consumer that cannot exist (a terminal run never returns to
  running, principle I). A signal for a run that ended or is stopping is refused, so nothing is buffered after the discard.
- **Decision 054-008 — Pending payloads count in the run's retained bytes** (Owner question 054-008). Options: (a) a separate
  budget for pending payloads; (b) one total of outputs and pending payloads. Chosen (b), delegate (Owner decision 2026-11-24-4):
  under (a) a consumption could take a run over its 4 MiB and would need a failure mode at a wait's entry; under (b) a consumption
  moves bytes within the total and never fails. *Revised 2026-11-26 after the stage 5 zoom-out (its Major 2):* the first text added
  a counter on `runs`, a new method and a fitness rule; the run already has a byte ledger (`run_instance_usage.logical_output_bytes`,
  migration `20261204000000_logical_output_byte_ledger`, read by `RunTx::logical_output_bytes()`), so pending bytes are a sibling
  column of that row, under one database check of the sum, and only the three sites that sum `RunData.outputs` themselves change.
- **Decision 054-009 — `buffered` is required and `true` (revised 2026-11-26: optional, and `true` when present).** The first draft's `buffered` meant nothing when false (zoom-out Nit 9
  of design 045's review of 2026-11-24). It is kept, as a constant, because design 044 declares signals in this shape and a later
  design may declare unbuffered signals (with a schema, say) beside them; a document never carries a field that says nothing
  without failing publication. Delegate (Owner decision 2026-11-24-4). *Revised 2026-11-26 after the stage 5 zoom-out (its Nit 5):*
  `buffered` is optional; a listed name is buffered; the field, when present, must be `true`, so that design 044's documents, which
  write it, stay valid, and no author has to write a field that says nothing.
- **Decision 054-010 — Code versions are consumers** (Gap 044-017). Architecture decision 0111 decision 1 is refined: the consumer
  of a code version's signal is the workflow function, consuming by sequence through design 044's command; `SIGNAL_WITHOUT_CONSUMER`
  applies to graph documents. Delegate (Owner decision 2026-11-24-4): one inbox and one lifecycle for both models (principle IV),
  as design 044's Decision 044-073 asked.
- **Decision 054-011 — The consumption is an evaluation of the planner.** Options: (a) load the pending rows into the run's state;
  (b) the store reads and deletes the oldest row of the name when the planner asks, once per entry
  (`Evaluation::BufferedSignal`); (c) the store satisfies waits after the domain's reduction. Chosen (b), delegate (Owner decision
  2026-11-24-4): (a) reads up to 1,000 rows on every transition; (c) puts a transition outside the domain; (b) keeps the domain pure,
  as the correlation of a wait is resolved today, and costs one statement per entry only when the run has pending signals.
  *Revised 2026-11-26 after the stage 5 zoom-out (its Nit 8):* no new evaluation; the existing `Evaluation::Correlation` of the
  wait carries `take_buffered`, and `Resolution::Correlation` the taken signal, since the correlation already is the one store
  round trip of a wait's entry.
- **Decision 054-012 — No refusal by the look-back or by a timeout.** A signal of a declared name that arrives after its wait timed
  out is buffered for a later wait of that name, never refused for that reason; this is what principle IV states for code runs
  ("a signal that arrives after the wait timed out is not dropped but kept"), applied to graph runs that opt in. Delegate (Owner
  decision 2026-11-24-4). *Revised 2026-11-26 after the stage 7 design critic (its Major 6):* a signal is buffered only when a
  consumer can still come; for a graph run whose root waits of that name have all been entered and whose version has no
  `continueAsNew` node, it is refused `409 NO_CONSUMER_AHEAD`, so that "too late" is not answered as a success. A step not yet
  entered counts as a consumer, so the refusal is never a guess.

- **Decision 054-013 — The continuation's handoffs tolerate a buffering and every terminal path discards** (stage 7 design critic,
  its Blockers 1 and 2 and Major 3). The scheduler's handoff (`continue_evaluated`) opens a successor's entry `waitEvent` store-side,
  so publication refuses, for a graph version that can continue as new, an entry wait of a declared name
  (`SIGNAL_ENTRY_WAIT_CONTINUES`); options were consuming in the handoff (a satisfied step the handoff cannot plan on) and the
  refusal, chosen because an author loses only the habit of waiting first. Store-side terminal paths (`finalize_chain`, the
  collection's finish) discard through one helper, and a check on `runs` refuses a terminal status with pending signals. The
  handoffs accept a run version advanced only by `signal.buffered` events, so a steady flow of signals cannot starve a
  continuation. Delegate (Owner decision 2026-11-24-4).

- **Decision 054-014 — One refusal reason for a fleet requirement not recorded** (stage 8 zoom-out, its Nit 5). Designs 044, 045
  and 054 named three reasons for one refusal (`CAPABILITY_NOT_ACTIVE`, `ACTIVITY_TIMEOUTS_NOT_ACTIVE`, `SIGNAL_INBOX_NOT_ACTIVE`).
  This design uses design 044's `CAPABILITY_NOT_ACTIVE` with the capability in the problem's detail, given by the shared hold of
  Task 054-18, so six SDKs map one code. Delegate (Owner decision 2026-11-24-4).

- **Decision 054-015 — What the stage 9 codebase review settled** (its Majors 1 to 9). The consumption moves its bytes by a
  statement of its own and is left out of the output charge, so they are never counted twice; the handoff's sequences and its
  replay lookup are stated; each new refusal of a reapplied signal is a `REJECTED` candidate with its own `ReapplyReason`, never an
  aborted reset; the chain path's receipt reader reads a buffered answer; a name is an `EventType`; the payload's digest is the
  journal's existing `payload_digest` column and `sagat_domain::workflow::Digest`; the `Capability` enumeration and its equality
  test are in scope; the checks are added `NOT VALID` and validated by a companion migration; a Signal-with-Start signal keeps
  its origin (`via`) when buffered and is not reapplied. Delegate (Owner decision 2026-11-24-4).

## Gaps

- **Gap 054-001** Payloads are never deleted today (`payloads` has no collector); a discarded or consumed signal's payload stays as
  run content until feature 020's purge decides the fate of payloads no row references. Not specific to this design.
- **Gap 054-002** A `waitEvent` inside a collection or a loop body is not a signal's consumer, today or here. A later design may
  admit body waits (the path transaction holds the run lock, so the inbox's rules would hold).
- **Gap 054-003** A graph version's signal has no schema of its own; a payload is checked as JSON of at most `maxBytes`, and is
  classified once consumed by the wait's `outputSchema`. Design 044 gives code versions a schema.
- **Gap 054-004** A run inside its start delay refuses signals, buffered names included, as feature 018 decided; a later design may
  buffer them (the run exists and has a lock).
- **Gap 054-005** No operator action deletes one pending signal; cancelling or resetting the run is the way. A later design may add
  an audited removal.

- **Gap 054-006** Design 044 writes "the inbox rows marked consumed" (its table of command records) and refuses an undeclared name
  on a code run (`SIGNAL_NOT_DECLARED`). Design 054 states the inbox's contract for it (Technical Plan 10: rows are deleted on
  consumption, `consume_pending_signals` all or nothing, each row's `version_id` for its classification) and defers code runs' undeclared names to it (Requirement 054-003); design
  044's owner is to cite that contract in its next revision.
- **Gap 054-007** Designs 044, 045 and 054 share `platform_requirement_hold` and the floor's trigger. Design 054's Task 054-18 makes
  them one prerequisite change, landed once by whichever design builds first; designs 044 and 045 are to name it as their
  dependency in their next revisions (stage 5 zoom-out, its Major 4).

- **Gap 054-008** Design 044 refuses a code run's signals from 3 MiB of retained bytes, at a visible count of 50,176 and while a
  closing window is open (`HISTORY_HEADROOM`, `RUN_CLOSING`); Requirement 054-007's bound is stated for graph runs, and the 2 MiB
  cap fits under 044's headroom. Design 044 is to apply the 2 MiB publication cap to its signals' declarations and state its
  headroom in its guide.
- **Gap 054-009** `SIGNAL_ENTRY_WAIT_CONTINUES` exists because the scheduler's handoff (`continue_evaluated`) opens a successor's
  entry `waitEvent` without the planner. The fix belongs in the handoff: leave a declared name's entry wait to the planner. Until a
  later design does it, a segment that continues as new starts with an activity, and the guide says why.
- **Gap 054-010** Design 045 names its refusal `ACTIVITY_TIMEOUTS_NOT_ACTIVE`; Decision 054-014 asks it to use
  `CAPABILITY_NOT_ACTIVE` with the capability named, given by the shared hold of Task 054-18.

## Owner questions

Owner questions 054-001 to 054-008 are decided by Decisions 054-001 to 054-008 above. None is open.
