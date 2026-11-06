# Runbook: reset a run to a point (feature 018)

For operators who start a run again from the middle of one that ended ("the first two steps were right, the third was wrong")
and for the people who answer for what that repeats. It says what a reset does and promises, which points exist, what to read
before confirming, and what to do when a reset is refused.

Design: decisions D1976 to D1979 and gap G1980 in [`specs/018-schedules-bulk-operations/research.md`](../../specs/018-schedules-bulk-operations/research.md);
the contract is the `Operations` tag of [`openapi.yaml`](../../specs/001-platform-foundation/contracts/openapi.yaml)
(`listResetPoints`, `previewReset`, `resetRun`, `listRunResets`, `getRunReset`).

## 1. What a reset is and what it promises

A reset **creates a new run** from a prefix of the journal of a run that has ended. Nothing is changed in the source.

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
  yourself first. `sourcePolicy: CANCEL_SOURCE` (reset an open run by cancelling it) is not built (section 6).

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
- `reapply`: always empty in this version (section 6).

A sequence that is not a point is `422 RESET_POINT_NOT_ELIGIBLE`; choose one the list names.

## 4. Doing it and reading what became of it

`POST /runs/{id}/resets` with `{sequence, reason, acknowledgeReexecution?, businessId?}` (Studio: the dialog; CLI:
`sagat workflow reset <run> --sequence N --reason "…" [--acknowledge] [--drop-business-id] [--key K]`) answers `202` with the reset:

| Field | Meaning |
|---|---|
| `resetId`, `sourceRunId`, `sequence` | the register entry, the run it came from and the last event kept |
| `state` | `CREATED`: the new run exists. `PENDING_SOURCE` and `FAILED` belong to the reset of an open source, which is not built, so a reset you can make today answers `CREATED` |
| `newRunId` | the run to follow; read it as any run (`GET /runs/{newRunId}`) |
| `reapplyReport` | always empty in this version |

`GET /runs/{id}/resets` lists the resets made from a run, newest first (at most 200); `GET /runs/{id}/resets/{resetId}` reads one. The
register entry is written in the transaction that creates the run, so a reset that crashed half way leaves no entry and no run,
and the same request sent again makes one.

In traces, the new run's span **links** the source's span and does not emit the restored prefix again: its first spans are the
steps after the point. Whatever you read in the prefix of the new run's journal happened in the source, at the source's time.

## 5. When a reset is refused

| Answer | Reason | What to do |
|---|---|---|
| `409` | `RESET_SOURCE_NOT_TERMINAL` | the run has not ended; wait for it, or cancel it and reset the ended run |
| `422` | `RESET_POINT_NOT_ELIGIBLE` | the sequence is not a point (or its prefix is blocked, section 2); pick one `reset-points` lists |
| `422` | `RESET_PREFIX_TOO_LONG` | more than 5,000 events before that point; pick an earlier one |
| `422` | `RESET_REEXECUTION_NOT_ACKNOWLEDGED` | steps that completed will run again; read the preview, then acknowledge |
| `422` | `RESET_NOT_SUPPORTED_FOR_SOURCE` | a child run, a chain member or a database execution; reset the parent or start a new run |
| `422` | `RESET_RULE_MISSING` | the prefix holds an event this version has no rule for (`details.eventType` names it); pick an earlier point |
| `409` | `BUSINESS_ID_IN_USE` | another open run holds the identifier; end it, or reset with `DROP` |
| `409` | `IDEMPOTENCY_KEY_REUSED` | the same key was used for another request; use a new key |
| `422`, no reason | `reapply` other than `NONE`, `sourcePolicy: CANCEL_SOURCE`, or a reason of fewer than 8 characters | not available in this version, or too short (a reason is missing from these answers: G1981) |
| `403` / `404` | | the caller lacks `run:operate`/`run:start`; or the run does not exist or is another tenant's (the two look the same) |

## 6. What is not built yet

- **Delivering signals and updates again** (`reapply` `SIGNALS`, `UPDATES`, `ALL`; T018-056): refused. A signal or an update the
  source received after the point is not delivered to the new run; if the run needs it, send it yourself.
- **Resetting an open run** (`sourcePolicy: CANCEL_SOURCE`; T018-057): refused; the states `PENDING_SOURCE` and `FAILED` of a
  reset exist in the register for it and are never written today.
- **Points inside a collection, a parallel branch, a loop iteration or a child scope** (G1980 (1)).
- **A setting for the ceiling** (G1980 (2)).
- **The permit of a concurrency key** (feature 022, T018-053): a reset takes none; that feature is not approved, so there is no
  `CONCURRENCY_PERMIT_UNAVAILABLE` to read yet.
- **Purging the register** (T018-051): the entries of `run_resets` are kept as long as the runs are.
