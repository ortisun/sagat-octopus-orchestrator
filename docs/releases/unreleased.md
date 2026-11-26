# Unreleased integration changes

These changes are on the project completion feature branch. This is not a published release.
Final compatibility, fault, canary, performance and workspace gates remain in the feature tasks.

## Breaking (security): no read returns a digest of a payload (feature 016, bugfix T016-066, G665)

Two reads returned the unsalted SHA-256 of a payload the platform keeps. A reader could hash guesses (`true`, a date, a
national identifier) and compare, offline, without a trace in the audit. That confirmed values of `pii` and `secret`
outputs and inputs they may not see (severity High, OD-2026-11-26-566). Both now return something that does not depend
on the payload:

- **`GET /runs/{runId}/events`**: `payloadRef` keeps its name and type. Its value is now `journal:<runId>:<seq>`, the
  place of the payload, where it used to be a 64-character hexadecimal digest. It is still `null` for an event that
  carries no payload. Two equal outputs now have different references.
- **Schedules** (every answer that carries a schedule): `action.inputDigest` is gone and `action.hasInput` (boolean)
  says whether the action has an input. The typed schedule action of the six SDKs changes the same way:
  `inputDigest`/`input_digest`/`InputDigest` becomes `hasInput`/`has_input`/`HasInput`. This is a compile-time break,
  made on purpose (see below).

There is no setting that restores the digest. What to do:

- *You tested `payloadRef !== null` to find the events with a payload, or showed it as an opaque label*: nothing
  changes.
- *You passed the event's `seq` to `payload-inspections` or `payload-disclosures`*: nothing changes. The reference now
  names that selector, `{"source": "journal", "sequence": <seq>}`.
- *You compared `payloadRef` values to tell whether two outputs are equal, or hashed an output to check it against
  `payloadRef`*: read the values through `POST /runs/{runId}/payload-disclosures` (with the audiences and the audit of
  feature 016) and compare those. An output you may not disclose is one the platform no longer lets you compare.
- *You compared `inputDigest` across schedule revisions to learn whether the input changed*: a change of input is a new
  revision. Read the input through `input-disclosures` if you may. The SDKs drop the field and do not leave it `null`,
  because `null == null` would silently answer "unchanged".
- *You keep a client of an older release*: it still parses both answers. An older Studio shows the reference where it
  showed a short digest. An older SDK's typed `inputDigest` is empty (`null`, or `""` in Go). **Code that compares it
  between two reads now always finds the two equal**, whatever the input: upgrade the SDK before you rely on such a
  comparison, or remove it. Against an older server, a new SDK and the new
  Studio report `hasInput: true` when the server still sends `inputDigest`, without keeping the value.

Inside the server nothing moves: the digest is still the content address of stored payloads, and no migration runs.

## Workers: leases no host received are withdrawn (feature 006, ADR-0098)

- A lease the engine committed and never handed to a host goes back to `READY` with its attempt unspent, recorded as the new
  journal event `step.lease_withdrawn`, once the operator records the requirement `lease-withdrawal-v1` with the new
  generic command `control-plane capability-activate lease-withdrawal-v1 --operator NAME --expect-live N` (owner role).
  Until then nothing changes. The requirement cannot be removed once a journal holds the event; `SAGAT_LEASE_WITHDRAWAL=off`
  on an engine stops new withdrawals. `docs/runbooks/lease-withdrawal.md`.
- Migration `20270208000200_lease_withdrawal_floor` builds a partial index on `run_events` under a SHARE lock (journal
  writes wait for it); on a large journal build it `CONCURRENTLY` first (the recipe is in the runbook, "Before upgrading").
- The six SDKs hand a lease they will not run back with `TASK_NOT_STARTED` only on a session of worker protocol 1.8 or
  later; below it, as before, the lease lapses. A handler that raises `TASK_NOT_STARTED` is reported as `WORKER_ERROR`.
- A release that carries this without its follow-up (the requests of ADR-0099, decision D006-090) cannot be the build a
  protocol 1.8 build rolls back to (gap G006-032).

## Breaking: every run input is checked against its version's schema (design 043)

From 0.3.0 the input of every run is checked against the `inputSchema` of the version the run pins, before the run is
created (ADR-0090, OD-2026-11-14-2). An input the schema refuses is `422 VALIDATION_FAILED` with `details.reason`
`RUN_INPUT_INVALID`, `details.field` and `details.rule`, where it used to be accepted. The six SDKs expose `field` and
`rule` beside `reason`, and the CLI and the Studio show them.

- An `inputSchema` must stay within a documented subset of JSON Schema. A schema outside it refuses every start of its
  version (`RUN_INPUT_SCHEMA_UNUSABLE`), and publication refuses a new one in `enforce` mode. Run
  `control-plane input-schema-inventory` before the upgrade to list the versions at risk; a superseded version that
  nothing starts any more is only counted, and `--all` lists it too. A document whose schema cannot be pre-checked now
  (the validator's four permits busy for 5 s) is answered a retryable `503`.
- **Runs in progress can fail** at the step that creates a child (`CHILD_INPUT_INVALID`) or continues
  (`CONTINUATION_INPUT_INVALID`) when that new run's input does not match its version's schema (OD-2026-11-14-9, an
  accepted deviation from constitution principle IV).
- Triggers on a version whose schema describes a manual input rather than the event envelope `{event, data}` stop
  starting runs: each refused event is a delivery without a run and an audit record `run.input_refused`.
- Schedule actions whose input the target version refuses are recorded `INPUT_INVALID`, are not retried, and pause the
  schedule when `pauseOnFailure` is set; trigger-now answers `422`.
- `SAGAT_RUN_INPUT_VALIDATION=report` admits refused inputs while callers are fixed; set it in every manifest **before
  migrating**, then go back to `enforce` and record the requirement `run-input-validation-v1`. The
  [runbook](../runbooks/run-input-validation.md) has the steps.

## Breaking: uncertain external effects wait for a person (feature 037)

New decisions follow Constitution 2.0.0, principle III. Journals written by v0.1 and v0.2 still
replay unchanged, and in-flight runs continue after an in-place upgrade. Only what the engine
decides from now on changes:

- An attempt whose outcome is unknown (lost lease, `ATTEMPT_TIMEOUT`, `OUTCOME_UNKNOWN`) waits for
  review in **any** activity. `highRisk` no longer changes this behavior.
- A lookup that answers `not_applied` no longer retries the original. The step waits with
  `RECONCILIATION_NOT_FINAL` until a person settles it.
- `RETRY_STEP` on a step whose outcome is unknown returns 409 with `details.reason`
  `OUTCOME_UNRESOLVED` for every role, administrators included. Use `RESOLVE_STEP` after checking
  with the provider.
- A compensation is planned only for an effect that succeeded. Defensive compensation of an
  unknown effect is gone; provider qualification for it is future work (US04).
- Cancellation and deadlines keep a pending review instead of giving it up.
  `COMPENSATION_FAILED` waits for independent compensations to settle.
- A successful scoped report whose output cannot be stored is kept as `STORAGE_QUOTA_EXCEEDED`
  instead of `RUN_DATA_LIMIT`.

Workers that are stopped before their in-flight activities end now create review items. Drain
workers before a restart ([scaling runbook](../runbooks/scaling.md#draining-workers-before-a-restart)).

## Schedules: the business-id prefix of an action (feature 018)

An action of a schedule may name a business-id prefix with a conflict and a reuse policy; each run it starts is bound to `<prefix>-<nominal time>`, and a
held id is the recorded decision of the time (`BUSINESS_ID_CONFLICT`, or `ALREADY_ACTED` naming the holder). A batch selector by `businessIdPrefix` selects runs.
Two things change for an operator: **upgrade the execution engines before the first schedule with a prefix is written** (an older engine fires it with no
business id and says nothing: [schedules runbook](../runbooks/schedules.md#11-the-business-id-of-the-runs-of-a-schedule-fr-018-016)), and **a policy with no prefix is now
a `422 SCHEDULE_SPEC_INVALID`** where it was accepted and dropped. Schedules that waited under a `remainingActions` limit for a manual action and were left with no next
time by an older build are not repaired by the upgrade: pause and resume them (G2092).

## Audit: business-id conflicts (feature 018)

A start refused `409` because its business id is held (`BUSINESS_ID_IN_USE`, `BUSINESS_ID_REUSE_REJECTED`), and the decision of a schedule
that finds its id held, now write the audit action `business_id.conflict` with that closed reason and never the id
([business-ids runbook](../runbooks/business-ids.md#5-reading-the-refusals)). A consumer of the audit trail that lists the actions it knows
meets one more.

## Batch operations: the effect on a run and the record of its item are one transaction (feature 018)

Each item of a batch job now records its outcome in the run's own transaction, after the effect (O1951, OD-2026-11-13-2): a crash
leaves the effect and the record both or neither, and a cancel of the job or a lease lost after the engine looked rolls the
effect back instead of letting one more through ([batch runbook](../runbooks/batch-operations.md#1-what-a-job-is-and-what-it-promises)).
An engine of this version still records an effect that an older engine applied without its record. Upgrade every engine before a
cancel of a job is relied on to stop an item in flight: an older engine keeps the two transactions.

## Batch operations: a selection that keeps failing ends the job (feature 018)

A batch job whose selection round fails (a statement over its ten-second timeout on a selector that reaches most of a namespace, a deadlock, a
persistent database error) used to be tried again at every poll, for ever, with nothing to tell the person who made it. The failure is now counted
in the job (`batch_jobs.selection_failures`, migration `20270116000000_batch_selection_failures`): the job waits 5 s, doubling to 5 min, between rounds,
survives ten failed rounds in a row, and **the eleventh ends it `FAILED` with `failureReason: SELECTION_FAILED`**, about 25 minutes after the first
failure. A round that picks resets the count. Nothing changes in the API (the reason was in the contract), and the read of a job gains no field. For an
operator: **apply the migration before the engines that count** (an older engine keeps retrying as before; during a rolling deploy a job that is still selecting can
fail up to ten rounds early, because an older engine does not reset the count), watch `batch_selection_failures_total{outcome}` and the new alert
`SagatBatchSelectionFailed`, which fires when a job ended this way and is not silenced by other jobs running (a burst of them means the database, not the selectors;
[batch runbook](../runbooks/batch-operations.md#51-a-selection-that-keeps-failing-selection_failed-g2093)), and know that the `down` of the migration refuses only while a job
that is still selecting has failed rounds on record. A cancel now brings the slot of the job back to the database's clock (a service whose clock ran ahead made a cancel wait),
and the error of a selection round is logged as its SQLSTATE and not as text.

## Batch operations: finished jobs and their items are purged after their retention (feature 018)

Until now nothing removed a batch job or its items: the plan, the migration and the runbook said they were kept for 30 days (at most 90), `expiresAt` was
stamped on every job and nothing acted on it, so `batch_jobs` and `batch_items` only grew (G2317). The migration `20270119000000_batch_purge` adds one bounded
database function, `batch_jobs_purge`, which the housekeeping of every engine now calls: a job that has **finished** (`COMPLETED`, `CANCELLED`, `FAILED`), whose
`expiresAt` has passed and which ended **at least 14 days ago** goes with its items; a job that has not finished, or is inside its window, never does. The service
now stamps a window of at least 14 days (it was one day, and nothing used it; the default stays 30 and the ceiling 90). A purged job answers `404` like an id that never
existed. `expiresAt` means "not removed before", not "removed at": a job that ended after its own window is kept for 14 days after it ended, so it outlives its window and, for a
90-day job that ends late, the 90 days (an owner question, O1958, is pending). **What a purge does not remove is the payload of a `SIGNAL` or `UPDATE` job**: it stays in `payloads`
and stays charged to the tenant, until feature 020's registry collects it (G2325); the job's own selector and reason are given back. There is no new route, setting, SDK surface or OpenAPI change. For an operator: **apply the migration before the engines that purge** (an engine with the
purge against an older schema warns `could not purge batch jobs` with `sqlstate=42883` and counts `batch_purge_failures_total`, and nothing else is affected);
**the first housekeeping pass after the upgrade removes the whole backlog** of jobs that ended more than their window ago (up to about 100,000 rows a pass, in calls
of at most 5,000 rows), so expect `batch_purged_total{kind}` to climb and `batch_items` to shrink over several passes (a pass calls the function again while a call used its bounds, up to 100 calls or 20 seconds);
the new alert `SagatBatchPurgeFailing` fires when a call failed in the last hour, and a failure at every pass means the job with the oldest window cannot be removed and **blocks the purge of every other
tenant's jobs** until it is fixed (runbook section 6.1, G2323); the index the function reads takes a short `SHARE`
lock on `batch_jobs` while it is built (build it `CONCURRENTLY` first on a large table: the migration finds it, the runbook has the recipe); the rollback drops the function and the index and loses no job
([batch runbook](../runbooks/batch-operations.md#6-how-long-a-job-is-kept-and-how-it-goes)).

## Breaking: event detail values require an explicit reveal

`getInboxEvent` omits `data` by default. `getConnectorDeadLetter` omits `body` and `headers` by
default. Their structural metadata remains available to authorized event readers.
`listConnectorDeadLetters` never returns body or header values.

Detail values require `reveal=pii`, `event:read`, current `payload:read-internal`, and a payload
reveal grant in that namespace. Admin also needs an explicit grant. Every successful reveal is
audited before values are returned; responses are private and non-cacheable. Clients must handle
the absent value fields and must not treat a saved grant as a broader role permission.

See [the events runbook](../runbooks/events.md) and
[payload classification](../runbooks/payload-classification.md).

## Classified workflow values

New workflow versions pin classification interpretation rules 1. Existing versions remain at
rules 0 and disclose no values through the new endpoint. Values use the pinned carrier's schema;
`secret` is never disclosed. Workflow input, workflow result and eligible journal outputs use the
explicit payload disclosure operation; existing structural inspection retains its purpose.

## Combined worker protocol

The integrated protocol is 1.6: scoped identity was introduced in 1.5 and managed database
capabilities in 1.6. Ordinary workers retain the documented feature gates. Do not reuse the
standalone unreleased 017 migration numbering against an existing integration database; preserve
backups and follow ADR-0036's checksum refusal policy.

## Step logs: capture on by default, and its measured cost

Capture of step logs is on by default at `info`: from this release the lines a step writes to its logger are stored
by the platform (design 041), unless a namespace or the deployment turns it off. The figures below hold only if task
T041-9, measuring the built carry, meets SC-041-007's per-size criterion (decision D041-126); otherwise this section
changes before the release. On a saturated
host with 1 API replica and 16 workers it costs about 18% of the step rate (82.0% of the rate with capture off) and
8.6 ms of completion p99 over the allowance; with 8 replicas and 128 workers every criterion is met (93.1% of the rate).
The owner accepted this measured cost (OD-2026-11-20-13). A namespace can turn capture off or lower its ceilings. Lines
can be dropped, visibly, when the log service is busy ("not stored: the log service was busy (not your quota)"); the
report always lands (OD-2026-11-20-14). The usage of the tenant's step-log quota is shown only to readers holding the
PII reveal grant in every namespace (OD-2026-11-20-19). See [the step-logs runbook](../runbooks/step-logs.md#1-what-capture-costs).
