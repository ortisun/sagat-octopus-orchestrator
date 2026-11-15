# Unreleased integration changes

These changes are on the project completion feature branch. This is not a published release.
Final compatibility, fault, canary, performance and workspace gates remain in the feature tasks.

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

## Schedules are on by default (feature 018, OD-2026-11-13-8)

`SAGAT_SCHEDULES_ENABLED` now defaults to `true` in the control plane and the engine (and in the development stack and the Helm chart, value
`schedules.enabled`): a deployment that sets nothing serves the schedule routes and runs the schedule pass. Upgrade in this order: migrate, then
the execution engines, then the control planes (OD-2026-11-14-3). **An explicit `SAGAT_SCHEDULES_ENABLED=false` kept from 0.2 keeps schedules off**,
and after the handover of the cron triggers such an engine is refused at registration: remove the line, or keep it on purpose
([schedules runbook](../runbooks/schedules.md#2-turning-it-on-and-off)). Batch operations and the two reset switches stay off by default.

## The cron triggers are handed over automatically after a deployment (feature 018, ADR-0091, D2248)

**The handover of the cron triggers from the legacy cron pass to the schedule pass is automatic.** A one-shot job of the deployment (the compose
service `schedule-trigger-handover`, the Helm Job `<release>-schedule-trigger-handover-<revision>`) runs `control-plane schedule-trigger-handover
--auto` as the narrow role `sagat_handover` and hands over once the database finds, in three rounds 15 s apart and no sooner than ten minutes after
the job began, that every engine seen in the last ten minutes lists `schedules-v1`, the live engines hold still (at least `--min-live`), no engine
below 0.3.0 is seen in the scheduler memberships, the reconciliation made no trigger schedule in the last ten minutes, a control plane is live and no
cron trigger lacks its schedule ([schedules runbook](../runbooks/schedules.md#62-the-handover-step-by-step)).

- **How to see that it happened:** `trigger_schedule_handover_mode{mode="AUTOMATIC"}` is 1 in every engine; the log line `the cron triggers were
  handed over to the schedule pass` (mode, epoch, engines, the count of tenants); `SELECT state, changed_mode, changed_at FROM
  schedule_trigger_handover`; every tenant that had a cron trigger has the audit record `schedule.trigger_handover`. While it waits,
  `trigger_schedule_handover_blocked{reason}` says why ([6.3](../runbooks/schedules.md#63-why-a-handover-is-blocked)).
- **It is one-way:** there is no command back, the schema `down` refuses once it happened, and no engine below 0.3.0 may run after it (it would
  ignore a pause).
- **What to do:** give `migrate` `SAGAT_HANDOVER_PASSWORD` (it creates and grants `sagat_handover` at every run and sets its login from a SCRAM
  verifier while the state is `LEGACY`), or create the login once; in Helm put `DATABASE_URL` (without the password) and `SAGAT_HANDOVER_PASSWORD`
  in the Secret `schedules.autoHandover.databaseSecret`. After the handover retire the role (`--retire-role`, `schedules.autoHandover.retireRole`,
  `SAGAT_HANDOVER_RETIRE_ROLE`, or `ALTER ROLE sagat_handover NOLOGIN PASSWORD NULL`), **then switch the job off
  (`schedules.autoHandover.enabled: false`, `SAGAT_SCHEDULES_AUTO_HANDOVER=false`)**, and only then delete the Secret: a later release would
  otherwise run a job that cannot log in. Give `migrate` `SAGAT_HANDOVER_MIN_LIVE` (the engine's replicas or the autoscaler's minimum), the
  floor the job counts live engines with.
- **`helm upgrade` with the values of 0.2 fails at render** until `schedules.autoHandover.databaseSecret` names the Secret or the job is
  switched off: the job is on by default and the chart requires its Secret (fail-closed; architecture review of 2026-11-15, m6). A role
  that may create in a schema is refused its grants and its login: on a database first made before PostgreSQL 15 run
  `REVOKE CREATE ON SCHEMA public FROM PUBLIC` first (security review of 2026-11-15). The schema `down` of the release leaves the role's
  login to you (`ALTER ROLE sagat_handover NOLOGIN PASSWORD NULL`, D2306).
- **To keep it manual:** `schedules.autoHandover.enabled: false` (or `schedules.autoHandover: false`) in Helm, `SAGAT_SCHEDULES_AUTO_HANDOVER=false`
  in compose; then hand over with `--apply` as before.
- **What is accepted:** an engine binary below the registry release that never registers keeps its legacy pass and ignores a pause (G2005); a
  control plane below 0.3.0 that publishes after the handover loses that trigger's first ticks until the reconciliation plans it (G2020); a backfill
  over ticks the legacy pass started more than seven days ago starts a second run (G2021). **An engine with `SAGAT_SCHEDULES_ENABLED=false` holds the
  handover** (it lists no `schedules-v1`).

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
