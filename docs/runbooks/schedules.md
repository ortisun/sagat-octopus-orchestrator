# Runbook: schedules (feature 018)

For the people who run the platform and for operators who answer "why did, or did not, this schedule start a run". It says how a schedule is turned on, what the engine does with it, what to look at when it misbehaves, and what is not built yet.

Design: [ADR-0050](../adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md); decisions, gaps and owner questions: [`specs/018-schedules-bulk-operations/research.md`](../../specs/018-schedules-bulk-operations/research.md). The batch half of the feature is in [`batch-operations.md`](batch-operations.md).

## 1. What a schedule is

A schedule starts a run of a published workflow at the times its rule gives. The rule (the *spec*) is an interval, cron expressions or calendars, in an IANA time zone, with an optional start, end and jitter. The platform, not the browser and not a worker, decides every time, on the **database clock**.

- **Who it acts for.** Every run is started as `system:schedule:<id>` with the role of the person who last wrote the schedule (`authorizedBy`), and that person's permission to start runs is checked again, in the transaction that admits the run, every time. A schedule whose author lost `run:start` stops starting runs: the time is decided `AUTHORITY_REVOKED` and the schedule pauses itself with a system note (`schedule.auto_paused`).
- **What a time can become.** Every nominal time of a schedule has exactly one decision, whether or not it started a run: `STARTED`, `BUFFERED` (waiting for a run to end), `SKIPPED_OVERLAP`, `DROPPED_BUFFER_FULL`, `DROPPED_DELETED`, `COALESCED` (several late times merged into one summary), `MISSED_CATCHUP_WINDOW`, `ALREADY_ACTED` (a backfill found it done), `AUTHORITY_REVOKED`, `BUSINESS_ID_CONFLICT`. They are listed newest first at `GET /namespaces/{id}/schedules/{id}/actions` and on the schedule's page in the Studio.
- **When a run is still open** (`overlap`): `SKIP`, `BUFFER_ONE`, `BUFFER_ALL`, `CANCEL_OTHER`, `ALLOW_ALL`. `CANCEL_OTHER` cancels the open run through the ordinary cancel, so only somebody who may cancel runs (`run:operate`: admin, operator) can write it.
- **After an outage** (`catchupMode`, `catchupWindowMs`, 366 days by default): `ONE` starts only the latest missed time, `ALL` starts every one inside the window, and the rest are summarized (`COALESCED`, `MISSED_CATCHUP_WINDOW`). A recovery is planned once, remembered on the schedule (`recovery_until`, `recovery_acts`) and carried out a step at a time, so a long outage is worked through over several rounds and never by one schedule holding the others back.
- **Pause, resume, run now, backfill.** Pausing acts on no time and catches none up afterwards. *Run now* decides one time at once, as the schedule would (`origin` `MANUAL`: it never shares an identity with a scheduled time, takes no remaining action, and works while the schedule is paused). A *backfill* acts on the times of a past range (at most 1,000), one backfill at a time per schedule; a time that already has a decision is never given a second run.
- **Permissions.** `schedule:read` (every human role); `schedule:write` with `run:start` to create, change or delete (admin, developer); `schedule:operate` to pause (admin, developer, operator); resuming, running now and backfilling need `schedule:operate` and `run:start`.

## 2. Turning it on

Off by default, in two places that must agree:

| Where | Setting | Default | What it does |
|---|---|---|---|
| control plane | `SAGAT_SCHEDULES_ENABLED` | `false` | serves the schedule routes (otherwise `404`) |
| engine | `SAGAT_SCHEDULES_ENABLED` | `false` | fires the schedules of the shards the replica holds |
| engine | `SAGAT_SCHEDULES_BATCH` | 100 (1 to 5,000) | due schedules looked at in a round |
| engine | `SAGAT_SCHEDULES_ROUND_ACTIONS` | 100 (1 to 1,000) | steps one round takes over all its schedules and backfills; the next round carries on |
| engine | `SAGAT_SCHEDULES_POLL_MS` | 1000 | pause when a round found nothing |

With the routes on and the engine off, schedules can be written and never fire: the lag gauge (section 4) grows. A round shares its `SAGAT_SCHEDULES_BATCH` between the tenants that have schedules due the same way as the passes of the runs ([`scaling.md`](scaling.md)), so one tenant's backlog does not hold back the others; `SAGAT_SCHEDULER_DISCOVERY_REST_MS` rests the search for other tenants.

Limits a namespace can change (`quota_ceilings`, `quota_limits`): the number of `schedules` (200 by default) and of `schedule_backfills` running at once (4). In the spec: at most 10 entries together (hard 32), a minimum spacing of 60 s (floor 10 s) over the next 1,000 times, jitter at most 24 hours, an input of at most 64 KiB.

## 3. When a schedule did not start a run

Open the schedule in the Studio (`/schedules`), or read its decisions and describe it over REST. In this order:

1. **Is it acting at all?** `state` `PAUSED` (look at the note and who wrote it), or *exhausted* (`remainingActions` reached zero), or past its `endAt`: there is no next time. A schedule that paused itself says why in its note (`the authorizing subject can no longer start runs`, or `a run of the schedule failed` when `pauseOnFailure` is set).
2. **Is there a decision for the time?** If yes, its outcome and reason say it: `SKIPPED_OVERLAP`/`BUFFERED` (a run it started is still open: `openRuns` on the schedule), `TRACKING_LIMIT` (it already tracks as many open runs as it may), `NO_PUBLISHED_VERSION` (the workflow has nothing to start; the time stays owed and is retried with a growing delay), `AUTHORITY_REVOKED`, `COALESCED`/`MISSED_CATCHUP_WINDOW` (it was too late: after an outage, or created with a start in the past).
3. **If there is no decision and the time has passed**, the engine did not get to it: is `SAGAT_SCHEDULES_ENABLED` on in the engine, does a replica hold the schedule's shard (`scheduler_shards_owned` sums to 64), is `scheduler_lag_seconds{pass="schedules"}` high? The decision is made in the database transaction that starts the run, so a replica that lost its lease is refused where it commits and the new owner does it: a time is never started twice.
4. **A run was started and then nothing happened to it**: that run is an ordinary run; see [`execution-engine.md`](execution-engine.md).

`schedule_actions_total{kind,outcome}` counts decisions (and `outcome="admission_retry"` the times the admission refused and left owed); `schedule_action_lag_seconds{kind}` is how long after its time a scheduled action started.

## 4. Watching it

| Signal | Meaning |
|---|---|
| `scheduler_lag_seconds{pass="schedules"}` | how long the oldest active schedule has been due; `SagatSchedulerLate` fires above 60 s for five minutes, for this pass as for the others ([`scaling.md`](scaling.md#scheduler-late)) |
| `schedule_actions_total{kind,outcome}` | decisions, counted once they are committed; a decision the fence refused counts nothing |
| `schedule_action_lag_seconds{kind}` | firing lag of the scheduled actions |
| `scheduler_fenced_total{kind="schedule"}` and `{kind="backfill"}` | steps another replica or a lost lease took away |
| `scheduler_fairness_limited_total{pass="schedules"}` | tenants given less than they showed, because another had a backlog |

No label is a tenant, a schedule or a run. The audit trail records `schedule.created`, `updated`, `deleted`, `paused`, `resumed`, `auto_paused`, `triggered`, `backfill_requested` and `backfill_cancelled` in the transaction of the change, never the input.

## 5. The input

The input of a schedule is charged content. An ordinary read **never returns it**: it gives its SHA-256 digest only, a change must send it whole, and the Studio asks for it again on every edit.

Its values come out through one route, `POST /namespaces/{namespaceId}/schedules/{scheduleId}/input-disclosures`, which needs `schedule:read` and applies the audiences of feature 016 to the input as `POST /runs/{runId}/payload-disclosures` does to a run's: `public` values to every reader, `internal` ones with `payload:read-internal`, `pii` ones only when the request says `{"reveal": ["pii"]}` and the caller has the namespace's reveal grant (otherwise `403` `PII_REVEAL_NOT_GRANTED`), and `secret` ones never. The classification is that of the input schema of the version the schedule would start now: the version it pins, or the workflow's active one. A workflow with no schema to classify by gives its structure to a caller with `run:payload:inspect` and nothing to the others. Every answer is `private, no-store`, and the read is on record before the values leave: `schedule.input_disclosed` for what an internal reader or an inspector got (the same read made again within five minutes is one record), and `payload.pii_revealed` for each reveal of personal data (never collapsed). A reader who sees only public values leaves no record. Look at the trail with `GET /audit?action=schedule.input_disclosed`.

## 6. What is not built yet

- A schedule kept by a **cron trigger** of a published workflow (US07) and the handover of triggers: no schedule has `origin` `TRIGGER` yet, and the filter for it returns none.
- **Business ids** and a **start delay** (US03), so the decision `BUSINESS_ID_CONFLICT` is never made.
- A **preview** of a backfill (G1966) and the validation of the input against the input schema of the workflow (G1955).
- Replay of a write by its `Idempotency-Key` (G1014): a write sent again after a lost answer is refused by what the first did (`409` for a name in use or a stale `expectedRevision`), and nothing is applied twice.
- The schedules, decisions and revisions of a deleted schedule are kept; their retention is G1954. Old decisions are purged by the maintenance pass, never one that waits.
