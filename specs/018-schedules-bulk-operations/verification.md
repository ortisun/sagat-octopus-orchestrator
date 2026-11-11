# Verification: schedules and bulk run operations (feature 018, T018-075)

What was run for T018-075, what it showed, and what each success criterion rests on. **Nothing here is a claim the author did
not run or read:** where a status rests on a test that someone else ran (the task notes, CI), it says so and names the test.
The statuses are strict: **VERIFIED** means the evidence proves the whole sentence of the criterion; **PARTLY** that it proves a
part, and says which part is missing; **BLOCKED** that what is missing needs a feature that is not approved or merged;
**NOT DONE** that nobody has built or tested it. `tasks.md` is not ticked by this document (T018-075 has a dated note).

The demo steps are those of [quickstart.md](quickstart.md), which was rewritten to say what happened; the outputs are there
(each step below links its section).

## (a) Environment

| What | Value |
| --- | --- |
| Commit | `98818160` (branch `feature/018-business-ids`, pull request #98), **plus the uncommitted changes of this task**: `apps/cli/src/{args,batch}.rs`, `apps/cli/tests/schedule_and_batch.rs`, `crates/application/src/schedules.rs`, `tests/integration/schedules-bulk-operations/firing.rs`, `docs/reference/cli.md`, `infra/docker/compose.yaml`, `specs/018-schedules-bulk-operations/{quickstart,verification,research,tasks}.md` and the runbooks named in section (g) |
| When | 2026-11-09, the stack was up from 23:22 to 00:00 UTC (its clock; the host is UTC-3); the gates of section (g) ran until 00:05 |
| Machine | Apple M3 Max, 16 cores, 48 GiB, macOS 26.3 (Darwin 25.3.0 arm64); load average 5 to 10 from other work |
| Docker | Docker Desktop, engine 29.9.2 linux/arm64, VM of 16 CPUs and 7.75 GiB; Compose 5.6.1. The daemon is shared with other projects whose containers were not touched |
| Compose project | `sagat018demo` (its own; volumes `sagat018demo_*`), the repository's `infra/docker/compose.yaml` with an override file that only renames the three built images to `sagat018-demo/{control-plane,execution-engine,dev-idp}:dev`, so that no `:dev` tag of another checkout moved. At the end: `docker compose -p sagat018demo --profile seed down -v --remove-orphans` (containers, network and the four volumes gone), the three images removed with `docker rmi` (no `prune`), the worker process stopped |
| Environment file | **Not `infra/docker/.env`**: the session was refused every read of files named `.env*` (the harness's permission rules), so `.env.example` could not be read, copied or edited. A throw-away file outside the repository with the variables the compose file requires (a random database password, the dev provider's issuer `http://dev-idp:9100/`, `SAGAT_ALLOW_INSECURE_JWKS_HTTP=true`) and the four lines of section 1 of the quickstart was used (step 1.1 SKIPPED) |
| Services | PostgreSQL `postgres:16-alpine`, NATS `nats:2.14.6-alpine`, Redis `redis:8.6.8-alpine`, the control plane, one execution engine, the development identity provider; every one on `127.0.0.1` |
| Versions (`sagat doctor`) | platform 0.2.0, `sagat` 0.2.0, worker protocol up to 1.6; Rust 1.98.0; `jq` 1.8.2 |
| CLI | `cargo build -p sagat-cli --bin sagat --locked` into `/Users/marcelo/cargo-target-018` (a debug build; `cargo install` was not run) |
| Shape | one control plane, one engine replica (a second one only for step 6.18), one tenant, one namespace, one worker (`sagat worker dev`), development credentials, no TLS: **a laptop, not a deployment** |

The images were built twice from this tree: first from `98818160` (the demo of sections 2 to 6, except the reruns), then again
after the two fixes of this task (only the control plane was restarted on the second build: section (b), "Rerun after the fixes").

## (b) The demo log

Results: **PASS** (the page was right), **DOC FIXED** (the page said something else; it now says the truth), **DEFECT FIXED** (the
product was wrong; fixed with a test first, decision in section (g)), **FAILED** (the product is wrong and not fixed here: a gap
is recorded), **SKIPPED** (not run, with the reason). Ids are `section.step`; "Q §n" links the quickstart.

| Step | What | Result | What happened |
| --- | --- | --- | --- |
| 1.1 | `cp infra/docker/.env.example infra/docker/.env` | SKIPPED | the session may not read `.env*` files (see (a)); a throw-away env file stood in |
| 1.2 | turn the feature on | DOC FIXED | the page did not say how. The routes and passes are off by default (right); `compose.yaml` now passes `SAGAT_SCHEDULES_ENABLED`, `SAGAT_BATCHES_ENABLED`, `SAGAT_RESET_CANCEL_SOURCE_ENABLED` (both services) and `SAGAT_RESET_REAPPLY_ENABLED` (control plane), each `false` unless the environment says `true`; the page lists the four lines to append to `.env`. **The lines are not in `.env.example`** (unreadable here): a maintainer adds them (G2061). The engine's log confirms the effect: `the optional passes of this replica: ["schedules", "batches", "resets"]` |
| 1.3 | `docker compose … up -d --build` | PASS | the images built (`build`, then `up -d`: the first release build is long on a loaded machine; the second, with the cache, was about 8 minutes); `migrate` exited 0; the control plane, the engine, the identity provider, PostgreSQL, NATS and Redis up |
| 1.4 | the seed | PASS | `development tenant created`, `development namespace created` (payments) |
| 1.5 | the CLI | PASS | built, not installed |
| 1.6 | `sagat dev token`, `sagat doctor` | PASS | nine `ok` lines and `nothing to fix` |
| 1.7 | `sagat new hello-world`, publish `hello-world`, publish `cart` | PASS | `published  hello-world v1`, `published  cart v1`; the cart document validates (Q §1) |
| 1.8 | `sagat worker dev …` | PASS | `serving 2 activities in payments (dev) at protocol 1.2`. It exits when the engine restarts (`the platform refused to hand out tasks: transport error`): the page now says so (G2060) |
| 1.9 | `export NS=…`, `export WF=…` | DOC FIXED | the publish command prints no workflow id (G2059); the page now reads them with `curl` and `jq`; a namespace call needs `X-Octopus-Namespace` |
| 2.1 | create `hello-every-minute` | PASS | `201`, `revision: 1`, `state: ACTIVE`, `nextActionAt: 2026-11-09T23:24:00Z` (Q §2) |
| 2.2 | `sagat schedule get hello-every-minute --recent 5` | DOC FIXED | the command takes the schedule's **id**, has no `--recent`, and prints the next five and the last five by itself; `schedule actions <id>` lists the decisions |
| 2.3 | the actions: `STARTED` with run ids, `startedBy` | PASS | three `SCHEDULED STARTED` rows with run ids; `startedBy: system:schedule:<id>` |
| 2.4 | each run has `schedule.nominalTime` (exact minute) and `actualTime` | **DEFECT FIXED** (2026-11-10, **not rerun on the stack**) | on 2026-11-09 the run resource had `startedBy` and nothing else of the schedule (FR-018-016; **G2057**, now closed). It now carries `schedule {scheduleId, nominalTime, kind, revision}` (`GET /runs/{id}` and `GET /runs`), absent for a run no schedule started, and `resetFrom {runId, sequence}` for a run a reset made. The run has **no `actualTime`**: the contract gives it none, and the action keeps it (`…23:24:04.040211Z`, 4.04 s after the nominal minute, inside the 5 s of jitter). Shown by `run_provenance` (integration, REST over a real PostgreSQL: a run now, a real firing at a nominal time on the schedule's grid that is passed through to the run to the microsecond, a reset, a plain rerun, a plain run), `contract_schedules_bulk_operations` (the schema) and the Studio, CLI and six-SDK tests of the note of T018-075; the compose stack was not started again (G2066) |
| 2.5 | pause with a note; no `SCHEDULED` action while paused | PASS | paused 23:26:08 to 23:28:37 (2 min 29 s): the minutes 23:27 and 23:28 have no action |
| 2.6 | trigger now while paused | PASS | `was triggered: STARTED, run …`; the action is `TRIGGERED` at `23:28:30.798523Z` |
| 2.7 | resume; the next action is the first minute after the resume | PASS | resumed 23:28:37; `next: 23:29:03.714Z` |
| 2.8 | `schedule backfill --start … --end …` | DOC FIXED | the flags are `--from` and `--to` (RFC 3339); `--to` later than now is `422 BACKFILL_RANGE_INVALID` (seen: I passed a minute ahead first) |
| 2.9 | backfill: `ALREADY_ACTED` for the minutes that ran, `STARTED` (sequentially) for the paused ones | DOC FIXED | right, but `ALREADY_ACTED` is **counted in the backfill** (`GET …/backfills`: `{"ALREADY_ACTED":3,"BUFFERED":1,"STARTED":1}`), not listed as an action; the two paused minutes are `BACKFILL STARTED` (the second waited, `BUFFERED`, for the first run, `BUFFER_ALL`); the CLI cannot list or cancel a backfill (G2058) |
| 2.10 | the audit trail shows `schedule.paused`, `.triggered`, `.resumed`, `.backfill_requested` "with the notes and actors" | DOC FIXED | the actions and the actors are there (`dev-admin`); **the note is not in the audit** (by design: a note is customer text, SC-018-014); it is on the schedule (`note`, `noteBy`, `noteAt`) |
| 2.11 | `schedule get` says how many runs are open | **DEFECT FIXED** | before the fix `schedule get` said `1 open run(s)` (later `2 open run(s)`) for a schedule whose runs had all finished seconds before, for as long as it was paused: it listed the runs the schedule *tracks*, which the pass forgets at its next step. Test first: `firing::a_description_lists_only_the_runs_that_are_still_open_though_the_pass_has_not_forgotten_the_ended_one` (red: `left: [01a1c2b0-12b0-…] right: []`). Fixed in `ScheduleService::describe`; the same test green; `rest::` and `service::` 95 passed. **Rerun on the rebuilt stack: see the section "Rerun after the fixes" below** |
| 3.1 | `workflow run hello-world --business-id order-123` twice: the second is `409 BUSINESS_ID_IN_USE` | DOC FIXED | `hello-world` finishes in 0.3 s: the first attempt happened to find it open (`409`), the retry 11 s later started a new run (the id was free: the default policy allows a duplicate after the holder ends). The page uses `cart`, which stays open: `409 BUSINESS_ID_IN_USE` (CLI exit 1) |
| 3.2 | `--if-exists use-existing` returns the same run | PASS | `run … already holds that business id: nothing was started` (CLI exit 0); by REST `POST /runs` with `USE_EXISTING` is `200` with the holder's run, and without the policy `409` |
| 3.3 | `--start-delay 2m` | DOC FIXED | the flag takes **seconds** (`--start-delay 120`) |
| 3.4 | the delayed run is `WAITING` with `startDelayedUntil`; a signal is `409`; it is released once after two minutes | PASS | `WAITING`, `startDelayedUntil: 23:25:53.199646Z`, `businessId: reminder-9`; the signal: `409 CONFLICT`, `details.reason` `STATE_CONFLICT`; journal `run.start_delayed` then, 0.9 s after the instant, one `run.start_released` |
| 4.1 | Signal-with-Start: `started: true`, `deliveredTo: ["wait-item"]` | PASS | by REST `202 {"runId":…,"started":true,"deliveredTo":["wait-item"]}`; the CLI prints `was started and signalled: 1 step(s) woke` |
| 4.2 | the same while the run is past `wait-item`: `409 NOT_WAITING`, nothing changes | PASS | `NOT_WAITING` (CLI exit 1). It holds **only while the run is open**: once the worker has finished it, the id is free and the call starts a new run; the page says so |
| 5.1 | `batch preview --workflow … --status RUNNING --created-before …` | DOC FIXED | the CLI takes `--selector '<JSON>'` (`{"filter":{"workflowId":…,"status":["WAITING"],"createdBefore":…}}`); a run that waits for a signal is `WAITING`, not `RUNNING`; `13 run(s) match` |
| 5.2 | `batch create … --action CANCEL --reason … --rate 5`; `batch get <id> --items` | DOC FIXED | right except `batch get` has no `--items`: `batch items <id>`; the job went `SELECTING`, `RUNNING` (12 of 13 in about 3 s at 5 a second), `COMPLETED` |
| 5.3 | items are `SUCCEEDED` or `SKIPPED`; counters equal items | PASS | `13 selected: 13 succeeded`, thirteen items `SUCCEEDED` |
| 5.4 | `reset-points`, `reset-preview`, `reset` | DOC FIXED | right; nothing in the demo fails, so the source is a run that **succeeded** (`<failedRunId>` cannot be made); the point is a sequence the list names (1, 7, 11); `--reapply signals` has something to deliver only from a point before the wait (1) |
| 5.5 | a new run with `rerunOf`, `run.reset`, the signals `delivered`, the source stays terminal | PASS | `rerunOf: <source>`, the journal `run.started`, `run.reset`, `wait.started`, `wait.satisfied`…; `signal item-added (event 3): delivered`; a second reset to point 7 kept `compose` and `wait-item` and ran `deliver` again; `sagat workflow resets` lists them |
| 5.6 | reapply needs `SAGAT_RESET_REAPPLY_ENABLED` | PASS | see 6.7 |
| 6.1 | engine killed while schedules are due | PASS | `kill` at 23:36:59, `up -d` at 23:37:21: the time due meanwhile was started once on the return, none doubled or lost; the worker stopped with the engine and the next time of the `SKIP` schedule was `SKIPPED_OVERLAP` until it was restarted (the page says so) |
| 6.2 | outage longer than the catch-up window | PASS | engine stopped 23:38:24 to 23:43:39 (5 min 15 s), window 120 s: `23:42:00 STARTED`, `23:43:00 COALESCED` (count 1), `23:39:00 MISSED_CATCHUP_WINDOW` (count 3, last 23:41:00) |
| 6.3 | authority revoked | DOC FIXED | `AUTHORITY_REVOKED`, schedule `PAUSED`, system note, audit `schedule.auto_paused`. No REST route removes a member: the demo adds and removes the member with SQL as the database owner, and the page says so |
| 6.4 | Signal-with-Start on a workflow whose entry does not wait | PASS | `409 SIGNAL_NOT_ACCEPTED_AT_START`, and `GET …/business-ids/…` is `404` |
| 6.5 | batch cancelled half way | **DEFECT FIXED** | **the first run failed**: `sagat batch cancel <id>` answered `sagat: content type must be application/json (BAD_REQUEST)`: the CLI sent a POST with no body, and the contract (`cancelBatchOperation`) requires `{"reason": 8..2000 characters}`; the page's `--reason` did not exist either. By REST the cancel works (`CANCELLING`, then `CANCELLED`: `5 succeeded, 7 cancelled`). Test first: `schedule_and_batch::a_job_is_read_listed_its_items_shown_and_cancelled` (the body and the content type) and the argument tests; `batch cancel <id> --reason <TEXT>` now sends the body. Rerun with the fixed CLI against the stack: `batch job … is CANCELLING`, then `22 selected: 6 succeeded, 0 skipped, 0 failed, 16 cancelled`; a second cancel `409 BATCH_NOT_CANCELLABLE`; no `--reason`: usage error, exit 2 |
| 6.6 | reset to a sequence that is not a point | PASS | `422`, `RESET_POINT_NOT_ELIGIBLE` |
| 6.7 | reapply on a deployment without the setting | PASS | control plane restarted with the flag `false`: `422`, `RESET_REAPPLY_NOT_AVAILABLE`, nothing made; with it on again the reset delivered the signal |
| 6.8 | reset that would deliver more than 256 things | PASS | a run of 257 waits sent 257 signals (11 s): the preview and the reset: `422`, `RESET_REAPPLY_TOO_MANY`; no reset was made (appendix) |
| 6.9 | reset of a run that has not ended (`--cancel-source`) | PASS | without it `409 RESET_SOURCE_NOT_TERMINAL`; with it `202 PENDING_SOURCE`, `newRunId: null`, then the run is `CANCELLED` and the engine makes the run (`CREATED`) |
| 6.10 | open run, a step in flight, no acknowledgement | PASS | `422`, `RESET_REEXECUTION_NOT_ACKNOWLEDGED`; the run stays `RUNNING` |
| 6.11 | open run on a deployment without `SAGAT_RESET_CANCEL_SOURCE_ENABLED` | PASS | `422`, `RESET_CANCEL_SOURCE_NOT_AVAILABLE`; the run stays `WAITING`, no reset |
| 6.12 | reset that waited and was killed | SKIPPED | needs a connection killed after a statement inside the transaction: only the test's trigger can (`fault_schedules_bulk_operations cancel_source`) |
| 6.13 | reset killed while it delivers | SKIPPED | the same (`fault_schedules_bulk_operations reset`: a kill after each of its 62 writing statements) |
| 6.14 | pause a trigger-owned schedule before the handover | PASS | `409 SCHEDULE_TRIGGER_HANDOVER_PENDING`; the tick fired every minute; a spec edit `409 SCHEDULE_TRIGGER_OWNED` |
| 6.15 | the handover | PASS | `handed over: … requirement epoch 1 …`; `HANDED_OVER` by `demo` as role `sagat`; the schedule paused, ran now, and no tick fired after the pause |
| 6.16 | handover while a publication is open | SKIPPED | needs a publication transaction held open: `trigger_round3::a_long_publication_makes_the_command_try_again_…` |
| 6.17 | run now or backfill a trigger schedule whose trigger an older control plane dropped | SKIPPED | there is no older control plane in the stack: `trigger_round3::a_backfill_or_a_run_now_of_a_schedule_whose_trigger_is_gone_…` |
| 6.18 | handover with an engine that lacks `schedules-v1` | PASS | a second engine with `SAGAT_SCHEDULES_ENABLED=false`: `error: 1 live execution engine(s) do not list schedules-v1 (instance ids: 934c24b5-…)`, nothing changed; after it was stopped the dry run passed |
| 6.19a | one tick reached by the legacy pass and the schedule pass | PASS | before the handover every tick of `cron-hello` has one run (`startedBy: system:engine`) and the schedule's action `ALREADY_ACTED` |
| 6.19b | … one killed after each write, in either order | SKIPPED | no kill points in the stack: `fault_schedules_bulk_operations upgrade::*` |
| 6.20 | quota at zero | PASS | `429 QUOTA_EXCEEDED`, `details.reason` `SCHEDULE_LIMIT`; the existing schedule fired next minute; the quota restored |

### Rerun after the fixes

The images were built a second time from the tree with the two fixes; **only the control plane was restarted on the new image**
(the fix of 2.11 is in `ScheduleService::describe`, which the control plane serves; the engine kept running the first build,
which holds neither fix), and the `sagat` binary of 6.5 was rebuilt.

- **2.11 on the rebuilt control plane (23:52 UTC).** The schedule was paused right after its 23:52:00 run finished. The database
  still tracks the ended run (`SELECT state, tracked_runs FROM schedules …` gives `PAUSED | {01a1c2b9-e01b-72d2-bfe7-5416af81c1b2}`;
  `workflow runs hello-world` says that run `SUCCEEDED` at 23:52:03.98), and the description now says
  `0 open run(s), 0 buffered` (`GET …/schedules/{id}`: `"openRuns":[]`); on the first build the same state said `1 open run(s)`.
- **6.5 with the fixed CLI** (against the first build's control plane, which is the same contract): `sagat batch cancel <id> --reason "stop
  now please"` gave `batch job … is CANCELLING: what is in flight finishes, the rest is cancelled`, the job ended
  `CANCELLED` with `22 selected: 6 succeeded, 0 skipped, 0 failed, 16 cancelled, 0 deferred`; the same without `--reason`:
  `sagat: --reason is required` and the usage, exit 2; a second cancel with a reason: `409 BATCH_NOT_CANCELLABLE`.
- **The snippets of the rewritten page**, run as written with the real CLI against the rebuilt stack (`replay.sh` of the
  session: the ids with `jq`, the `awk` captures of `$SCHEDULE`, `$RUN` and `$BATCH`, the delayed start and its signal, the REST
  Signal-with-Start, the selector with `date`, the batch job at 5 a second (`17 selected: 17 succeeded`, the preview and the job
  seeing the carts the earlier steps had left open), and the reset with its reapplied signal): every one gave what the page says.

### The count

53 steps: **32 PASS, 12 DOC FIXED, 3 DEFECT FIXED, 0 FAILED, 6 SKIPPED** (`1.1`, `6.12`, `6.13`, `6.16`, `6.17` and `6.19b` were skipped).
The two defects of 2026-11-09 (2.11, 6.5) were found by the demo, fixed with a test seen failing first, and rerun. The step that failed
that day (2.4, G2057) was fixed on 2026-11-10 across the OpenAPI, the AsyncAPI, the Studio, the CLI and six SDKs, with a test seen failing first in
each part, and is counted a defect fixed **without a rerun of the step on the stack** (G2066): it was `1 FAILED` in the count of 2026-11-09.

## (c) Success criteria and requirements

Evidence that is **not** from this task is quoted from the task notes of `tasks.md` (which record commands and results) or from
CI, and named so. CI run `42649041664` passed on head `345dd70e` (pull request #98, 2026-11-09 15:52 UTC) the six jobs it ran:
what to run, lake runtime, architecture rules, **sdks** (unit tests, the conformance suite, user stories, faults), **studio**
(lint, types, unit tests, build, the browser journeys) and **rust** (fmt, clippy, tests). It skipped contracts, scale and
recovery, native Go and Java, the local environment (compose smoke test), dependency advisories, the released v0.2 upgrade and
the observability files (the rules of a pull request, Constitution 4.3.0). Seven commits separate that head from `98818160`; the
run on `98818160` (`42717877244`), read with `gh pr checks 98` at about 00:05 UTC, had passed what to run, architecture, lake, **studio** (16 min 13 s) and the package smoke tests, with **rust** and **sdks** still pending: it is not evidence for any row until it ends. The names are those of
`tests/integration/schedules-bulk-operations/` (`integration_schedules_bulk_operations`), `…/fault/…` (`fault_…`) and
`…/contract/…` (`contract_…`) unless a crate is named.

| Criterion | Status | Evidence | What is missing or why |
| --- | --- | --- | --- |
| **SC-018-001** at least 200 table cases over every spec kind, zone and DST case, equal across two engine processes | VERIFIED | `calendar_processes::` (integration, 5 tests, 0.3 s, no database; 2026-11-10, D2160): 440 cases (eleven zones: Lisbon, New York, Sydney, Auckland, Santiago, Adelaide, Lord Howe, Kolkata, Kathmandu, Newfoundland, UTC; ten specs: calendars with the gap skipped or shifted and the fold taken once or twice, weekdays, monthly, intervals with an offset, hourly, two cron strings, an exclusion; jitter on seven of them; four dates around the changes of 2026) computed in this process and in two others started as this test binary with an empty environment and told opposite things (`TZ`, locale, `TZDIR`, directory, thread count): the 13,200 `(nominal, actual)` instants are equal as integers of milliseconds; eleven gaps and folds of the south and the half hour (Sydney, Auckland, Newfoundland, Adelaide, Lord Howe) equal the instants Python's `zoneinfo` gives, an oracle that does not share the database; a hand mutant (the jitter seeded with `$TZ`) is red; `cargo test -p sagat-infra-ir --test schedule_calendar --locked`: 27 tests, among them 730 daily occurrences for each of `Europe/Lisbon` and `America/New_York` (1,460 cases) against an oracle written from the zones' rules (T018-013); the three spec kinds, jitter and the first times in `schedules::` scenario 1 and the domain suites `schedules`, `schedules_properties`; this demo: an interval with jitter acted at nominal times on the exact minute with 4.04 s of jitter | *Closed 2026-11-10.* What the evidence does not say: the table of `schedule_calendar` checks the instants against the zones' rules written by hand for Lisbon and New York only, and the other nine zones are checked for equality across processes and, for eleven gaps and folds, against `zoneinfo`, not against a table of their own; two **builds** with two zone databases cannot be run here: the calendar embeds the `chrono-tz` of its build and nothing reports or compares the version (G2073, open) |
| **SC-018-002** crash at each statement of the firing transaction, a stale-fence replica and a duplicate pass give one run and one action per nominal time, with `startedBy` and provenance | VERIFIED | `fault … firing::`: the start (28 writing statements on 13 tables) and the skip (3 writes) are killed after each write: nothing changes, the next pass acts once, `verify_run` clean (T018-020); `fault … firing_decisions::` (12 tests, both roles, 2026-11-10, D2159, G1018): the steps that start no run are killed after each write too: a time buffered (3 writes), the look again at a waiting time (2), a time dropped with the buffer full (3), a recovery with no action left coalesced (3), times older than the window missed (3), a schedule with no action left (2), an ended run forgotten (2), a failed run forgotten and the schedule paused with its audit record (7), and the steps of a backfill (skipped 4, buffered 4, found acted on 3, the end of the range 3): the writes are the same on two schedules in one state, every table of the database is as it was after each cut, the step taken again leaves the state the watched one left (outcomes, times, counters, recovery, audit records), one action row for a time, the runs replay; a hand mutant (the audit record of the auto-pause in a second transaction) is red; `admission::` (the same time never starts a second run; the stale fence; the three lease tests); `pass::` two replicas; this demo 6.1 | (1) *closed 2026-11-10 (D2159):* the decision-only steps are killed at each write (what is still open of G1018 is a test that boots the control plane with schedules enabled and a skewed clock); (2) *closed 2026-11-10 (G2057):* the provenance is exposed: `schedule {scheduleId, nominalTime, kind, revision}` on the run and in the list, in the `run.finished` webhook as `scheduleId` and `nominalTime`, in the Studio header, `sagat workflow inspect` and the six SDKs (`run_provenance::`, `outbound::the_webhook_of_a_run_a_schedule_started_…`, the three finishing paths of `workflow-primitives`); it is the first run only: the later segments of a chain and the legacy cron pass's runs have none (G2063) |
| **SC-018-003** every overlap, catch-up, pause/resume, trigger-now, backfill, remaining-actions, pause-on-failure and revision/delete scenario of US02 records the expected outcome table | VERIFIED | `firing::` (skip, buffer one, buffer all, allow all, two processes one run), `cancel_other::` (7) and `fault … cancel_other`, `catchup::` (8, plus the trigger-owned 2), `backfill::` (11), `trigger::` (9), `changes::`/`service::`/`rest::` (pause, resume, revisions, delete), `firing::a_schedule_with_two_actions_left…` (remaining actions, pause on failure); this demo: 2.5 to 2.9, 6.2, 6.3 on the real stack | `TRACKING_LIMIT` is proved in the domain only (T018-026); T018-029 stays unticked only for the review of the trigger-owned part |
| **SC-018-004** revoking the authorizing subject pauses the schedule at its next action; no run starts | VERIFIED | `authority::` (6 tests; a revocation that commits between the read and the admission stops the run, found by a test that failed first); this demo 6.3: `AUTHORITY_REVOKED`, schedule `PAUSED`, system note, audit `schedule.auto_paused`, no run | none |
| **SC-018-005** rolling upgrade with old and new engines on one cron trigger: one run per tick and the 004 input; pause refused before the handover and honoured after; a trigger published by an old control plane gets its schedule from reconciliation | PARTLY | `trigger_firing::` (both orders of the two passes, `ALREADY_ACTED`, the key `cron-<epoch>`, the `cron.fired` document, `system:engine`), `fault … upgrade::` (each pass killed after every write; four sequences; an old binary after the handover), `trigger_handover::` (11), `trigger_schedules::pause_resume_trigger_and_backfill_are_refused_until_the_handover…`, `trigger_reconcile::` (8); this demo 6.14, 6.15, 6.18, 6.19a | the "old" engine is the legacy pass run in the same binary, and the old control plane is a row written by the test: no released binary was run (the released-upgrade profile is not run locally and was skipped by CI); T018-060 and T018-061 are built and **unticked, review pending** |
| **SC-018-006** 32 concurrent starts on one id give one open run; every reuse/conflict policy matches its table inside and after the window; a chain keeps the binding until final | VERIFIED | `business_ids::` (32 starts: one open run; 32 over a closed holder: one new run; 32 with `USE_EXISTING`: one run; the policy table inside and after the window; chain continuity; the terminal-commit race) and `fault … business_ids` (T018-036, ticked); this demo 3.1, 3.2; and, since 2026-11-10, the same claim made by a schedule's action (`scheduled_business_ids::`, `fault … slot_order::a_scheduled_action_under_a_business_id…`, `firing::` at every write of the start under an id) | none |
| **SC-018-007** delayed runs release once, are cancellable and refuse signals; a crash around release leaves one release; with 022 run-level concurrency the release acquires the permit or queues the run | PARTLY | `delay::` integration (9) and fault (kill after each writing statement of the release; the next pass releases once), the race with a run that stopped qualifying (T018-037); this demo 3.4: `WAITING`, signal `409`, one `run.start_released` | the 022 clause is **BLOCKED** (feature 022 not approved: T018-037, T018-053); a declared Update on a held run: closed by `update_with_start::a_delayed_run_admits_the_update` (G1972) |
| **SC-018-008** the four Signal-with-Start and three Update-with-Start cases of US04 pass, plus a Signal-with-Start whose new run 022 would queue; no signal is stored without a waiting step | PARTLY | `with_start::` (8), `with_start_rest::` (5), `update_with_start::`, `update_with_start_rest::` (T018-040, -041, -042); this demo 4.1, 4.2, 6.4 | the 022 case is **BLOCKED** (G1973 (2)) and the race of a wait's timeout against the delivery at creation has no test (G1973 (3)); T018-040 unticked for that |
| **SC-018-009** a 1,000-run job with an engine kill, a lost broker hint and a cancellation applies each run at most once, counters equal items, a run whose creating transaction was open at creation is selected, and a lock monitor never sees a batch transaction with two run locks | VERIFIED | `fault … batch_thousand::` (both roles, 19 s as the owner and 20 s as the runtime role, run three times as the owner, 2026-11-10, D2158): one job of **1,000 real runs** that cancels them at 200 items a second under the default settings of the executor and the pass, on two replicas of the engine (a keeper and a `BatchPass` each, over the pool of the runtime role when asked, the real run service for the effect); the thousandth run is started in a transaction that is **open when the job is created** (a lock held on the table of its last write), the selection takes nothing while it is open and then selects 1,000 items with that run among them; three times the process that holds the job has its connection cut at the record of an item **after the effect on the run committed** (the item is `PENDING`, its run `CANCELLED`, read from the database), is killed, replaced by a new one and the shard is taken over by a live one that records the item; a fourth is killed at a moment nothing chose; the first dead process, which still believes it holds its shards, takes no step; the API cancels the job after 700 items. At the end (701 succeeded and 299 cancelled as the owner, 700 and 300 as the runtime role; at least 700 succeed, the rest are cancelled), none skipped, failed or deferred (`selected` 1,000 equals the items and the runs, the counters add up, no item is stuck), no run has two requests to stop it in its journal and none is audited twice, the runs of the 701 are `CANCELLED` with one request and those of the 299 are still `RUNNING` with none (the job undoes nothing), one record of the end, the monitor of `pgrowlocks('runs')` looked 4,290 times (owner) and 4,520 times (runtime role) and the most run locks one transaction held was 1, no deadlock; hand mutants: a fresh key at each attempt (four items `SKIPPED`: a repeat answers as the first only because the key is the same) and a selection that does not wait for the horizon are red; `batch_executor::` (12: pages, the wait for an open transaction, a crash between the effect and its record), `fault … batch` (7: a kill after each write of the five transactions of the executor; a cancel racing an item), `batch_pass::`, `batch_locks::a_batch_job_and_the_api_on_the_same_runs_never_hold_two_run_locks_or_deadlock` (24 runs, the monitor looked 1,157 times, never two run locks, no deadlock); the benchmark: a selection of 10,000 runs in 0.3 s, 16 jobs shared fairly; this demo 5.1 to 5.3, 6.5 | *Closed 2026-11-10, with two things said.* The clause **"a lost broker hint"** has nothing to test: there is no hint (a job is found by the pass's own poll, and no message announces it), so a broker that loses one can neither delay nor repeat a step. **At most once** is by the key `batch:{job}:{run}`, while the effect and the record of the item are two transactions: the test cuts the second and kills the process, and the key makes the repeat the same effect; the window between the last look before the effect and the effect, and one extra effect after a cancel, are not cut by a test and are what O1951 asks the owner about (G1961, open) |
| **SC-018-010** missing `run:batch`, missing action permission (`deployment:manage` for `REPIN`), mid-job revocation, foreign run ids and `REPIN` without 019 are refused or skipped as specified | PARTLY | `batch::creating_a_job_needs_run_batch_and_every_permission_of_the_action` (the role matrix), `batch_executor::a_creator_who_loses_a_permission_fails_the_job…` (`AUTHORITY_REVOKED`), `batch_security::` (foreign ids), `batch::parameters_the_action_does_not_take…` (`REPIN` refused); `contract_rest_security_multitenancy` (T018-048, -070) | `deployment:manage` for `REPIN` is **BLOCKED** (T018-079 needs feature 019) |
| **SC-018-011** for every eligible point of a seeded corpus (a 022 permit holder, search-attribute upserts) the reset run's replay equals its projection, outputs and attribute rows equal the source, the 022 holder count equals the open non-restored acquisitions, a prefix with an event lacking a rule is refused, and ineligible, test-tree and purged sources are refused | PARTLY | `reset_corpus::` (12 tests, 2026-11-10, D2156, D2157): a parallel fork, a saga that completes, a saga that is compensated, a collection of two items and one of none, a loop that runs and one that does not, a run with a child, a timer, a wait satisfied by a signal and one by an event, and an approval (over REST), each ended by a real worker and scheduler and reset to **every point `reset_points` offers**, against the rule of D1022 written by hand (the points offered and why each other candidate is not one: `open-work`, `not-running`, `excluded:saga.aborting`, `scoped-events`): the replay equals the projection before the new run goes on and after it ends, rows `1..N` are the source's in every column (type, node, path, detail, payload digest, actor, trace, time) and marked `restored`, the root steps completed up to `N` hold the status and output of the source and the rest none, what a worker is given of the kept steps is what the source's was given, the steps that remain run again (and the children the prefix does not hold are started), every other place of the journal and the one after its end is `RESET_POINT_NOT_ELIGIBLE` for the reset and the preview and makes no run, one register entry per reset, the source untouched; **it found a defect** (the finish of a collection of no item was a point and the run made from it did not replay to its projection; fixed, D2156, with two domain tests and the corpus seen failing first); hand mutants (the open-work block, the excluded-event block, the fix) are red; `reset::at_every_point_the_run_that_is_made_replays_to_its_projection…` (all four points of a sequential run), `fault … reset` (a kill after each of 62 writes: nothing changes, the retry makes one run), `reset::` refusals (`RESET_RULE_MISSING` with the type named, `RESET_NOT_SUPPORTED_FOR_SOURCE` for a child and a chain member), the benchmark (replay equals projection for 45 runs up to 5,000 restored events); this demo 5.4, 5.5, 6.6 | *The shapes are in the corpus; what it says of the code* (G2072, with G1979, the owner's): no run of a collection or a loop can be reset to anything but its start (the events of a scope are not restored, G1980 (1)), and the completion of a parallel, a saga, a timer, a wait, an approval's decision and a loop are no points, so those runs are reset around the node and never to its completion; the declared state is checked at one point only (`reset::the_declared_state_and_the_output_of_a_signal_in_the_prefix_are_restored`); a prefix at the ceiling is tested by its boundary and not with 5,000 events. The 022 permit holder and 020 attribute rows, `RUN_PURGED` and test-tree refusals are **BLOCKED** (022 and 020 not approved: T018-053, -055, -077). The database-execution branch of `RESET_NOT_SUPPORTED_FOR_SOURCE` has no test **and cannot have one by SQL**: a trigger ties `origin` to an internal workflow, so a test cannot make such a run, and none was forced (G1980 (5)) |
| **SC-018-012** `CANCEL_SOURCE` racing source completion, a crash during the reset transaction, reapply order and `NOT_REAPPLIED` behave as specified; the source never reopens | VERIFIED | `cancel_source::` (the pending reset, the races, the fence), `fault … cancel_source` and `cancel_source_expiry`, `cancel_source_rest::` (the engine with the flag makes the run; a step in flight needs the acknowledgement), `reapply::` (order, the first undeliverable stops the rest, `LEGACY_EVENT`, the ceiling of 256, key replay) and `fault … reset`; this demo 5.5, 6.7 to 6.11 | built and **unticked** (T018-056, T018-057: left for the maintainer to tick after reading them, reviews done); a failed source compensation is in `cancel_source::` |
| **SC-018-013** OpenAPI additive, AsyncAPI baseline only grows, IR and worker baselines unchanged, reasons documented; five SDKs pass the shared scenarios | PARTLY | OpenAPI: 27 operations marked `x-implemented: '018'`, `contract_schedules_bulk_operations` (8 tests, 2026-11-10; 7 when the row was first written), `docs/reference/api.md` blessed, the cross-tenant battery over every one; IR: the one additive, capability-gated field `reapplyOnReset` (T018-078); worker protocol unchanged; reasons: `contracts/errors.md`, `docs/runbooks/*`; SDKs: `sdk_conformance` over the whole suite, 75 scenarios in six languages, none skipped, 1,853 s (T018-068, 2026-11-06) and the CI job **sdks** green on `345dd70e` | *closed 2026-11-10 (T018-010, D2143):* the AsyncAPI additions (`data.scheduleId`, `data.nominalTime` of `RunFinishedWebhook`, by reference the broker body) are made and `events.baseline.json` regenerated on purpose (the diff is those two properties); `contract_asyncapi_events_webhooks_messaging` 5 passed without bless; the six SDKs read `Run.schedule` and `Run.resetFrom` and `schedules-operate` holds the same two observations in six languages (the whole suite was rerun on 2026-11-10: section (h)). **Still missing:** nothing asserts that `workflow.schema.json` and `worker.baseline.json` are unchanged (checked with `git diff` against the merge-base with `develop`: G2065) |
| **SC-018-014** canary schedule inputs, batch payloads and business ids are absent from logs, spans, metrics, audit and outbox; each new quota refuses before work; the real OTel export has fixed labels only | PARTLY | `observability::customer_text_…` (the real exporter to a collector that keeps bodies: eight canaries, every log line, the Prometheus page; the search is shown able to find a planted canary), `privacy::` (a business id through every table, log and metric), `batch::` (a signal payload is in no job row and no audit record), `disclosure::`, `observability::every_ceiling_…` (five quotas: `429`, no table changed), `counted::` (label closure), `audit_rollback::` (8) (T018-071, -039, -033, 2026-11-09); this demo 2.10 (the note is not in the audit) | a batch **signal payload** is checked in the tables, not in the export, the logs or the outbox; the audit rollback does not cover the firing, the backfill, trigger-now, `revoke_authority` or the item audit (G2031); G2028, D2085 are the agent's decisions the owner has not accepted; T018-071 unticked on that |
| **SC-018-015** a real browser runs the schedule, batch and reset flows keyboard-only with WCAG AA contrast and table alternatives; the quickstart demo runs | PARTLY | fourteen keyboard-only journeys (`frontend/e2e/tests`: schedules 4, batch 4, reset 4, business id and delay 2), axe `wcag2a/2aa/21a/21aa/22aa` on the list, the form, the dialog, the error and the empty state; three runs in a row locally (14 of 14) and one run on Linux in CI (job **studio** on `345dd70e`); **the quickstart demo** is this task: run for real on 2026-11-09, three defects fixed (2.4 on 2026-11-10, without a rerun on the stack: G2066) and no step failed, six steps skipped and four rows of the failure table not shown (counts by `grep` of the table of section (b): 32 PASS, 12 DOC FIXED, 3 DEFECT FIXED, 0 FAILED, 6 SKIPPED, 53 in all) | T018-074 is narrowed pending the owner (O1952): no journey for the preview or the cancel of a backfill (G2038), and the failure states of G2034/G2035; only one Linux run; the demo's outputs are not a person's first day (SC-012-001 is separate) |
| **SC-018-016** a barrier test runs every run-driven transition against each aggregate path; `pg_locks` never shows a run-driven transaction holding a schedule, business-id, job, item, obligation or pending-reset row, and no deadlock occurs | VERIFIED | `fault … run_driven_locks::` (ticked, T018-076): an `ACCESS EXCLUSIVE` lock on every table of the aggregates, found by name, under which a completion, a failure, a lease that runs out, a deadline, a cancellation, a timer, a wait timeout and a hand-off must still finish within 60 s; the controls show the firing and the reset request do wait; hand mutants (an unconditional and a conditional read) turn it red; `batch_locks::` reads `pgrowlocks` and `pg_stat_database.deadlocks` | the proof is by blocking the tables, not by sampling `pg_locks` of the run-driven transaction, as the task's own note says (a stronger form of the same sentence) |
| **SC-018-017** an activation racing a registering instance never leaves it serving; an instance lacking the capability answers `BINARY_CAPABILITY_MISSING` or stops its passes; an instance silent for 30 s no longer blocks an activation | VERIFIED | `instances::` (33), `instances_hostile` (7), `instances_migration` (2), `instances_process` (11), `instances_rest` (7), `fault … instances` (the race in both orders held by a lock and watched in `pg_stat_activity`, the limit under 24 concurrent registrations); this demo 6.18 (the engine without the capability is named and blocks the activation; once stopped it no longer does) | built and **unticked, review pending** (T018-080); the flags of batches and resets are not on the registry (G1999) and what a holder of the runtime role can still do is G2000 |
| **FR-018-016** scheduled runs: workflow, version policy, input, `nominalTimeField`, business-id prefix with policies (id `<prefix>-<nominal time>`); `startedBy` and the provenance of the run | PARTLY (2026-11-10) | the prefix and the policies are written and read back over REST, the six SDKs, the CLI and the Studio; each run is bound to `<prefix>-<nominal time>` in the transaction that admits it (`scheduled_business_ids::`, `rest::an_action_under_a_business_id…`, the conformance scenario `schedules-operate`); a held or remembered id is the recorded decision of the time, terminal, never retried, the schedule not paused and no action used up; the lock order of the plan (`slot_order::`) and the crash matrix (`fault … firing::`, 34 and 4 writes) hold; the prefix and the id reach only the revision, the binding and the run (`privacy::`, `observability::`); provenance of the run: G2057 (h) | the input is not checked against the schema of the target (G1955, G2081) |
| **FR-018-046** contracts and compatibility | PARTLY | REST additive under `/v1` (`contract_schedules_bulk_operations`); the worker protocol unchanged (`worker.baseline.json`); the IR gains only `reapplyOnReset`; existing starts, signals, reruns, cron triggers and SDK clients unchanged: the legacy pass and the SDK conformance on `345dd70e`; this demo: the cron trigger fired as before until the handover | *the AsyncAPI additions are made (2026-11-10, T018-010)*, **with one limit that stays open against this requirement: the final head of a chain a schedule started is a successor that copies no schedule column, so its `run.finished` webhook carries no `scheduleId` (G2063, tested on a real chain)**; the released-upgrade profile (an older release against the new schema and back) was not run here and CI skipped it on the pull request |
| **FR-018-049** delivery: runbooks, alert rules for late schedules and stalled batches, trigger-owned migration notes, the reproducible demo | PARTLY | `docs/runbooks/{schedules,batch-operations,run-reset,business-ids,instance-registry}.md` (with the capacity and the handover procedure), alert rules `SagatSchedulerLate` (carries the schedule and batch lag, summary names them) and `SagatBatchJobsStalled` with `promtool test rules` (`batch_stalled_test.yaml`, in CI); `infra/observability/tests/instance_registry_test.yaml` run by hand only; **the demo of `quickstart.md`**: rewritten from a real run (this document) | no operator has reviewed the runbooks (T018-073 unticked); a separate `SagatSchedulesLate` was never made; the demo has no FAILED step (step 2.4 was fixed on 2026-11-10 and not rerun on the stack, G2066), six SKIPPED steps and four rows of the failure table it cannot show; the four lines are not in `.env.example` (G2061); the quickstart's snippets were replayed one by one, not the whole page in one go |
| **NFR-018-005** performance measured on a declared environment, nothing promised | VERIFIED (as measured) | `tests/performance/reports/schedules-bulk-operations.md` and its two addenda (T018-072, 2026-11-09): firing lag, batch rate, claim latency, reset time, with the failures; findings G2043 to G2050 | not rerun here; **it promises nothing** and says so; one machine, one day; the findings that ask for decisions (a ceiling, a pace) are the owner's |

Counts of the criteria: **VERIFIED 6** (003, 004, 006, 012, 016, 017), **PARTLY 11** (001, 002, 005, 007, 008, 009, 010, 011, 013, 014, 015)
plus the two requirements FR-018-046 and FR-018-049 **PARTLY**; **BLOCKED** and **NOT DONE** appear as parts of PARTLY rows
(022 and 019, in 007, 008, 010, 011; the AsyncAPI additions in 013) and in section (d), because no criterion is wholly blocked.
NFR-018-005 is VERIFIED as measured.

## (d) Open and blocked tasks

28 of the 81 tasks are unticked (`grep -c '^- \[ \]'`); 53 are ticked. The unticked ones, with what the notes say remains and
what unblocks them:

| Task | State | Why it is open | What unblocks it |
| --- | --- | --- | --- |
| T018-007 | built, line not reconciled | the 2026-11-04 note leaves `pending_resets` "open with US06"; US06 built it (`observability::every_ceiling_…` covers it) | the maintainer reads and ticks |
| T018-008, -009, -011 | built, lines not reconciled | each note lists parts "still open" at its date (the business-id table, `runs` columns, resets, the registry, the purge functions); the migrations exist (`20261230…` to `20270115…`, 15 of them) | the maintainer reconciles; the trigger-owned backfill became the reconciliation pass |
| T018-010 | built in part (updated 2026-11-10) | the AsyncAPI additions and the baseline regeneration are **done** (D2143); what is not: no test asserts that `workflow.schema.json` and `worker.baseline.json` are unchanged (G2065), and the `inputBytes` of G1015 | the maintainer's decision whether the sentence needs an assertion of its own, then the tick |
| T018-012 | built in part | the slot-order cases of the admission paths of 019, 021, 022, 024, 025, 026 and 030 exist only as those features merge (G2029) | features 019, 022 (not approved), 021, 024 to 026, 030 |
| T018-016 | built in part (updated 2026-11-10) | the business-id prefix of a schedule's action is **built** (FR-018-016: the prefix, the two policies, the id of every run, the decision of a held id; (j)); what is left is the input check against the schema of the target at write and at action (G1955, G2081: the platform enforces `inputSchema` nowhere, so it is the owner's) | the owner's answer on G1955 |
| T018-029 | built | trigger-owned catch-up done (D2040); unticked with T018-060/-061 until their review | the reviews |
| T018-037 | built, **022 BLOCKED** | the release does not acquire 022's run permit | feature 022 approved and merged |
| T018-040 | built, **022 BLOCKED** | the new run 022 would queue; the timeout race (G1973) | feature 022; a test |
| T018-047 | built, **022 BLOCKED** | `TEST_RUN` skips need 022 | feature 022 |
| T018-051 | **NOT DONE / BLOCKED** | purge of jobs and items; the selector through 020's `RunSelector` (the selector by `businessIdPrefix` is done, G2027 closed: (j)) | feature 020 approved and merged |
| T018-052, -053, -054, -055 | built in part | the corpus has its shapes since 2026-11-10 (D2157; it found and fixed D2156) and says which completions are no points (G2072, with G1979, asks the owner); the 022 permit and the 020 attribute fold; scoped prefixes and a configurable ceiling (G1980); the database-execution branch has no test and cannot have one by SQL | owner decision, features 020 and 022 |
| T018-056, -057 | built, review done | left unticked "for the maintainer to tick after reading" | the maintainer |
| T018-060, -061 | built, **review pending** | D2033 to D2045, G2004 to G2008; the Studio's disabled buttons before the handover (G2004) | the review |
| T018-071 | every item has evidence | left for the maintainer if D2085 and the limits of G2028 and G2031 are accepted | the owner |
| T018-072 | done | the benchmark report is read-only evidence; its findings ask for decisions (D2112) | the maintainer |
| T018-073 | built | an operator's review of the runbooks | an operator |
| T018-074 | built, **narrowed pending the owner (O1952)** | no journeys for the preview or the cancel of a backfill (G2038); G2034, G2035 | the owner's answer to O1952 |
| **T018-075** | this task | quickstart demos done (this document); **the gates are the maintainer's** (section (e)); the Studio build and the runtime-role sweep were not run here | the maintainer runs section (e) and ticks |
| T018-077 | **BLOCKED** | features 020 and 022 integration | 020 and 022 merged |
| T018-079 | **BLOCKED** | `REPIN` needs feature 019 | 019 merged |
| T018-080 | built, **review pending** | the registry; G1999 (batches and resets still on per-process flags: ADR-0081), G2000 | the review; ADR-0082 accepted |

Owner questions open: **O1950** (who may write a `CANCEL_OTHER` schedule), **O1951** (must the effect on a run and the record of
its batch item be one transaction), **O1952** (which failure states the browser journeys must show). The decisions the agent
took and the owner has not accepted: D2014, D2016, D2025 and ADR-0081, ADR-0082 and ADR-0083 (all **proposed**). Features 019,
020 and 022 are not approved; 029, 031 and 036 will activate through the same registry.

## (e) Final gates

The maintainer runs these on the merged tree (rule 17) and fills the table. **Nothing below was run by this task unless the
"Run here" column says so**; the blanks stay blank.

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
GITHUB_EVENT_NAME=pull_request GITHUB_BASE_REF=develop python3 scripts/check-design-docs.py
python3 scripts/check-architecture.py && python3 scripts/check-agent-config.py && python3 scripts/check-upgrade-boundary.py
RUST_TEST_THREADS=2 cargo test --workspace --locked --no-fail-fast
for t in integration fault contract e2e perf; do cargo test -p sagat-tests --test ${t}_schedules_bulk_operations --locked; done
for t in integration fault; do SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test ${t}_schedules_bulk_operations --locked; done   # the runtime-role sweep
cargo test -p sagat-tests --test integration_platform_foundation --test integration_security_multitenancy --test contract_rest_security_multitenancy --test contract_developer_experience --test contract_observability_operations --locked
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_platform_foundation --test integration_security_multitenancy --locked
cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture      # six languages, every scenario
cargo test -p sagat-tests --test e2e_workflow_studio --locked -- --ignored journeys     # the browser journeys
(cd frontend && npm run lint && npm run typecheck && npm test && npm run build)
(cd frontend/apps/studio && npm run api && npm run check:api)
cargo test -p sagat-tests --test contract_developer_experience --locked   # with SAGAT_DOCS_BLESS unset: docs/reference/cli.md was changed by hand in this task (one line of the usage), it must be green
promtool check rules infra/observability/alerts.yaml && promtool test rules infra/observability/tests/*_test.yaml
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env.example config -q   # the compose file with the four new lines
# the per-SDK checks listed in sdks/AGENTS.md (TypeScript, Python, Go, C#, Java, Rust)
```

| Gate | Result | Run by / date | Notes |
| --- | --- | --- | --- |
Filled on 2026-11-10 by the maintainer's session (Claude Code, one script, `gates53.sh`, 41 steps, every one with exit 0, the full output of
each kept) on commit `52c56b01` (head of pull request #98 after the run provenance), private `CARGO_TARGET_DIR`, `RUST_TEST_THREADS=2`, real
PostgreSQL, a Mac15,9 shared with other work. What was **not** run is said in the row.

| Gate | Result | Run by / date | Notes |
| --- | --- | --- | --- |
| `cargo fmt --check` | pass | maintainer's session, 2026-11-10, `52c56b01` | |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass | same | |
| `check-design-docs.py` (pull-request mode, base `develop`), `check-architecture.py`, `check-agent-config.py`, `check-upgrade-boundary.py` | pass | same | `check-architecture.py` also enforces that the shard lease row is a leaf lock and that every `run.finished` body says its schedule; the 365 Python tests of `scripts/tests` pass (2 skipped) |
| `cargo test --workspace --locked --no-fail-fast` (`RUST_TEST_THREADS=2`) | **partly** | same | the unit and doc tests of every crate except `sagat-tests`: 2,957 passed, 0 failed; the `sagat-tests` targets were run one by one (the rows below), **not** the whole workspace in one command: the targets of other features that this branch does not touch were not run here (CI runs the ones a pull request selects) |
| `integration_schedules_bulk_operations` (owner and runtime role) | pass | same | 638 passed, 1 ignored, in both roles |
| `fault_schedules_bulk_operations` (owner and runtime role) | pass | same | 99 passed in both roles |
| `contract_schedules_bulk_operations`, `e2e_…`, `perf_…` | **partly** | same; `perf_…` 2026-11-10 earlier | `contract_` 8 passed; `perf_schedules_bulk_operations` as CI runs it: 4 passed, 13 ignored (the benchmark itself ran on 2026-11-09, report in `tests/performance/reports/`); `e2e_schedules_bulk_operations` not run |
| `integration_platform_foundation`, `integration_security_multitenancy` (both roles) | pass | same | 104 and 126 passed in both roles (the runtime-privilege audit is in the first) |
| `contract_rest_security_multitenancy`, `contract_developer_experience`, `contract_observability_operations` | pass | same | 21, 8 (`docs/reference` unchanged, no bless) and 11; also `contract_payload_classification` 18, `contract_iceberg_writes` 63, `contract_asyncapi_events_webhooks_messaging` 5, `contract_rest_events_webhooks_messaging` 2 |
| `sdk_conformance` (all scenarios, six languages) | pass | same | `-- --ignored`, one server build, 1,723 s; registry tests 3 passed. It failed twice on 2026-11-09 for reasons that are not the feature (the OpenBao scenario once under load: `database.rs:103`; the `reset` golden once, a real change that was re-blessed from TypeScript and reproduced by the others) |
| the six SDKs' own checks | **partly** | the provenance task, 2026-11-10; CI | each SDK's checks passed after the typed `schedule`/`resetFrom` were added (TypeScript, Python, Go, C#, Java, Rust, as `sdks/AGENTS.md` lists them); not repeated by the maintainer's session on this commit; the CI `sdks` job passed on `98818160` (before the provenance) and its run on `52c56b01` is the confirmation |
| Studio: lint, typecheck, `npm test`, build, `check:api` | pass | same | lint, `typecheck` and `typecheck --workspace @sagat/e2e` clean, 99 files and 1,464 tests, build, `check:api` matches the contract, `npm audit --audit-level=moderate` 0 vulnerabilities |
| browser journeys (`e2e_workflow_studio … journeys`) | pass | same; CI | 52 passed (the 14 journeys of this feature and the others), 3.7 minutes; CI passed it on Linux on `345dd70e` and `98818160` |
| `promtool check rules`, `test rules` | **not run** | | `promtool` is not installed here; `trigger_schedules_test.yaml` and `instance_registry_test.yaml` exist and were reported passing by the implementing task, the CI step runs only `batch_stalled_test.yaml` (G2022) |
| `docker compose … config -q` | **partly** | the quickstart task, 2026-11-10 | with none of the four switches set, `docker compose config` resolves all four to `false`; the four lines are not in `infra/docker/.env.example` (G2061: the sessions were denied reading `.env*`), so the command with `--env-file infra/docker/.env.example` was not run on a file that has them |
| the quickstart, end to end once more on a clean stack | **not run** | | the quickstart was run for real on 2026-11-10 on the commit before the provenance; step 2.4 was not rerun on a stack after it (G2066) |

## (f) What is not verified, and why

- **Linux.** Every demo step ran on macOS with a Docker Desktop VM. The Linux evidence is one CI run (`42649041664` on `345dd70e`)
  of six jobs; the rule of a pull request skips the rest, and the run on `98818160` had rust and sdks pending at 00:05 UTC.
- **The released-upgrade profile** (an older release against the new schema, and back): not run here (the six local `release_upgrade`
  tests are `#[ignore]` and need the verified v0.2.0 build) and skipped by CI on the pull request. SC-018-005 and FR-018-046 lean on
  it for "old engines".
- **No mutation battery.** Mutation testing runs when a release is closed (OD-2026-11-06-2); the task notes record hand mutants of
  the individual tasks, not a battery of this release. This task ran none.
- **The workspace tests, clippy of the whole workspace, the full runtime-role sweep, the Studio build, the six SDKs and the
  conformance suite were not run by this task** beyond the commands listed in section (g): the gates are the maintainer's
  (section (e)).
- **A person's first day** is not timed (SC-012-001 of feature 012 is open); the demo was followed by its author, who knew
  where it would go wrong.
- **What the stack cannot show**: four rows of the failure table (6.12, 6.13, 6.16, 6.17) and the kills of 6.1 and 6.19b need fault
  injection inside a transaction or an older binary; their evidence is the named tests, which CI ran once.
- **Scale and load** (G2050): one tenant, one engine replica, no HTTP in front, development credentials, no TLS.
- **The quickstart as one script**: its snippets were run one by one (some with ids filled in, the `awk` and `jq` forms
  separately), not the whole page from an empty stack in one go.

## (g) What this task changed

| Where | What | Why |
| --- | --- | --- |
| `infra/docker/compose.yaml` | the four feature flags passed from the environment to the control plane and the engine, each `false` by default, with a comment | step 1.2; no default of a binary changed; `docker compose config` validates it |
| `apps/cli/src/{args,batch}.rs`, `apps/cli/tests/schedule_and_batch.rs`, `docs/reference/cli.md` | `sagat batch cancel <id> --reason <TEXT>` sends the JSON body the contract asks for | defect of step 6.5 (D2132) |
| `crates/application/src/schedules.rs`, `tests/integration/schedules-bulk-operations/firing.rs` | the description of a schedule lists only the runs still open | defect of step 2.11 (D2133) |
| `specs/018-…/quickstart.md` | rewritten to the truth | D2134 |
| `specs/018-…/{research,tasks}.md` | decisions D2131 to D2136, gaps G2057 to G2062, a dated note under T018-075 | D2135 |
| `docs/runbooks/*` | sentences that the demo showed false (listed in the note of T018-075) | cross-check |

The commands that were run for the two fixes and the documents (private `CARGO_TARGET_DIR=/Users/marcelo/cargo-target-018`,
`CARGO_PROFILE_DEV_DEBUG=0`, `RUST_TEST_THREADS=2`): `cargo test -p sagat-cli --locked --test schedule_and_batch` (red: `variant
BatchCommand::Cancel has no field named reason`, then 13 passed), `cargo test -p sagat-cli --locked` (every suite `ok`, 0 failed),
`cargo test -p sagat-tests --test integration_schedules_bulk_operations --locked -- a_description_lists_only_the_runs` (red:
`a run that ended is not an open run … left: [01a1c2b0-12b0-726b-a603-30843b31e354] right: []`, then 1 passed in 2.08 s) and
`… -- rest:: service::` (95 passed in 66 s), then `… -- firing:: service:: rest:: schedules:: actions::` in **both roles** (145
passed in 93 s as the owner, 145 passed in 94 s with `SAGAT_TEST_AS_RUNTIME_ROLE=1`), `cargo fmt --check` (clean), `cargo clippy -p
sagat-cli -p sagat-application -p sagat-tests --all-targets --locked -- -D warnings` (clean), `cargo test -p sagat-tests --test
integration_platform_foundation --locked -- runtime_privileges` (43 passed: the audit that reads every Rust literal as SQL),
`GITHUB_EVENT_NAME=pull_request GITHUB_BASE_REF=develop python3 scripts/check-design-docs.py`, `check-agent-config.py` and
`check-architecture.py` (all three pass), `docker compose config` with an environment that sets none of the four lines (all four
`false`), a check that every relative link and anchor of the five pages written or edited resolves (0 problems), and `cargo test -p
sagat-tests --test contract_developer_experience --locked` (8 passed,
among them `the_command_reference_is_what_the_tool_prints`, the golden of `docs/reference/cli.md` whose one line was edited by
hand, and `the_links_in_the_docs_lead_to_files_that_exist`, without the bless switch). **Not run:** the rest of the suites of
`integration_schedules_bulk_operations` (635 tests; 145 of them ran, in both roles), the fault and contract suites, the workspace
tests and the other crates' clippy.

## (h) The run provenance slice (2026-11-10, G2057, T018-010)

What it closes: a run says which schedule started it (FR-018-016) and which run a reset made it from (FR-018-037), and the `run.finished`
webhook says which schedule started the run (FR-018-046). The decisions are D2141 to D2146, the gaps it opens G2063 to G2066. The tree
was the commit `2e52f5b2` plus the uncommitted changes of this slice; nothing was pushed or fetched. The stack was **not** started: step
2.4 is a defect fixed and not rerun (G2066).

| Part | Evidence (run on 2026-11-10 unless said) |
| --- | --- |
| REST `Run.schedule`, `Run.resetFrom` | `integration_schedules_bulk_operations run_provenance::` (4: a run now compared with its action to the microsecond and then under revision 2; a real firing, under a fixed clock, of a nominal time one period before the schedule's first calculated time, passed through to the run, with the schedule's own calculation of the next time checked after it; a plain run and a plain rerun without either field, `absent` not `null`; a reset); `contract_schedules_bulk_operations` 8 (the run and the list validated against the schema, the reset case of `resets::`); the unit tests of `dto_exec` 3 |
| AsyncAPI, baseline | `contract_asyncapi_events_webhooks_messaging` 5 without bless (the body of a scheduled run validates against `RunFinishedWebhook` and every property of `data` it writes is declared); `events.baseline.json` regenerated, the diff is `data.scheduleId` and `data.nominalTime` on `RunFinishedWebhook.payload` and nothing else |
| The webhook of each finishing path (what is real and what is stamped) | **Run transaction, real:** `integration_events_webhooks_messaging` 112 (`run_provenance::`: the body of a run a firing started, the body of a plain run with exactly its old keys, and a run that has both a business identifier and a schedule: its `data` key for key, the canary of each in no body and no `event_outbox.payload`; hand mutant: `businessId` added in the scheduled branch, red). **Collection, loop and chain-finalization writers, synthetic:** `integration_workflow_primitives` `…schedule_started…` (collection, loop) and `the_finalization_writer_says_the_schedule_a_stamped_head_carries_…` (chain) on runs **stamped by hand** with the provenance columns (the guard trigger off for one statement), each with and without a schedule, seen failing with the function switched off: they prove each writer carries the data, not that the firing reaches it (G2069). **A real chain, from a real firing: the final head says none:** `chain_handoff the_final_head_of_a_chain_a_schedule_started_has_no_schedule_and_its_webhook_says_none` (the first segment has `runs.schedule_id`, the successor none, the final head's `run.finished` no `scheduleId`): the product's behaviour (D2144), and **G2063 stays open against FR-018-046**. **The rule:** `check-architecture.py` fails when a writer builds the body without the call (D2148, nine tests, hand mutant red) |
| Rollback tolerance | `integration_workflow_primitives iteration_policy_zero::zero_policy_writer_and_downgrade_obey_both_relation_lock_orders` failed on the first version (a run finished on a schema without the columns: 762 of 763) and passes with the snapshot read (D2143). **After that fix the gates that depend on the finishing paths were rerun whole on the final tree:** clippy of the workspace; `integration_workflow_primitives` 763 passed (22 ignored); `integration_events_webhooks_messaging` 112; `fault_events_webhooks_messaging` 8; `contract_asyncapi_events_webhooks_messaging` 5; `integration_platform_foundation` 104 and `integration_schedules_bulk_operations` 638 (1 ignored), each in both roles; `contract_schedules_bulk_operations` 8; `integration_security_multitenancy` 126; the workspace tests outside `sagat-tests` (exit 0). The other rows below ran before it, on code that differed only in that one function |
| Studio | 8 vitest tests of the run header (`RunDetailPage.test.tsx`), the whole suite 1,462; `lint`, both `typecheck`, `build`, `check:api`, `npm audit` clean; the browser journeys 52 passed (the first run failed three, all three journeys updated), among them the one that follows the provenance link back to the schedule, keyboard only, with axe |
| CLI | `inspect_provenance` 4 (a schedule, each kind in words, an unknown kind and a hostile value, a reset, a run with neither); the three requests of the plain output unchanged; `contract_developer_experience` 8 without bless (`docs/reference/cli.md` and `api.md` are unchanged) |
| Six SDKs | each SDK's own checks (TypeScript 160, Go `-race`, `vet`, `gofmt`; Python 365, mypy, ruff; C# 512 and `format`; Java `verify`; Rust fmt, clippy, tests); `sdk_conformance` **76 scenarios in six languages**, 75 equal to their goldens at once and `reset` equal after the four `resetFrom` objects were blessed from TypeScript (diff read: only additions), `reset` and `schedules-operate` rerun once more in the six; `e2e_sdk_multilanguage` 6, `fault_sdk_multilanguage` 9 |
| Runtime role | `integration_schedules_bulk_operations` 638 (1 ignored), `fault_schedules_bulk_operations` 99, `integration_platform_foundation` 104 (with the audit of every SQL literal) and `integration_security_multitenancy` 126, each in both roles; `contract_rest_security_multitenancy` 21 |
| Other targets of the brief | `contract_developer_experience` 8, `contract_payload_classification` 18, `contract_observability_operations` 11, `contract_iceberg_writes` 63, `contract_rest_events_webhooks_messaging` 2, `contract_rest_durable_execution` 2, `contract_grpc_*` 70, 9, 7 and 10, `fault_events_webhooks_messaging` 8, `fault_platform_foundation` 38, `e2e_platform_foundation` 42, `e2e_developer_experience` 4 and 1, `e2e_database_connections` 1 |
| Static | `cargo fmt --check`, clippy of the workspace with `-D warnings`, `check-architecture.py`, `check-agent-config.py`, `check-design-docs.py` (pull request mode), `check-upgrade-boundary.py` |

**What the gates found that the slice had not:** the rollback tolerance above; the shared `reset` golden; three browser journeys; and a
failing test on the committed tree that is not this slice's (`sagat-execution-engine --test config
no_shipped_manifest_turns_the_reset_switch_on`: the guard read `${SAGAT_RESET_CANCEL_SOURCE_ENABLED:-false}` of `compose.yaml`, from
the commit `731b67ed`, as a switch turned on; `value_given` now reads the default of an interpolation, four cases added). **What was not a
defect:** one failure of `contract_grpc_durable_execution protocol::task_stream_lifetime_admission_…` while three suites ran together (it
passed alone and in the whole target), and a restart of Docker Desktop during a run, which failed 166 tests of `workflow-primitives` and
the first whole-suite conformance run (`UnexpectedEof` on a database connection; every container of the machine had exited seconds before):
the two were rerun whole. **Not done:** the compose stack (G2066), a test that pins `workflow.schema.json` and `worker.baseline.json`
(G2065), provenance on the later segments of a chain and on the runs of the legacy cron pass (G2063), a column or filter in the Studio's run
list (G2064), mutation testing, the released-upgrade profile and Linux (CI's).

**Review fixes (2026-11-10; D2147 to D2155, G2067 to G2071).** Three independent reviews found no blocker; their findings were fixed in the same tree
with the test first or a hand mutant for each guard. The rows above were corrected (the webhook of each finishing path says what is real
and what is stamped; SC-018-013, SC-018-015, FR-018-046 and FR-018-049 agree with the demo's counts, 53 = 32 PASS + 12 DOC FIXED + 3 DEFECT
FIXED + 0 FAILED + 6 SKIPPED by `grep`). Gates rerun whole on the final tree: `cargo fmt --check`, clippy of the workspace with `-D
warnings`, `check-architecture.py` (with its new rule), `check-agent-config.py`, `check-design-docs.py`, the 365 Python tests of
`scripts/tests` (OK, 2 skipped), the workspace tests outside `sagat-tests` (exit 0), `integration_schedules_bulk_operations` 638 (1
ignored), `fault_schedules_bulk_operations` 99 and `integration_platform_foundation` 104 in **both roles**,
`integration_events_webhooks_messaging` 112, `fault_events_webhooks_messaging` 8, `integration_workflow_primitives` 764 (22 ignored),
the contract suites (schedules 8, developer experience 8, REST security 21, payload classification 18, observability 11, Iceberg 63,
AsyncAPI 5, REST events 2, REST durable execution 2), the Studio gates (1,464 vitest tests) and its browser journeys (52 passed). Not rerun
because not touched: the six SDK suites and the conformance suite (the review changed no SDK file).

## (i) The test-only gaps of four criteria (2026-11-10, D2156 to D2160, G2072, G2073)

What it closes: the tests that rows SC-018-001, -002, -009 and -011 said were never written. The tree was `c8289d97` plus the
uncommitted changes of this slice (tests, one clause of `crates/domain/src/execution/reset.rs`, the contract row of
`contracts/journal-and-events.md`, this feature's documents); nothing was pushed, fetched or committed. The private target
`/Users/marcelo/cargo-target-018`, `RUST_TEST_THREADS=2`, Docker Desktop. The full output of every run was kept.

| Command | Result |
| --- | --- |
| `cargo test -p sagat-domain --test reset --locked` | 21 passed (two of them red first: the finish of a collection offered as a point) |
| `integration_schedules_bulk_operations` as the owner / as `sagat_app` (`SAGAT_TEST_AS_RUNTIME_ROLE=1`) | 655 passed, 2 ignored (the child process of `calendar_processes`, and the measurement of `trigger_reconcile` that was ignored before), 428 s / 441 s |
| `fault_schedules_bulk_operations` as the owner / as `sagat_app` | 112 passed, 160 s / 158 s |
| `integration_platform_foundation`, both roles (the runtime-privilege audit) | 104 passed each |
| `integration_security_multitenancy`, both roles | 126 passed each |
| `contract_schedules_bulk_operations` | 8 passed |
| `cargo test --workspace --exclude sagat-tests --locked --no-fail-fast` | 2,958 passed, 7 ignored, 0 failed (236 result lines), 186 s |
| `cargo fmt --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `check-architecture.py`, `check-agent-config.py`, `check-design-docs.py` (pull-request mode against `develop`) | clean |

The new tests and what each costs: `reset_corpus::` 12 tests, 12 s with two threads (the approval one 3 s); `calendar_processes::` 5
tests, 0.3 s; `firing_decisions::` 12 tests, 12 s; `batch_thousand::` one test, **19 s as the owner and 20 s as the runtime role**
(it starts 1,000 runs through the service, 8 s of it). None is over a minute. Hand mutants, each run once and each red: the
open-work block and the excluded-event block of `reset_points`, a fresh key for each attempt of an item, a selection that does not
wait for the horizon, the audit record of the auto-pause in a second transaction, a jitter seeded with `$TZ`; the removal of the fix
of D2156 is the red the new tests were seen in first. Not run as a mutant: the refusal of a stale replica in `batch_thousand`. Not rerun because nothing they read changed: `integration_workflow_primitives`,
`integration_durable_execution` (the engine and the run store were not touched), the SDK suites, the conformance suite, the Studio.

## (j) The business-id prefix slice (2026-11-10, FR-018-016, G2027, D2171 to D2183, G2081 to G2090)

What it closes: a schedule's action may carry a business-id prefix with the two policies (FR-018-016; T018-016 but for the input schema, G1955), and a
batch job may select runs by the prefix of their business id (G2027; T018-045). The tree was `5426ea6c` plus the uncommitted changes of this slice;
nothing was pushed, fetched, committed, merged or rebased. The private target `/Users/marcelo/cargo-target-018`, `RUST_TEST_THREADS=2`, the Colima
daemon of this project reached through a forwarded socket (the platform's own PostgreSQL containers, started by the tests). The full output of every
run was kept.

**Red first.** Each part's tests were seen failing for the right reason before the code that makes them pass: `scheduled_business_ids::` 11 of 12
failed with `NotAllowed("business identifiers on schedules are not available yet")` (the twelfth is the control, a schedule without a prefix);
`batch_business_id_selector::` 7 of 8 failed with `BATCH_SELECTOR_UNSUPPORTED` (the eighth, the plan test, failed first on its own setup, the copy of
runs, and was fixed before the code existed); `rest::a_prefix_that_is_not_one…` failed on the field (`/action` where `/action/businessIdPrefix` is
promised) and was green once the codec named it; the two defects the independent reviews found (D2181, D2182) each have a test seen failing before the
fix (`a_revocation_that_commits_after_the_action_read_the_membership_stops_an_action_whose_id_is_held_too`: `BUSINESS_ID_CONFLICT` where
`AUTHORITY_REVOKED` is due; `a_waiting_action_that_finds_its_id_held_gives_back_the_action_it_kept…`: `next_action_at` stayed `NULL`). **Not red first, said
plainly:** the codec's unit test and the CLI's tests were written after the code (each guard was then shown red by a hand mutant, below), the REST
test of the whole path (`rest::an_action_under_a_business_id…`) was written after it worked, and the six SDKs and the Studio were done by agents, whose
red runs are in their reports (every one of them reports a failing first run for the part it added).

| Command | Result |
| --- | --- |
| `cargo fmt --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean on the final tree (the first fmt check of the gate script found six lines of formatting in four files, formatted, and the check run again) |
| `check-architecture.py`, `check-agent-config.py`, `check-upgrade-boundary.py`, `check-design-docs.py` (pull-request mode against `develop`); `python3 -m unittest discover -s scripts/tests` | clean; 365 tests, 2 skipped |
| `cargo test --workspace --exclude sagat-tests --locked --no-fail-fast` | 2,964 passed, 7 ignored, 0 failed (236 result lines) |
| `integration_schedules_bulk_operations`, as the owner | 683 passed, 2 ignored, 464 s; **one failure, `trigger_followup::a_buffered_run_now_of_a_trigger_owned_schedule…`** (3 runs where 2 are expected): it passed alone at once (`trigger_followup::` 20 passed, 17 s). It is a clock-of-the-day test: its cron trigger is `0 * * * *` (UTC) and it acts on the wall clock plus 30 s, and the run that failed was the one that crossed 11:00 UTC; I did not prove the cause by crossing an hour again, but nothing it reads was touched here |
| the same as `sagat_app` (`SAGAT_TEST_AS_RUNTIME_ROLE=1`) | 677 passed, 2 ignored, 496 s; **seven failures, all `No space left on device`** in the PostgreSQL of the test (the 30 GB disk of the Colima VM, 6.7 GB free at the start of the run: the same limit `integration_workflow_primitives` hit before): `migration::` (six) and `observability::customer_text…`. Rerun by name in a fresh process (`migration::` and `observability::`): 24 passed |
| `fault_schedules_bulk_operations`, both roles | 115 passed each (193 s as the owner) |
| `integration_platform_foundation` (the runtime-privilege audit) / `integration_security_multitenancy`, both roles | 104 / 126 passed each |
| `integration_events_webhooks_messaging` | 112 passed |
| `integration_workflow_primitives` | 747 passed, 22 ignored, **17 failed with `No space left on device`** in a 904 s run; the 17 rerun by name in a fresh process: 17 passed (25 s) |
| `contract_schedules_bulk_operations` | 8 passed (the first run of the gates: 7 and one failure, the sentence of the remediation of `SCHEDULE_SPEC_INVALID` in `errors.md`, which a test compares with the code's: fixed, 8 passed) |
| `contract_developer_experience` (blessed once after reading the diff, then without the switch) / `contract_rest_security_multitenancy` / `contract_payload_classification` / `contract_observability_operations` / `contract_iceberg_writes` | 8 / 21 / 18 / 11 / 63 passed; the digests of the iceberg-writes baseline did not move |
| `fault_platform_foundation` / `e2e_platform_foundation` / `e2e_developer_experience` / `e2e_sdk_multilanguage` / `fault_sdk_multilanguage` / `e2e_database_connections` | 38 / 42 / 32 (5 ignored) / 1 (6 ignored) / 1 (9 ignored) / 0 (1 ignored) passed: the targets `select-test-targets.py` names for the changed files, beside the ones the brief lists |
| the browser suite as CI runs it once (`cargo test -p sagat-tests --test e2e_workflow_studio --locked -- --ignored journeys`) | 54 passed, 2 skipped (Playwright's summary does not name them; neither is a journey of this slice), 261 s: the two new journeys, `SCHEDULES the run a schedule makes carries the business id of its prefix…` and `BATCH a job made from the business id filter of the list…`, ran and passed (G2088 closed) |
| the Studio gates (`frontend/AGENTS.md`): `npm audit --audit-level=moderate`, `npm run lint`, `npm run typecheck`, `npm run typecheck --workspace @sagat/e2e`, `npm test`, `npm run build`, `npm run check:api` | all exit 0; `npm test`: 99 files, 1,513 tests passed |
| the six SDK checks (`sdks/AGENTS.md`): TypeScript `npm test`, Go `go test -race ./...`, `go vet`, `gofmt -l`, Python pytest, mypy, `ruff check` and `ruff format --check`, .NET `dotnet test` and `dotnet format --verify-no-changes`, Java `./mvnw -q verify`, Rust `cargo fmt --check`, `clippy -D warnings` and `cargo test --locked` (the SDK; its conformance crate too, by the agent) | all exit 0: TypeScript 168 passed, Python 391, .NET 523; the Go packages report `(cached)` (the agent's uncached run: 91 tests in the two packages it touched) |
| the whole conformance suite, six languages by 76 scenarios (`cargo test -p sagat-tests --locked --test sdk_conformance -- --ignored --nocapture`, no bless switch) | **69 scenarios passed in the full run, 1,735 s; it stopped at the 70th, `switch-builders`, on `migrate: … No space left on device` (the Colima VM disk again, the server of the scenario could not start)**; that scenario and the six after it (`update-with-start`, `worker-registration`, `worker-shutdown`, `worker-stream-faults`, `worker-transport-faults`, `workflow-instance-limits`) were each run alone in a fresh process (`SAGAT_SDK_SCENARIO=<name>`): 7 of 7 passed, 24 to 33 s each. So all 76 passed, in two runs, not in one |

**What the new tests cost** (two test threads): `scheduled_business_ids::` 16 tests, about 11 s; `batch_business_id_selector::` 8 tests, about 16 s (the plan test, which copies a run 6,000 times and `ANALYZE`s, 4.7 s); `rest::` the four new tests 3 to 4 s each; `privacy::` the new test 2.4 s; the crash matrices `firing::` (seven tests, the two new steps among them) 11 s and `slot_order::` (thirteen) 11 s. None is over a minute.

**Goldens and baselines touched.** `tests/contract/sdk-conformance/expected/schedules-operate.json` (+221 lines: seven new observations, the run the trigger-now made in `serverState`) and `expected/batch-operations.json` (the observation `business-prefix`, which was the refusal `BATCH_SELECTOR_UNSUPPORTED`, is now a preview that counts the two runs under the prefix; thirteen new ones; `query-selector` only changes its trace placeholder number): each blessed from TypeScript into a scratch copy, read observation by observation (`golden_diff.py`: no existing observation changed but those two), applied, and the six languages played both scenarios **without** the bless switch, ok. `docs/reference/cli.md` (the usage of the two commands: seven lines) blessed after reading the diff; `docs/reference/api.md` did not change (no operation changed, only schemas and descriptions); `frontend/apps/studio/src/api/schema.d.ts` regenerated (`npm run check:api` clean); `tests/contract/iceberg-writes/fixtures/openapi-operations.json`: unchanged (`contract_iceberg_writes` passes as is).

**Hand mutants** (one per new guard, each alone, the named tests run, the file restored byte for byte; `mutants_prefix.py`):

| Mutant | Result |
| --- | --- |
| a refused action uses up a remaining action (`record_held` keeps the decremented count) | red: `an_open_holder_under_fail…` |
| a waiting action is not settled in place (`settle_waiting` answers false) | red: `a_waiting_action_that_finds_its_id_held…` (both) |
| the run is not given its id (`business: None`) | red: `each_run_a_schedule_starts_is_bound…` |
| `USE_EXISTING` recorded as a conflict | red: `use_existing…` (two tests) |
| the end of the prefix range is the prefix itself (no step up) | red: `a_prefix_selects_the_runs…` |
| the selector's prefix is ignored | red: `a_job_by_prefix_takes_its_runs_in_pages…` |
| a policy without a prefix is dropped, not refused | red: `rest::a_prefix_that_is_not_one…` |
| the prefix put in a log line | red: `privacy::the_prefix_of_a_schedules_action…` |
| the prefix put in a span of the export | red: `observability::customer_text_of_a_schedule…` |
| a held id answered as a refusal to retry | red: `an_open_holder_under_fail…` |
| the authority not looked at again when the id is held / refused | red, both: `a_revocation_that_commits_after_the_action_read…` |
| the next time not given back to a schedule whose waiting action was settled | red: `a_waiting_action_that_finds_its_id_held_gives_back…` |
| a lock the runtime role may not take added to the exported preview statement | red: `runtime_privileges::every_statement_the_stores_issue…` (the audit reads the new statements). **A first probe, `FOR UPDATE` on `runs`, survived: the runtime role holds `UPDATE` on `runs`, so it was no violation; the probe was wrong, not the audit** |

**The independent reviews** (the correctness and the security reviewer of the repository, given the diff and the specification, not my conclusions): one Major each, both real, both shown by a test that failed first and fixed (D2181: the claim ended the transaction before the authority was looked at again, so a revocation that committed in that window was recorded as a decision about the id; D2182: a waiting action settled because its id was held left the schedule active with an action and no next time). Each reviewer's other claims were read and checked: that a schedule goes on acting for a revoked author while the id stays held is **false** (the pass reads the membership every step; a test says so); that the privilege audit might not read the exported statements is **false** (the mutant above); the remaining nits are in D2183 and G2089, G2090.

### (j.2) The third review of the slice (2026-11-10, D2186 to D2195, G2091 to G2093)

Three independent reviews (security, correctness, architecture) read the tree at the end of (j). Same environment: the private target, `RUST_TEST_THREADS=2`,
the Colima daemon of this project; nothing pushed, fetched, committed, merged or rebased; the full output of every run kept.

| Finding | What was done | Red first |
| --- | --- | --- |
| **A. (Major, correctness)** a waiting `TRIGGERED` or `BACKFILL` action left the schedule `ACTIVE` with `remaining > 0` and no next time, started or settled | D2190: the reservation of a remaining action counts the waiting **scheduled** rows only (`Standing.reserved`, `waiting_scheduled`); G1017 closed. **The started half is older than this slice** (G1017 described it as "a time early"; nothing ever gave the time back); the held half is the same cause, which the give-back of D2182 (guarded by `kind == Scheduled`) did not reach | `waiting_reservation::` five tests, all red with `(Active, Some(1), 0, None)` where `Some(14:00)` is due: a trigger-now and a backfill step, each started and settled, and the due time decided |
| B1. which revision a waiting action takes | D2186: the head's (as the workflow, version, input and overlap always were); the row keeps its revision; runbook says so | a characterization test of a rule that held: no code changed, so it could not be red |
| B2. `a_time_that_is_acted_on_again…` cannot show the claim/replay order | D2193: it cannot, **and no test can**: with the claim first the decision is refused by the identity of the action's row with the same answer and state. Renamed to what it proves, strengthened (counters, the one row) | the hand mutant (replay skipped when there is an id) **survived** the old test and every other one |
| B3. edge prefixes through the job's items and the run list | one test compares the list, the preview and the items of a job on 29 ids at the edges of the range, for 20 prefixes (`%`, `_`, `\`, `~`, `~~`, `\|`, `}`, made of them, ending in them) | green at once (nothing was wrong); the hand mutant (range end of the selection statement alone inclusive) is caught by it and by no other test |
| B4. tests | `use_existing_uses_no_remaining_action_up`; `two_waiting_actions_whose_ids_are_held_are_settled_one_after_the_other` (found that the second is released five seconds after the first: written so); `use_existing_names_the_current_segment_of_a_chain_that_holds_the_id`; `a_recovery_in_mode_all_settles_…` and `a_recovery_in_mode_one_whose_earliest_time_is_held_starts_nothing`; `every_role_that_reads_schedules_also_reads_runs`; the `Debug` of `FilterBody` and of `TargetParts` | the role test and the two `Debug` tests each red against a hand mutant; the rest characterize behaviour that held (G2090 closed) |
| B5. the Studio dropped spaces alone as "no prefix" | D2195: an error on the field (`businessPrefixProblem`, one function for the field and the request) | `model.test.ts` and `ScheduleFormPage.test.tsx`: 2 failed before the change, the two tests that asserted the silent drop replaced |
| **C1. (Major, security)** a wide prefix costs pages times matches; a round over ten seconds is retried for ever | (a) the preview no longer sorts (D2191); (b) read, not assumed: no failure counter on `batch_jobs`, `SELECTION_FAILED` raised nowhere, for every selector; a bound needs a migration, so **none built** (D2192, G2093); (c) keyset paging recorded in G2089 | the plan assertion (no `Sort` in the preview) failed on the ordered statement: `Limit -> Sort -> Index Scan on runs_business_id_idx` |
| C2. the selector in the idempotency fingerprint | D2187: checked; the batch create and a person's start keep a SHA-256 covering the text, the schedule write keeps none; the existing rule, not changed | |
| D1. FR-018-044 against D2180 | no audit action added; G2086 says the reviewer found the text unmet and that the choice is the owner's | |
| D2, D3, D6, D8 | the lock-order table of `plan.md` and D2188; the order of the upgrade (engines first, G2091), the `422` for a policy with no prefix, in the schedules runbook, `sdks/RELEASE_NOTES.md` and `docs/releases/unreleased.md`; the exception to "one decision per time" (D2189) | |
| D4. a batch twin of the privacy canary | `privacy::the_prefix_of_a_batch_selector_reaches_only_the_job_that_holds_it`: a preview, a job to its end, the reads, a replay; every table but `batch_jobs`, `business_ids` and `runs`; logs, spans, metrics | green at once; red when `select_round` logs the selector (a hand mutant) |
| D5, D7 | `TargetParts` hides the prefix in `Debug`; `#[doc(hidden)]` on the exported statements; the CLI comment says its length check is a convenience. **Left:** the clone of the admission per firing, one source for the literal `BUSINESS_ID_IN_USE` (D2194) | |

**Hand mutants of this block** (each alone, the file restored byte for byte): the old reservation, every waiting row counted: red, all five of `waiting_reservation::`; the range end of
`SELECTION_BY_PREFIX_SQL` made inclusive: red, the edge test only (the eight other tests of the file green); the preview ordered again: red, the plan test; `TargetParts` with a derived
`Debug`: red; `FilterBody`'s `Debug` printing the prefix: red; `viewer` without `run:read`: red; `select_round` logging the selector: red, the batch canary; the replay skipped before the claim:
**survived, by design** (D2193).

| Command | Result |
| --- | --- |
| `cargo fmt --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `check-architecture.py`, `check-agent-config.py`, `check-upgrade-boundary.py`, `check-design-docs.py` (pull-request mode) | clean |
| `cargo test --workspace --exclude sagat-tests --locked --no-fail-fast` | 2,968 passed, 7 ignored, 0 failed (236 result lines) |
| `integration_schedules_bulk_operations` as the owner | 695 passed, 2 ignored, 476 s; **2 failures, both `No space left on device`** (`limits::ten_exclusions…`, `limits::ten_spec_entries…`): rerun by name in a fresh process, 2 passed |
| the same as `sagat_app` | 690 passed, 2 ignored, 469 s; **7 failures, all `No space left on device`** (`migration::` six, `observability::customer_text…`): rerun by name in a fresh process (`migration::` and that test), 22 passed |
| `fault_schedules_bulk_operations`, both roles | 115 passed each (177 s) |
| `integration_platform_foundation` (the runtime-privilege audit reads the new count statement), both roles | 104 passed each |
| `contract_schedules_bulk_operations` | 8 passed |
| the Studio gates (`npm audit`, lint, typecheck, typecheck of the e2e workspace, `npm test`, build, `check:api`) | all exit 0; `npm test`: 99 files, 1,515 tests |
| conformance, `batch-operations` and `schedules-operate`, six languages each, no bless (no golden and no SDK file changed here; the server statement of the preview did) | ok, 46 s and 26 s |

What the new tests cost (two threads): `waiting_reservation::` five tests, 4.3 s; the 22 tests of `scheduled_business_ids::`, 14 s (the chain test alone 2 s); the edge corpus test 3.5 s;
the batch canary 2.2 s. None is over a minute. **Not run:** the full conformance suite (the coordinator's), the mutation battery (OD-2026-11-06-2), `integration_workflow_primitives` and
`integration_security_multitenancy` (no migration and no statement they read changed).

**What is not verified, and why.** (1) The input of a schedule is still not checked against the schema of the target (G1955, G2081): the platform
enforces `inputSchema` nowhere, so it is the owner's answer; `NOMINAL_TIME_FIELD_INVALID` is still never answered. (2) The tests of G2090 were written
in (j.2) and the gap is closed. (3) G2082 to G2087 and G2089 are registered, not closed: no SDK exposes `details.field`; Python, Go and C# print the schedule input and Java and
C# `Run` print `businessId`; the Studio's batch page cannot show the prefix; the reuse window is the default 30 days; a business-id conflict is not
audited; ids are predictable and two writers of one id meet by design. (4) The demo of the quickstart on the Compose stack was not run for this slice:
the proof is the test suites above, the conformance scenarios and the browser journeys, which start the platform themselves. (5) The Colima VM disk
(30 GB) filled during the long suites (`integration_workflow_primitives`, the run of `integration_schedules_bulk_operations` as `sagat_app`, the
conformance suite): the failures were `No space left on device`, each rerun by name or by scenario in a fresh process and passed; a full green run of
those suites in one process was not obtained.
(6) `trigger_followup::a_buffered_run_now…` failed once in the full run and passed alone; the cause was found by the coordinator (the hourly cron
firing inside the 31 s the test moves its clock) and the test changed by them; it is not touched here. (7) The six
SDK passes changed two build files that the brief did not name: the Java `pom.xml` (a JUnit test dependency of the conformance module) and the C#
test project file; `git status` lists them. (8) Mutation testing was not run (OD-2026-11-06-2); only the hand mutants above.

## Appendix: the 257 waits of step 6.8

```sh
python3 many.py 257 > many/workflow.json     # N sequential waitEvent nodes w1..wN on the event "tick", the first awaited from the start
sagat workflow publish many/workflow.json
sagat workflow run many-ticks --input '{"k":"1"}' --business-id many-1
# 257 times: POST /runs/<run>/signals/tick, trying again (up to 200 times, 50 ms apart) while the next wait has not begun
```

```python
import json, sys
n = int(sys.argv[1])
nodes = []
for i in range(1, n + 1):
    node = {"id": "w%d" % i, "type": "waitEvent", "eventType": "tick", "correlationExpr": "input.k", "timeoutMs": 3600000}
    if i < n:
        node["next"] = "w%d" % (i + 1)
    nodes.append(node)
print(json.dumps({"apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
                  "metadata": {"name": "many-ticks", "description": "Waits for %d ticks, one after the other." % n},
                  "entrypoint": "w1", "triggers": [{"kind": "manual"}],
                  "inputSchema": {"type": "object", "properties": {"k": {"type": "string"}}}, "nodes": nodes}, indent=1))
```

The run finished in 19 s (`SUCCEEDED`, 257 signals in the journal); `sagat workflow reset-points <run>` lists only the start of the run
(a wait is not a point), and the preview and the reset of point 1 with `--reapply signals` are `RESET_REAPPLY_TOO_MANY`.
