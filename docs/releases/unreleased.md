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

## Schedules: the business-id prefix of an action (feature 018)

An action of a schedule may name a business-id prefix with a conflict and a reuse policy; each run it starts is bound to `<prefix>-<nominal time>`, and a
held id is the recorded decision of the time (`BUSINESS_ID_CONFLICT`, or `ALREADY_ACTED` naming the holder). A batch selector by `businessIdPrefix` selects runs.
Two things change for an operator: **upgrade the execution engines before the first schedule with a prefix is written** (an older engine fires it with no
business id and says nothing: [schedules runbook](../runbooks/schedules.md#11-the-business-id-of-the-runs-of-a-schedule-fr-018-016)), and **a policy with no prefix is now
a `422 SCHEDULE_SPEC_INVALID`** where it was accepted and dropped. Schedules that waited under a `remainingActions` limit for a manual action and were left with no next
time by an older build are not repaired by the upgrade: pause and resume them (G2092).

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
