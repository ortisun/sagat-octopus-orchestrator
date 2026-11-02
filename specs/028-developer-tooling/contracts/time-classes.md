# Time-class catalog (draft, feature 028; coordinator decision C13)

The one place that says which clock each instant of the platform is on (FR-028-003, ADR-0063). At implementation it
becomes the machine-readable `crates/infra-postgres/time-classification.json`, checked against `pg_catalog` (columns,
column defaults and `pg_proc` bodies). Every draft cites this catalog instead of restating the rule; a feature that
adds a time column adds its row here in the same delivery.

## Classes and rules

| Class | Meaning | "Now" it is compared with |
|---|---|---|
| `workflow_due` | an instant compared with now to decide that something a workflow observes is due | `sagat_workflow_from(now())` for selection, the post-lock workflow instant for decisions |
| `workflow_record` | an instant of a run's history (journal, run and step lifecycle, decisions) | `sagat_workflow_from(...)` when measured (lag, retention age) |
| `infrastructure` | liveness, credentials, windows, rate, jobs, audit | `now()` / `clock_timestamp()` |

1. Two stored instants of different classes are never compared; `workflow_due` and `workflow_record` are one family.
2. A comparison or difference with now uses the now of the instant's class: lag metrics and retention ages over a
   workflow-class column use workflow now (in production the same value).
3. Workflow time is derived from one infrastructure sample (`sagat_workflow_from(ts)`); two volatile clock calls are
   never used for one decision.
4. An instant that decides what a workflow observes (due, expired, timed out, cache hit) is workflow time; an instant
   that bounds a credential, a lease, a session, a window, a rate or a platform job is infrastructure time.
5. A value that is both a workflow deadline and a credential's validity is stored twice, once per class, and the
   credential is accepted only while both hold.

## Baseline schema (examples; the manifest lists every column)

| Instant | Class |
|---|---|
| `steps.due_at`, `steps.next_attempt_at`, `steps.attempt_deadline_at`, `runs.deadline_at`, `workflow_triggers.next_fire_at`, `workflow_triggers.retry_at`, `approval_timers.due_at`, `loop_timers.due_at`, `loop_scopes.deadline_at`, `loop_scopes.delay_due_at`, `execution_chains.original_deadline_at` | `workflow_due` |
| `run_events.occurred_at`, `runs.finished_at` and the other run and step lifecycle instants, approval decision instants | `workflow_record` |
| `steps.lease_expires_at`, `steps.event_since` (compared with `inbox_events.received_at`), `worker_sessions.expires_at`, `scheduler_shards.expires_at`, `inbox_events.received_at`/`expires_at`, `telemetry_pending.behind_since`, `script_test_runs.finished_at`, cleanup and receipt windows, `audit_records.created_at` | `infrastructure` |
| Scheduler lag (`operations.rs`): timers, waits, retries, run deadlines over workflow columns | workflow now; lease lag stays infrastructure |
| Telemetry purge over `runs.finished_at` (`telemetry.rs`) | workflow now |

## Instants added by the parity drafts (as named in their data models on 2026-10-27)

| Feature | Instant | Class | Note |
|---|---|---|---|
| 018 | schedule `next_action_at`, `next_actual_at`, `retry_at`, `next_wake_at`; action `nominal_at`; `runs.start_delayed_until`; `runs.schedule_nominal_at` | `workflow_due` / `workflow_record` | the schedule pass bounds its scan with `sagat_workflow_from(now())`; backfill ranges are workflow time |
| 018 | batch-operation selection bounds and cursors | `infrastructure` | operator operations over runs |
| 019 | `detached_deadline` (asynchronous completion timeout) | `workflow_due` | an attempt deadline |
| 019 | `detached_at` | `workflow_record` | |
| 019 | task-token validity, registration and push-concurrency leases, `breaker_open_until`, heartbeat and poll instants | `infrastructure` | credentials and liveness |
| 020 | retention age measured from `runs.finished_at` | workflow now | a time-skipping test can exercise retention (G1420) |
| 020 | sink cursors, leases, `oldest_pending_at`, `acked_at`, `purged_at`, `tombstone_at` | `infrastructure` | |
| 021 | `previous_valid_until` (credential overlap) | `infrastructure` | |
| 022 | `concurrency_waiters.deadline_at` | `workflow_due` | queued admission timeout |
| 022 | `concurrency_waiters.enqueued_at` | `workflow_record` | ordering only |
| 022 | step result cache `expires_at` (TTL) | `workflow_due` | a hit or miss is observed by the workflow; purge of expired entries uses workflow now |
| 022 | cache-fill and other job leases | `infrastructure` | |
| 024 | data-snapshot `retain_until`, freshness observations | `infrastructure` | unless a freshness check is a step of a workflow, which then reads workflow time |
| 025 | agent step wall-time budget (`agent_steps.deadline`) | `workflow_due` | like a step deadline |
| 025 | daily usage windows, MCP token and task-handle `expires_at`, call receipts | `infrastructure` | quotas and credentials |
| 026 | public link `expires_at`, decision-link token validity, link codes | `infrastructure` | credentials |
| 026 | approval deadline (014) | `workflow_due` | a decision link carries its own infrastructure expiry; rule 5 |
| 026 | stream retention | `infrastructure` | |
| 027 | app session and code `expires_at` | `infrastructure` | |
| 030 | operation `deadline_at`, `start_deadline_at` | `workflow_due` | the caller's effective deadline |
| 030 | operation-message `next_attempt_at` | `infrastructure` | delivery retry |
| 031 | codec-assertion lifetimes, job leases | `infrastructure` | |
| 032 | region and cell epochs | not instants | counters; `activated_at` and region leases are `infrastructure` |

A draft whose instant is not listed classifies it with rule 4 and adds its row; a disagreement is resolved in this
catalog, not in the draft.
