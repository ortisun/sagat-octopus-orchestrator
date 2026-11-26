# Security check of the quota fix (2026-11-26, commit 8a693ca02)

Recorded by the implementer from the coordinator's relay of the security reviewer's check.

Verdict: the quota fix (decision D022-133) is fixed: `used` of `concurrency_live_keys`, `rate_limit_live_keys` and
`flow_control_key_waiters` is withheld (`usageWithheld: true`) from a caller who may not reveal personal data in the
namespace, and shown with the reveal grant. Three low items remain, each to be fixed (a security finding is never
accepted):

1. (low) The alerts' `items` count keys per group to anyone with `ops:read`, Viewer included, without the reveal right:
   `crates/infra-postgres/src/operations.rs:196` and `:219` (`count(*) AS items`), served by
   `crates/api-http/src/dto_ops.rs:30` (`crates/application/src/operations.rs:663`). With `queued_runs` set so that a key's
   waiter cap is 1, every waiting run-scope rate key meets `RATE_LIMIT_QUEUE_SATURATED`, and a Viewer watching `items`
   for a group sees it drop by one when a key's last waiter is served. Fix: report `items` of every flow-control alert
   kind only to a caller who may reveal personal data in that namespace, the grant read in the same snapshot; a test
   seen failing first; D022-133 corrected (it said no other surface shows these counts).
2. (low) No test pins the one snapshot of `usage_if_revealed_in_namespace`
   (`crates/infra-postgres/src/reveal_grants.rs:165-185`): make it one `SELECT` that reads the role and the grant and
   returns the usage only when the grant exists, as `step_log_usage_if_revealed` does, so that the single snapshot
   follows from the statement.
3. (low) Contract drift: `specs/001-platform-foundation/contracts/openapi.yaml:6809-6810`, the comment of
   `crates/api-http/src/dto_gov.rs:21` and the Studio's generated `frontend/apps/studio/src/api/schema.d.ts:5118` still
   say `usageWithheld` applies only to `tenant_step_log_bytes`; the description changes of the earlier slices (the
   listings' cursor lead and group view, `spansPages` deprecated, the view enum's five groups, the group schemas'
   "summed whole") are not in the canonical contract either.

Also checked: the per-key waiter ceiling refusal (`CONCURRENCY_QUEUE_FULL`, `RATE_LIMIT_QUEUE_FULL`) counts others'
waiters only on a key value the caller uses itself, which `getRun`'s `position` already shows it: per-key information
inherent to sharing a key. Recorded in gap G022-040 so that the owner decides it knowingly.
