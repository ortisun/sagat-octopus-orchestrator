# Runbook: reset a run to a point (feature 018)

For operators who start a run again from the middle of one that ended ("the first two steps were right, the third was wrong")
and for the people who answer for what that repeats. It says what a reset does and promises, which points exist, what to read
before confirming, and what to do when a reset is refused.

Design: decisions D1976 to D1979, D1982 to D1995, D1996 to D2005, D2059 to D2068 and D2079 to D2082 (the expiry), gaps G1980, G1982 to G1988, G1989 to G1992, G2015 to G2018 and G2024 to G2026 in [`specs/018-schedules-bulk-operations/research.md`](../../specs/018-schedules-bulk-operations/research.md);
the contract is the `Operations` tag of [`openapi.yaml`](../../specs/001-platform-foundation/contracts/openapi.yaml)
(`listResetPoints`, `previewReset`, `resetRun`, `listRunResets`, `getRunReset`).

## 1. What a reset is and what it promises

A reset **creates a new run** from a prefix of the journal of a run that has ended. Nothing is changed in the source. (A run
that is still open can be reset too, by cancelling it first and waiting for it to end: section 4.2.)

- **The same workflow version and the same input** as the source. The new run is a run like any other: it has its own id, its
  own deadline and its own trace. It names its origin (`rerunOf` on the run) and the source lists it in its resets.
- **The prefix is copied, not replayed.** The new run's journal starts with the events `1..N` of the source exactly as the source
  stored them: who did each thing and when are the source's. The first of them says it was restored, and a `run.reset` event
  follows the prefix, carrying the SHA-256 of the prefix, so a reader can see where the copy ends and the new run begins.
  Outputs of the steps in the prefix are the source's; none of them runs again.
- **From the point on, the run is the platform's.** The steps after the point run again, as new attempts of the new run.
- **Whose authority.** To list the points and preview, `run:operate` and `run:read`; to reset, `run:operate` and `run:start`
  (the same as starting a run again from an ended one). The reset is audited as `run.reset` on the new run, with the reason you
  gave and the sequence and source it came from.
- **A reason is required**: 8 to 2,000 characters of plain text, kept with the reset in the register and in the audit record.
- **A repeat is the same reset.** The request carries an `Idempotency-Key`; the same key and the same request answers the first
  reset (the same reset id and the same new run). The same key with another sequence is `409 IDEMPOTENCY_KEY_REUSED`.
- **The business identifier.** With `businessId: KEEP` (the default) the new run takes over the source's identifier and the
  reset is refused (`409 BUSINESS_ID_IN_USE`) if another open run holds it; with `DROP` the new run has none.

What it does **not** promise: a step that runs again repeats whatever it does outside the platform. The platform does not undo
what the source did, and an activity that is not idempotent on its own business key will do it twice. That is why the preview
tells you, step by step, what runs again (section 3), and why the reset needs your acknowledgement when anything that
completed in the source will run again.

## 2. Which points exist

`GET /runs/{id}/reset-points` (Studio: *Reset to a point…* on the run; CLI: `sagat workflow reset-points <run>`) lists, by
ascending sequence, the places the run can be reset to: **the start of the run** and **the end of a step, a child run, a
collection or a loop at the root of the run, where the run was quiet**: nothing in flight, not paused, not stopping.

A point is not listed when its prefix holds:

| What | Why |
|---|---|
| an event of a kind this version has no reset rule for | a new kind of event must declare how a reset treats it before it can be copied; `RESET_RULE_MISSING` names the event type |
| a terminal tail, a compensation or a chain hand-off | these end or undo the run; they are not a place to start again from |
| an open window, work or scope (an attempt in flight, a wait, a timer, a retry, a branch, a collection or loop not finished successfully) | the new run could not know what to do about it |
| an event of an iteration (a scope below the root of the run) | this version restores the events of the root only (G1980 (1)) |
| something that happened to a step later in the source (compensated, skipped, resolved or reconciled) | the new run, which would not have had that happen, cannot inherit it |

Also:

- **At most 5,000 events** are copied (`RESET_PREFIX_EVENTS`, a constant of the application): a longer prefix is refused
  (`RESET_PREFIX_TOO_LONG`) because the copy is one transaction. Choose an earlier point. The ceiling is not a setting yet.
- **Not every run can be reset**: a child run, a member of a continuation chain and a run of a database execution are refused
  (`RESET_NOT_SUPPORTED_FOR_SOURCE`).
- **The source must have ended.** A run that is still open is `409 RESET_SOURCE_NOT_TERMINAL`: wait for it, or cancel it
  yourself first. `sourcePolicy: CANCEL_SOURCE` resets an open run by cancelling it (section 4.2; it is off unless the deployment turns it on).

### 2.1 What a reset costs, and what that means for the person who runs it

**Measurements of one machine on one day, not promises** (NFR-018-005; an Apple M3 Max, PostgreSQL 16 in Docker with `fsync` on, the release build, the runtime role, sources built by a worker one activity after another; the report, its method and its failures: [`tests/performance/reports/schedules-bulk-operations.md`](../../tests/performance/reports/schedules-bulk-operations.md), G2046, G2047).

- **A reset holds the row of its source run locked for the whole re-emission, about 0.93 ms for every event of the prefix.** The source row is taken at the start of the transaction that creates the new run and released at its commit, and the events are written one statement at a time: 0.95 s at 1,000 events and **4.65 s at 5,000**, the most a reset accepts (`RESET_PREFIX_EVENTS`). Every other writer of that run (a cancel, a retention sweep, another reset) waits that long. The reset as a whole took 4.7 s at 5,000 events, **0.15 to 0.24 s inside the 5 s statement timeout of the pool**: a reset near the ceiling on a slower or a busier database can run into it (what the caller then sees was not measured), so prefer an earlier point for a long run. About 19 s would be the lock of a prefix of 20,000 events (the hard ceiling the plan names; a linear extrapolation, because no code path accepts it): **do not raise the ceiling toward it** before the restore writes the events in pages.
- **The list of points, the preview and the reset read and replay the whole journal of the source, and the replay costs more than linearly in the length of the journal** (the length of the journal, not of the prefix): the list of points took 9 ms at 1,002 events, 89 ms at 5,002 and **1.3 s at 20,002**; the preview of the same 5,000-event prefix took 98 ms on a journal of 5,002 events and 1.4 s on one of 20,002, and the reset of it 6.0 s (the extra 1.3 s is before the transaction, so the lock is no longer). A run with a journal of hundreds of thousands of events would make the list of points a request of many seconds.
- **A point past the ceiling is refused cheaply**: `RESET_PREFIX_TOO_LONG` in 46 ms at the median (158 ms the slowest) on a journal of 20,002 events, before the replay, creating no run.
- **What the reset made is exact**: replay equals projection (`verify_run`) held for all 45 runs the benchmark's resets made, up to 5,000 restored events (20 ms to check).

What that means for you: choose the earliest point that serves; do not run resets of long runs in bulk (each holds its source for about a millisecond an event); expect the Studio's point list of a very long run to be slow; and read the lock time before anyone asks for a higher ceiling.

## 3. What to read before you confirm

`POST /runs/{id}/reset-previews` with `{sequence}` (Studio: choosing the point; CLI: `sagat workflow reset-preview <run> --sequence N`)
changes nothing and answers:

- `restored`: the steps the new run keeps as they were (a decision of an approval is kept as a decision, not asked again).
- `reexecuted`: the steps the source ran **after** the point, which the new run runs again. For each: whether it **completed in
  the source** (`completedInSource`: running it again repeats an effect that already happened) and the markers the workflow
  declared on it: `compensateWith`, `reconcileWith`, `irreversible`, `highRisk`. Read the `irreversible` and `highRisk` ones twice.
- `requiresAcknowledgement`: true when any step that completed in the source will run again. The reset is then refused
  (`422 RESET_REEXECUTION_NOT_ACKNOWLEDGED`) unless the request says `acknowledgeReexecution: true` (Studio: the checkbox;
  CLI: `--acknowledge`).
- `prefixEvents`: how many events of the source the new run re-emits.
- `reapply`: with `reapply` in the request (`SIGNALS`, `UPDATES`, `ALL`; section 4) the signals and Updates the source received
  after the point that the reset would deliver again, in the order they arrived: each is `WILL_TRY`, or `NOT_REAPPLIED` with the
  reason it already has. Empty for `NONE`, the default.

A sequence that is not a point is `422 RESET_POINT_NOT_ELIGIBLE`; choose one the list names.

## 4. Doing it and reading what became of it

`POST /runs/{id}/resets` with `{sequence, reason, acknowledgeReexecution?, businessId?, reapply?, sourcePolicy?}` (Studio: the dialog; CLI:
`sagat workflow reset <run> --sequence N --reason "…" [--acknowledge] [--drop-business-id] [--reapply none|signals|updates|all] [--cancel-source] [--key K]`)
answers `202` with the reset:

| Field | Meaning |
|---|---|
| `resetId`, `sourceRunId`, `sequence` | the register entry, the run it came from and the last event kept |
| `state` | `CREATED`: the new run exists. `PENDING_SOURCE`: the source is being cancelled and the run is not made yet (`CANCEL_SOURCE`, section 4.2). `FAILED`: it will not be made, and `failureReason` says why (section 4.2) |
| `newRunId` | the run to follow; read it as any run (`GET /runs/{newRunId}`); `null` until the run is made |
| `failureReason` | why a reset that waited failed; `null` otherwise |
| `reapplyReport` | what the reset delivered again, one entry for each signal or Update, in the order they arrived (section 4.1); empty for `NONE` |

`GET /runs/{id}/resets` lists the resets made from a run, newest first (at most 200); `GET /runs/{id}/resets/{resetId}` reads one. The
register entry is written in the transaction that creates the run, so a reset that crashed half way leaves no entry and no run,
and the same request sent again makes one.

### 4.1 Delivering signals and updates again

By default a reset delivers nothing after the point: if the run needs a signal the source received, you send it. With `reapply`
the reset sends it for you, **in the transaction that creates the run**, so the new run is never seen without it and a crash
leaves no half delivery:

| `reapply` | What is delivered again |
|---|---|
| `NONE` (default) | nothing |
| `SIGNALS` | each signal the source received after the point (`wait.satisfied` of a step at the root, recorded `via: signal`), with the payload the source gave it |
| `UPDATES` | each declared Update after the point whose declaration says `reapplyOnReset: true` (its arguments were kept for this), with those arguments |
| `ALL` | both, interleaved as they arrived |

What the report says. Each entry is `{sequence, kind: SIGNAL|UPDATE, name|operation, outcome, reason?}`: the row of the source it
is, the name of the signal or of the operation, and what came of it. It **never holds a payload, an argument or a digest of
them**. A repeat of the same request answers the same report.

| `outcome` | `reason` | Meaning |
|---|---|---|
| `REAPPLIED` | | delivered; the new run's journal has it as a signal or an Update of its own, audited by the caller |
| `REJECTED` | `NOT_WAITING` | no step of the new run was waiting for the signal. **It ends the signals**: the ones after it are not tried |
| `REJECTED` | `UPDATE_REJECTED` | the validator of the Update declined it |
| `REJECTED` | `REVISION_CONFLICT` | an Update is applied only to the state it was applied to in the source; the new run's state is at another revision (an earlier Update was not reapplied) |
| `REJECTED` | `UPDATE_NOT_ACCEPTED` | the declaration does not accept the arguments or the state they make |
| `NOT_REAPPLIED` | `NOT_RETAINED` | the payload or the arguments are no longer kept |
| `NOT_REAPPLIED` | `LEGACY_EVENT` | the row was written before the journal said what satisfied the wait or whether the Update kept its arguments, so it cannot be told from what must not be sent again |
| `NOT_REAPPLIED` | `PREVIOUS_SIGNAL_NOT_DELIVERED` | an earlier signal was not delivered (it was rejected, not kept or legacy), and delivering this one would reach the wait the earlier was meant for |

Read it as a list of what the new run now has and does not have. The usual cause of a `REJECTED` `NOT_WAITING` is a step between
two signals that has to run again first: the new run is not yet at the wait the second signal was for. Send it yourself when it
is.

What is never delivered again: a wait an **event** satisfied, the signal of a **Signal-with-Start** (the caller asked for it in
one request with the start), and anything below the root of the run (an iteration). An Update that did not opt in with
`reapplyOnReset` is not a candidate: its arguments were not kept.

Limits. At most **256** candidates (those that are only reported count): more is `422 RESET_REAPPLY_TOO_MANY`, for the preview as
well, and nothing is dropped; pick a later point. The reset needs `run:signal` for `SIGNALS` and `ALL`, `run:update` for
`UPDATES` and `ALL`. A reset of a reset delivers the Updates of the first source too (their arguments are followed through
the resets that made the run, **at most 64 resets deep**; past that an Update says `NOT_RETAINED`). The preview reads how long
what is kept is, not what it says, so it says `WILL_TRY` for what is kept; the reset can still find that arguments that are
kept do not read as arguments, and say `NOT_RETAINED`.

Equal signals. Two equal signals (the same name and payload) that a reset delivers one after the other are two signals in the
new run, and a reset of that reset delivers both again: they are told apart by the journal (the second step began to wait only
after the first signal), not by the clock. When that cannot be shown, the entries are two.

Audit. `run.reset` is audited with your justification and, when something is delivered again, the closed word of the policy
(`… reapply=ALL: <justification>`); each delivery is audited as the signal or the Update it is, by the caller, and carries the trace
id of the request, which is how to join it to the reset. Neither says what was sent.

Keys. What the platform sends under a key of its own, `reset:{reset id}:{sequence of the row}`, is audited like any signal or
Update. The prefix `reset:` (like `batch:` and `schedule:`) cannot be the `Idempotency-Key` of a call or the update key of
`updateRun`; it is refused `400` / `INVALID_UPDATE_KEY`.

#### Turning it on

It is a **deployment setting**, off by default: `SAGAT_RESET_REAPPLY_ENABLED=true` on the **control plane**, which serves the
reset and publishes workflows. The engine has no setting: it reads the versions it was given, whatever the setting says, and a
reset is made by the control plane. With it off, a preview or a reset with `reapply` other than `NONE` is
`422 RESET_REAPPLY_NOT_AVAILABLE` and nothing is made; `NONE` is served as always. **A retry of a reset that succeeded is answered
with what it made even where the setting is off now** (the `Idempotency-Key` promise holds): only a request that was never made
is refused. With it on, a workflow may declare an Update with `reapplyOnReset: true` (ADR-0032: a new publication that declares
it is refused where the setting is off). Replicas of the control plane read it each; during a rollout a request can be served by
one and refused by another.

What happens to workflows published before:

- A version published **with the setting on** keeps its declaration and its retained arguments whatever the setting says later;
  turning it off stops the reset from delivering them and does not erase anything. Turning it on again serves them.
- A version published **before** the field existed has no Update that opted in: its Updates are not candidates (`retained: false`,
  or `LEGACY_EVENT` for a row older than the marker). Publish a new version that opts in; the runs of the old version cannot
  be reapplied.

Retained arguments are content of the tenant: they are charged to the tenant's content quota like any payload (once, however many
runs apply the same arguments). Read the tenant's usage before turning the setting on for a workflow with large or frequent
Updates.

In traces, the new run's span **links** the source's span and does not emit the restored prefix again: its first spans are the
steps after the point. Whatever you read in the prefix of the new run's journal happened in the source, at the source's time.

### 4.2 Resetting a run that has not ended (`CANCEL_SOURCE`)

`sourcePolicy: CANCEL_SOURCE` (Studio: *Cancel and reset to a point…* on a run that is open; CLI: `--cancel-source`) resets a run
that is still open: the run is **cancelled in the ordinary way** (it is never terminated; the steps in flight are told to stop
and may finish), and the new run is made **once the source has ended**, if the point is still one. Nothing is made while the
source is open, and the source never reopens.

What the request does, in one transaction that holds the row of the source: it checks what a reset of an ended run checks
(`run:operate`, `run:start`, the permissions of the `reapply`, the justification, that the point is one of the journal as it is
now, the acknowledgement and the length of the prefix), judges the namespace's limit, writes the entry of the register
`PENDING_SOURCE`, cancels the source, writes the receipt of the key and two audit records (`run.cancel_requested`, which says
only that the cancellation is for this reset, and `run.reset`, which carries your justification). It answers `202` with the entry:
`state: PENDING_SOURCE`, `newRunId: null`. A source that is already being cancelled or compensated is not cancelled twice; the
entry waits for it all the same. A source that **has already ended** is simply reset at once (`CREATED`), whatever the policy
says and whether or not this is turned on.

**The acknowledgement is asked before anything is cancelled.** A cancellation is not taken back, so a request that is certain to
fail later is refused now: `acknowledgeReexecution: true` is required (`422 RESET_REEXECUTION_NOT_ACKNOWLEDGED`, nothing
cancelled) when a step after the point has completed **or is in flight** (leased or running: the attempt usually finishes after
the cancellation, and the new run then runs that step again from the point). The preview (`requiresAcknowledgement`) says the
same for a run that is open. The engine judges it again on the journal the source ended with (below), so an acknowledgement
that the request did not need is harmless and one that it needed is never skipped.

**Turning it on.** `SAGAT_RESET_CANCEL_SOURCE_ENABLED=true` on the control plane (it accepts the request) **and** on the engine
(it makes the run); both read the same variable and it means the same, off by default (`true`/`false`, `yes`/`no`, `1`/`0`;
unset or empty is off; anything else is refused naming the variable). The engine also reads `SAGAT_RESET_CANCEL_SOURCE_BATCH`
(entries completed in a round, 1 to 1,000, default 100), `SAGAT_RESET_CANCEL_SOURCE_PER_TENANT` (how many of them one tenant may
take of a round, 1 to 1,000, default 10, so that one tenant's entries cannot fill the round), `SAGAT_RESET_CANCEL_SOURCE_POLL_MS`
(pause when a round found nothing, default 1,000) and `SAGAT_RESET_CANCEL_SOURCE_EXPIRY_SECS` (how long an entry waits for its source to end,
in seconds: from 3,600, an hour, to 604,800, seven days, both included; default 86,400, a day; anything else, a unit (`24h`) or a
fraction, refuses to start naming the variable; **the engine alone reads it**, the control plane expires nothing and does not need it).
**Set the same value on every engine:** an engine applies its window to the entries of the shards it holds, and shards move between
engines, so the window an entry is judged by is the one of the engine that holds its shard when a round looks (D2082, G2025); each
engine logs its `expiry_secs`, `batch` and `per_tenant` once when the pass starts. **The flag governs the completion and the expiry
together:** with it off in an engine, that engine does neither (see the order of the rollout below).
Where the control plane has it off, a request for an open source is
`422 RESET_CANCEL_SOURCE_NOT_AVAILABLE` and nothing is cancelled; **a retry of a request that was accepted is answered from the
register whatever the setting is now**.

**Order of the rollout (ADR-0081): the engine first, then the control plane; to turn it off, the control plane first.** The
dangerous state is the control plane on and every engine off: a request is then accepted and answers `202`, **the customer's run
is cancelled**, and nothing makes the replacement until an engine has the flag on. No entry is destroyed (it waits in the
register, counted against the limit), **but nothing expires it either: the expiry is a step of the same pass, so an engine with the
flag off neither completes nor expires** (D2080, G2024). Every entry that was accepted keeps its place in its namespace, up to the
limit, **until an engine with the flag on runs**; then the runs are made for the sources that have ended (G1989) and **every entry
older than the window expires at once, at most 100 a second** (the pause after a round that expired entries, so that the writes
keep to that ceiling: the pause is added to the time of the round, so the pace is lower, and it was not measured for this pass). `scheduler_lag_seconds{pass="resets"}` grows once the sources have ended (the alert `SagatSchedulerLate`); for an entry
whose source is still open nothing fires, and `workflow_resets_pending` staying high is the sign. An engine with the flag off says
so in its log when it starts. The registry that would refuse the
mismatch is not built (ADR-0050 decision 9, T018-080), so do not leave the control plane on while the engines are being
changed. The engine takes the entries of the shards it holds, like the pass of the batch jobs: several engines never make one
run twice (a lease that moved, or an entry that another engine completed first, refuses the step and changes nothing).

**What the engine checks again when the source has ended**, on the journal the source ended with and with the authority the
member who asked holds **then** (their membership is read again, and locked, in the transaction that creates the run):

| It fails with | When |
|---|---|
| `BOUNDARY_NO_LONGER_ELIGIBLE` | the prefix is not the one that was asked for, or the point is no longer one, or the prefix is too long |
| `REEXECUTION_NOT_ACKNOWLEDGED` | a step completed after the point and the request did not acknowledge that steps run again. The request is refused for the steps it can see (completed, or in flight), so this is the rare case of a step that was leased only after the request was made; ask with `acknowledgeReexecution: true` when you expect it. The Studio always asks |
| `AUTHORITY_REVOKED` | the member who asked is no longer a member, or lost `run:operate`, `run:start`, or the right to deliver again what the `reapply` delivers |
| `BUSINESS_ID_IN_USE` | `KEEP` and another open run holds the identifier by now |
| `ADMISSION_REFUSED` | the namespace has no room for the run, or the platform refused it |
| `REAPPLY_TOO_MANY` | the source received more to deliver again than a reset delivers (256) |

The entry becomes `FAILED` with that reason (`failureReason`) and the platform records `run.reset` on the source with the closed
reason, never your justification. A reason this version does not know is a reason a later version added: read it as another
reason. The `reapply` the request asked for is delivered even where the engine has no setting for it: what the control plane
accepted stands.

**A failed reset leaves a cancelled run and no replacement.** The request cancelled the source and nothing takes that back, so
when `workflow_reset_completions_total{outcome="failed"}` rises, read `GET /runs/{id}/resets` for the reasons (the label says
the same) and start the run again by hand (`sagat workflow start`, or a new reset of the ended source with the reason fixed:
the ended source is reset at once). Only these reasons, and the expiry below (`SOURCE_DID_NOT_END`, which `outcome="failed"` does not count),
end an entry; a failure of the platform itself (storage, a lease that
moved, a connection limit, an error nobody foresaw) leaves the entry pending and is tried again after a pause that doubles from
2 s to 5 minutes, with a warning in the engine's log that names `tenant_id` and `reset_id`; one entry in that state does not
hold the others (each tenant takes at most `SAGAT_RESET_CANCEL_SOURCE_PER_TENANT` places of a round, and an entry that failed
is left out of the scan until its time).

**What an operator can do about an entry that waits: end the source, or let the entry expire. There is no call to withdraw it**
(G1991, G2018: the owner decided the expiry, OD-2026-11-08-1, and left the withdrawal for later). Removing the member, or their
`run:operate` or `run:start`, makes the engine end the entry `FAILED` `AUTHORITY_REVOKED` **when the source has ended**, not
before: it does not free the place of the namespace while the source is open. The ways to free the place are to end the source
(retry, skip or resolve the step, or cancel the run's compensation; the run is cancelled already, so it ends once what is in
flight has reported), which makes the run the entry asked for (or fails it with one of the reasons above), or to wait for the
expiry (below). An administrator can raise the limit of a namespace up to the hard ceiling of 64, no further.

**An entry whose source does not end expires** (D2059). An entry that is still `PENDING_SOURCE` **a whole window after its request**
(24 hours; the deployment sets 1 hour to 7 days with `SAGAT_RESET_CANCEL_SOURCE_EXPIRY_SECS` on the engine) while its source is
still open (a run in `MANUAL_REVIEW` is open) ends `FAILED` with the reason **`SOURCE_DID_NOT_END`**: no run is made, the entry
leaves `workflow_resets_pending` and its place in the namespace is free again (a new request is accepted at the limit). What the
operator sees: `GET /runs/{id}/resets` and the Studio and the CLI show the entry as one that could not be made, with
`failureReason: SOURCE_DID_NOT_END` in words; `workflow_reset_completions_total{outcome="expired",reason="SOURCE_DID_NOT_END"}` rises
by one; the audit trail of the source has one `run.reset` record by `system:engine` that says `completes reset {id}: failed
SOURCE_DID_NOT_END`, and nothing the requester wrote; the engine's log has one line with `tenant_id`, `reset_id` and the window.
What the expiry does **not** do: it never touches the source, which the request cancelled and which stays cancelled (still ending,
or in manual review, for as long as it takes), and it makes no run. **What to do:** end the source before the window is over (the
steps in the table of the source are the same as for any run that does not end), or let the entry expire; once the source has
ended, reset it again with a new `Idempotency-Key`, which is made at once (a retry with the key of the request that expired is
answered from the register, with the entry as it is now: `FAILED`). **An ended source always wins**: an entry whose source has
ended is completed in the next round even if the window is long over, and an entry is never both expired and completed. The window
is counted **from the request**, on the database's clock, so a restart of the engine does not start it again; the first round
of an engine with this version expires, a few at a time, **every entry that is already older than the window** (read the register
before you turn the flag on with entries in it). A round expires at most `SAGAT_RESET_CANCEL_SOURCE_BATCH` entries and
`SAGAT_RESET_CANCEL_SOURCE_PER_TENANT` of any one tenant, oldest first, apart from the completions, **and a round that expired entries
is followed by the pause they cost at a hundred a second, a ceiling on the pace** (a full round of 100 rests a second; at least 200 ms for any;
the time of the round is added to it, so the pace is lower, and it was not measured for this pass), so the first
rounds of an engine with this version expire the old entries at that pace at the most and not as fast as the database allows; a failure of the
platform in one entry leaves it pending, with a warning that names `tenant_id` and `reset_id`, and does not stop the others; an entry
past its window whose source **another transaction holds at that moment** (the one that may be ending the run) is left for a
second and tried again, without a warning (a debug line names the ids). The migration
`reset_expiry` (the list of the reasons the register admits) must be applied **before** an engine with this version runs: against
the older schema the write of the reason is refused and every expiry stays pending with that warning.

**Reading the audit trail: the join is by the reset id.** The four records of one reset name the id of the entry in their
reason and nothing else of it but closed words: `reset {id} requested at {N}...` and `cancelled for the reset {id}` on the
source (the request), `reset from {source} at {N}... completes reset {id}: ...` on the run that was made, and
`completes reset {id}: failed {REASON}` on the source when the entry failed (the completion). The id is the one that
`GET /runs/{id}/resets` and `sagat workflow resets` show; the trace id of the request is on the request's records only,
because the engine's pass has no request.

**An entry waits for its source for the window, and no longer.** A source in `MANUAL_REVIEW` (a compensation that
failed, an unknown outcome) is open: the entry waits for an operator to act on the run (retry, skip or resolve the step); when
the run ends the entry is completed in the next round, **if that is within the window**; if the window is over first, the entry
expired (above). There is no call that withdraws an entry (G1991, G2018): it ends `CREATED` or `FAILED`. Entries count against the
namespace's limit while they wait: **16 per namespace by default, at most 64** (`429 QUOTA_EXCEEDED`, `details.reason`
`PENDING_RESET_LIMIT`), so a source that never ends holds a place for the window at most **where an engine runs the pass: with the flag
off in every engine nothing expires an entry and the places stay pinned until one runs** (G2024). `GET /runs/{id}/resets`,
`sagat workflow resets <run>` and the Studio list the entries with their state and reason.

## 5. When a reset is refused

| Answer | Reason | What to do |
|---|---|---|
| `409` | `RESET_SOURCE_NOT_TERMINAL` | the run has not ended; wait for it, or ask for `sourcePolicy: CANCEL_SOURCE` (section 4.2) |
| `422` | `RESET_CANCEL_SOURCE_NOT_AVAILABLE` | `CANCEL_SOURCE` for a run that is open, on a deployment whose operator did not turn `SAGAT_RESET_CANCEL_SOURCE_ENABLED` on; wait for the run to end, or turn it on (section 4.2). Nothing was cancelled |
| `429` | `PENDING_RESET_LIMIT` | the namespace already has the most resets waiting for their source that it may (16 by default, at most 64); wait for one to end or raise the limit |
| `422` | `RESET_POINT_NOT_ELIGIBLE` | the sequence is not a point (or its prefix is blocked, section 2); pick one `reset-points` lists |
| `422` | `RESET_PREFIX_TOO_LONG` | more than 5,000 events before that point; pick an earlier one |
| `422` | `RESET_REEXECUTION_NOT_ACKNOWLEDGED` | steps that completed will run again; read the preview, then acknowledge |
| `422` | `RESET_NOT_SUPPORTED_FOR_SOURCE` | a child run, a chain member or a database execution; reset the parent or start a new run |
| `422` | `RESET_RULE_MISSING` | the prefix holds an event this version has no rule for (`details.eventType` names it); pick an earlier point |
| `409` | `BUSINESS_ID_IN_USE` | another open run holds the identifier; end it, or reset with `DROP` |
| `409` | `IDEMPOTENCY_KEY_REUSED` | the same key was used for another request; use a new key |
| `422` | `RESET_REAPPLY_NOT_AVAILABLE` | `reapply` other than `NONE` on a deployment whose operator did not turn `SAGAT_RESET_REAPPLY_ENABLED` on; ask for `NONE` or turn it on (section 4.1). The sentence the API gives a caller does not name the variable |
| `422` | `RESET_REAPPLY_TOO_MANY` | more than 256 signals and retained Updates after the point; pick a later point or `NONE` |
| `422`, no reason | a `sourcePolicy`, a `reapply` or a `businessId` the server does not know, or a reason of fewer than 8 characters | not a policy, or too short (a reason is missing from these answers: G1981) |
| `400` | | the `Idempotency-Key` starts with `batch:`, `schedule:` or `reset:` (the platform's own) |
| `403` / `404` | | the caller lacks `run:operate`/`run:start`; or the run does not exist or is another tenant's (the two look the same) |

## 6. What it counts

| Metric | Labels | Meaning |
|---|---|---|
| `workflow_reset_reapply_total` | `kind` (`SIGNAL`, `UPDATE`), `outcome` (`REAPPLIED`, `REJECTED`, `NOT_REAPPLIED`), `reason` (the closed reasons of section 4.1, `none` when there is none) | What a reset delivered again, counted once the reset is committed: a reset that was refused, cut off or answered from the register counts nothing. No tenant, run, signal or operation is a label |
| `workflow_reset_completions_total` | `outcome` (`created`, `failed`, `expired`), `reason` (`none` for `created`; for `failed` one of the six closed reasons of the table of section 4.2; for `expired` always `SOURCE_DID_NOT_END`) | The resets that waited for their source and were completed or expired, counted once the transaction is committed: a step on an entry that was completed first, whose source is open and is not past its window, whose source is in the hands of another transaction, or that lost its lease counts nothing. `failed` is the judgement of the platform on the journal the source ended with and does not count an expiry. No tenant, run or reset is a label |
| `workflow_resets_pending` | | Resets waiting for their source (gauge, from the monitor of the engine); it falls when an entry is completed, fails or expires. **Every replica sets it to the same count**, so read it with `max`, never `sum`: a sum multiplies the count by the number of replicas |
| `scheduler_fenced_total{kind="reset"}` | `kind` | Steps of the pass that were refused because the replica no longer held the lease of the shard (an entry that another engine completed first is not counted: it is an ordinary race). A few during a rebalance are normal; a count that keeps rising says two replicas dispute a shard |
| `workflow_reset_completions_total{outcome="failed"}` | `reason` | An entry ended `FAILED`: the source was cancelled and no run replaces it. Read the reasons (section 4.2) and start the run again by hand if it is still wanted |
| `scheduler_lag_seconds{pass="resets"}` | | How long the oldest pending reset whose source has **ended** has waited for the engine to make its run (0 when none): the engine has the pass off, no replica holds the shard, or it is failing (look at its log). A source that is still open is not lag: the entry waits for the run |

A rising `NOT_WAITING` or `REVISION_CONFLICT` says that resets are made to points where what followed does not fit: read the report
of one of them (`GET /runs/{id}/resets`). The Updates a reset delivers again do not have the span and the latency histogram of
an Update made through the API (G1986); their events are in the journal, and the telemetry derives their spans from it.

## 7. What is not built yet

- **Delivering again what was received below the root** (an iteration of a collection or a loop; G1983): not delivered, and not
  reported.
- **Withdrawing a reset that waits** (G1991, G2018): an entry ends `CREATED` or `FAILED` (by the judgement of the platform, or by expiring) and nothing else. The expiry
  is built (section 4.2); **no alert fires when one expires and nothing says that an entry is about to** (G2016): read `workflow_reset_completions_total{outcome="expired"}`.
  An alert on `increase(workflow_reset_completions_total{outcome="expired"}[1h]) > 0`, **with its `promtool` rule test, is a precondition for turning the feature on by default**
  (ADR-0081 decision 4, G2026); none is written until it can be tested.
- **A permit of a concurrency key and the checks of the features that are not approved** (`CONCURRENCY_PERMIT_UNAVAILABLE`,
  `RUN_PURGED` and the reasons of a `REMATERIALIZE` rule exist as names; nothing writes them yet).
- **Points inside a collection, a parallel branch, a loop iteration or a child scope** (G1980 (1)).
- **A setting for the ceiling** (G1980 (2)).
- **The permit of a concurrency key** (feature 022, T018-053): a reset takes none; that feature is not approved, so there is no
  `CONCURRENCY_PERMIT_UNAVAILABLE` to read yet.
- **Purging the register** (T018-051): the entries of `run_resets` are kept as long as the runs are.
