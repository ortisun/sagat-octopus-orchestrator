# Unreleased integration changes

These changes are on the project completion feature branch. This is not a published release.
Final compatibility, fault, canary, performance and workspace gates remain in the feature tasks.

## Flow control: concurrency keys and rate limits (feature 022, release 0.3.0 slice)

A workflow can now say how much of its work may run at once, and how often it may start, per business key, with the
excess waiting durably in arrival order instead of failing (user stories US02 and US07 of
[feature 022](../../specs/022-flow-control-step-policies/spec.md); ADR-0056, ADR-0088, ADR-0089, ADR-0093, ADR-0094,
ADR-0096). Off by default; nothing changes for a definition that does not declare it.

**For authors.**

- `concurrency: {limit, key?, group?, queueTimeoutMs?}` at the top level of a definition (at most `limit` running runs
  per key) or on a root `activity` or `script` node (at most `limit` attempts per key at once).
- `rateLimit: {limit, per, key?, group?, queueTimeoutMs?}`, `per` one of `second`, `minute`, `hour`, at the same two
  places: at most `limit` run starts, or leased attempts, per key in any interval of `per` on the database clock. There
  is no burst: the first `limit` start at once, the next as the oldest grants age out.
- Work beyond a gate waits `WAITING` (wait kind `CONCURRENCY` or `RATE_LIMIT`), holds no worker and spends no attempt; it
  is granted oldest first, and fails `CONCURRENCY_WAIT_TIMEOUT` or `RATE_LIMIT_WAIT_TIMEOUT` at its queue deadline (10
  minutes by default, one day at most). A step that waits is never `READY`, so it never fills a worker's poll. A run that
  declares both gates holds both or neither.
- A synchronous start beyond a full per-key queue, the namespace's `queued_runs` or its live keys is refused before any
  run exists: `429 QUOTA_EXCEEDED` with `CONCURRENCY_QUEUE_FULL`, `RATE_LIMIT_QUEUE_FULL` or `FLOW_CONTROL_LIMIT`; an event
  or a cron start stays pending under its retry. A reset takes a fresh permit and slot or is refused
  (`CONCURRENCY_PERMIT_UNAVAILABLE`, `RATE_LIMIT_SLOT_UNAVAILABLE`) and never waits.
- Publication refuses an invalid policy, either field inside a body, and either field with `continueAsNew`; definitions
  that share a group with different limits are warned (`CONCURRENCY_LIMIT_MISMATCH`, `RATE_LIMIT_MISMATCH`).
- Not in this build: the builders of the six SDKs, the Studio's fields and views, and the shared conformance scenarios
  (tasks T022-048 to T022-056, T022-067, T022-083 to T022-085): declare the fields in the workflow's JSON document.

**For operators.** Read the [flow-control runbook](../runbooks/flow-control.md) before you turn either on.

1. Give every control plane and every engine the same digest key ring, a file named by
   `SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE` (`version:hex`, at most 16 KiB; Helm `flowControl.digestKeys.existingSecret`,
   compose overlay `compose.flow-control.yaml`). A process with a switch on and no file, or with a file that lacks a
   version the database has in force, does not start. The images' group is now gid 10001.
2. Turn on `SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED`, `SAGAT_FLOW_CONTROL_RATE_LIMITS_ENABLED`, or both, and restart.
3. Activate the digest key version (`SELECT * FROM flow_control_key_activate(1::smallint, '<your name>');`), then each
   capability for the fleet: `control-plane flow-control-activate --capability concurrency-keys --operator <your name>`
   and `--capability rate-limits`, as the schema-owning role. Until then publication and starts that use one are
   refused `422 VALIDATION_FAILED`, `FLOW_CONTROL_ACTIVATION_PENDING`; afterwards a process without the capability
   answers `503`, `BINARY_CAPABILITY_MISSING`.
4. To turn one off, switch it off and keep the key file mounted: a switched-off process that holds the file still lists
   the gate and drains what was admitted (D022-062).

New reads: `Run.concurrency`, `Run.rateLimit` (with `position` and `estimatedGrantAt` on a single-run read) and a step's
`waitingOn` for a reader of the run; `GET /namespaces/{namespaceId}/concurrency-keys` and `GET .../rate-limit-keys` with
`ops:read`. A key is
shown only as a label, and only to a caller with the namespace's reveal grant; other readers get `null` and the key's
instants cut to the minute (D022-060, D022-061). New series `concurrency_acquisitions_total` and
`rate_limit_decisions_total` (by `scope` and `outcome`); the wait histograms, waiter gauges, grant lag and unleased grants,
and the three alerts `CONCURRENCY_QUEUE_STUCK`, `RATE_LIMIT_GRANT_LAGGING` and `RATE_LIMIT_QUEUE_SATURATED`, are
documented in the runbook and built by task T022-082.

New namespace quota resources, each a namespace's value, else the tenant's default, else the deployment's (default /
hard ceiling): `queued_runs` (1,000 / 10,000), `flow_control_key_waiters` (100 / 10,000, always below `queued_runs`),
`concurrency_live_keys` and `rate_limit_live_keys` (10,000 / 100,000 each), `concurrency_limit` (10,000 at most),
`flow_control_queue_timeout_ms` (600,000 / 86,400,000), `flow_control_node_declarations` (16 / 64 node declarations of
one kind), `rate_limit_per_second` (20 / 20), `rate_limit_per_minute` (6,000 / 6,000), `rate_limit_per_hour`
(10,000 / 10,000). A declared per-second limit is held to 20 until the benchmark of SC-022-029 (three engines, one key
at 100 a second) is published; a key's canonical value is at most 256 bytes. The live-keys ceilings can be passed by the
keys that concurrent transactions create at the same moment (accepted by the owner, OD-2026-11-20-3 and
OD-2026-11-20-11; runbook section 4). Finish the rolling upgrade before setting any `rate_limit_*` limit; rolling back
migrations `20270203000000` and `20270206000000` is refused while runs or limits use them (runbook sections 5 and 6).

**Release gates: do not activate in production before they are closed.** Concurrency keys: T022-104 (publication's
refusal of a key over a `secret` field, gap G022-015; built, `FLOW_CONTROL_KEY_SECRET` and `FLOW_CONTROL_KEY_PII`, awaiting
its review) and T022-111 (the group rows of the key listings); this release does not close with them open. Rate limits: also
T022-080 (the fault suite's review), T022-113 (the rate twins of the run creators) and gap G022-018 (the purge's order
and backlog), as the runbook's section 6 lists them.

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
