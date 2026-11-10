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
| 2.4 | each run has `schedule.nominalTime` (exact minute) and `actualTime` | **FAILED** | the run resource (`GET /runs/{id}`, `GET /runs?scheduleId=`) has `startedBy` and nothing else of the schedule: no nominal time, kind or revision (FR-018-016 says runs "expose schedule id, nominal time, kind and revision"; the columns exist and are never read for a response). The **action** has them (`nominalTime: …23:24:00Z`, `actualTime: …23:24:04.040211Z`: 4.04 s after, inside the 5 s of jitter). **G2057**; the page says where to read them |
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

53 steps: **32 PASS, 12 DOC FIXED, 2 DEFECT FIXED, 1 FAILED, 6 SKIPPED** (`2.4` failed; `1.1`, `6.12`, `6.13`, `6.16`, `6.17` and `6.19b`
were skipped). The two defects (2.11, 6.5) were found by the demo, fixed with a test seen failing first, and rerun; the failure
(2.4) is a gap (G2057) that the page states plainly and that this task does not fix, because the fix crosses the OpenAPI, the
generated reference, the Studio's types and six SDKs.

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
| **SC-018-001** at least 200 table cases over every spec kind, zone and DST case, equal across two engine processes | PARTLY | `cargo test -p sagat-infra-ir --test schedule_calendar --locked`: 27 tests, among them 730 daily occurrences for each of `Europe/Lisbon` and `America/New_York` (1,460 cases) against an oracle written from the zones' rules (T018-013); the three spec kinds, jitter and the first times in `schedules::` scenario 1 and the domain suites `schedules`, `schedules_properties`; this demo: an interval with jitter acted at nominal times on the exact minute with 4.04 s of jitter | no test computes the same instants in two separate **processes** and compares them: the equality rests on the calculation being a pure function of the schedule id (SHA-256 seed) and on `pass::two_replicas_split_the_schedules_by_shard_and_every_time_starts_one_run` and `fault … clocks.rs` (skewed clocks); the cron grammar and calendars are covered by the table only in the two zones |
| **SC-018-002** crash at each statement of the firing transaction, a stale-fence replica and a duplicate pass give one run and one action per nominal time, with `startedBy` and provenance | PARTLY | `fault … firing::`: the start (28 writing statements on 13 tables) and the skip (3 writes) are killed after each write: nothing changes, the next pass acts once, `verify_run` clean (T018-020); `admission::` (the same time never starts a second run; the stale fence; the three lease tests); `pass::` two replicas; this demo 6.1 | (1) the other decision-only steps (buffer, coalesce, drop, reconcile, auto-pause, backfill step) are not killed at each write (G1018, open); (2) **the provenance is stored and never exposed**: the run answer has `startedBy` only (G2057, demo step 2.4 FAILED) |
| **SC-018-003** every overlap, catch-up, pause/resume, trigger-now, backfill, remaining-actions, pause-on-failure and revision/delete scenario of US02 records the expected outcome table | VERIFIED | `firing::` (skip, buffer one, buffer all, allow all, two processes one run), `cancel_other::` (7) and `fault … cancel_other`, `catchup::` (8, plus the trigger-owned 2), `backfill::` (11), `trigger::` (9), `changes::`/`service::`/`rest::` (pause, resume, revisions, delete), `firing::a_schedule_with_two_actions_left…` (remaining actions, pause on failure); this demo: 2.5 to 2.9, 6.2, 6.3 on the real stack | `TRACKING_LIMIT` is proved in the domain only (T018-026); T018-029 stays unticked only for the review of the trigger-owned part |
| **SC-018-004** revoking the authorizing subject pauses the schedule at its next action; no run starts | VERIFIED | `authority::` (6 tests; a revocation that commits between the read and the admission stops the run, found by a test that failed first); this demo 6.3: `AUTHORITY_REVOKED`, schedule `PAUSED`, system note, audit `schedule.auto_paused`, no run | none |
| **SC-018-005** rolling upgrade with old and new engines on one cron trigger: one run per tick and the 004 input; pause refused before the handover and honoured after; a trigger published by an old control plane gets its schedule from reconciliation | PARTLY | `trigger_firing::` (both orders of the two passes, `ALREADY_ACTED`, the key `cron-<epoch>`, the `cron.fired` document, `system:engine`), `fault … upgrade::` (each pass killed after every write; four sequences; an old binary after the handover), `trigger_handover::` (11), `trigger_schedules::pause_resume_trigger_and_backfill_are_refused_until_the_handover…`, `trigger_reconcile::` (8); this demo 6.14, 6.15, 6.18, 6.19a | the "old" engine is the legacy pass run in the same binary, and the old control plane is a row written by the test: no released binary was run (the released-upgrade profile is not run locally and was skipped by CI); T018-060 and T018-061 are built and **unticked, review pending** |
| **SC-018-006** 32 concurrent starts on one id give one open run; every reuse/conflict policy matches its table inside and after the window; a chain keeps the binding until final | VERIFIED | `business_ids::` (32 starts: one open run; 32 over a closed holder: one new run; 32 with `USE_EXISTING`: one run; the policy table inside and after the window; chain continuity; the terminal-commit race) and `fault … business_ids` (T018-036, ticked); this demo 3.1, 3.2 | none |
| **SC-018-007** delayed runs release once, are cancellable and refuse signals; a crash around release leaves one release; with 022 run-level concurrency the release acquires the permit or queues the run | PARTLY | `delay::` integration (9) and fault (kill after each writing statement of the release; the next pass releases once), the race with a run that stopped qualifying (T018-037); this demo 3.4: `WAITING`, signal `409`, one `run.start_released` | the 022 clause is **BLOCKED** (feature 022 not approved: T018-037, T018-053); a declared Update on a held run: closed by `update_with_start::a_delayed_run_admits_the_update` (G1972) |
| **SC-018-008** the four Signal-with-Start and three Update-with-Start cases of US04 pass, plus a Signal-with-Start whose new run 022 would queue; no signal is stored without a waiting step | PARTLY | `with_start::` (8), `with_start_rest::` (5), `update_with_start::`, `update_with_start_rest::` (T018-040, -041, -042); this demo 4.1, 4.2, 6.4 | the 022 case is **BLOCKED** (G1973 (2)) and the race of a wait's timeout against the delivery at creation has no test (G1973 (3)); T018-040 unticked for that |
| **SC-018-009** a 1,000-run job with an engine kill, a lost broker hint and a cancellation applies each run at most once, counters equal items, a run whose creating transaction was open at creation is selected, and a lock monitor never sees a batch transaction with two run locks | PARTLY | `batch_executor::` (12: pages, the wait for an open transaction, a crash between the effect and its record), `fault … batch` (7: a kill after each write of the five transactions of the executor; a cancel racing an item), `batch_pass::`, `batch_locks::a_batch_job_and_the_api_on_the_same_runs_never_hold_two_run_locks_or_deadlock` (24 runs, the monitor looked 1,157 times, never two run locks, no deadlock); the benchmark: a selection of 10,000 runs in 0.3 s, 16 jobs shared fairly; this demo 5.1 to 5.3, 6.5 | no test runs **one 1,000-run job** through a kill and a cancellation (the tests use a handful of runs, the benchmark 900 to 10,000 without a kill); there is no broker hint to lose (a job is found by the pass's poll); **at most once** is by the key `batch:{job}:{run}`, while the effect and the record of the item are two transactions (G1961, O1951 open) |
| **SC-018-010** missing `run:batch`, missing action permission (`deployment:manage` for `REPIN`), mid-job revocation, foreign run ids and `REPIN` without 019 are refused or skipped as specified | PARTLY | `batch::creating_a_job_needs_run_batch_and_every_permission_of_the_action` (the role matrix), `batch_executor::a_creator_who_loses_a_permission_fails_the_job…` (`AUTHORITY_REVOKED`), `batch_security::` (foreign ids), `batch::parameters_the_action_does_not_take…` (`REPIN` refused); `contract_rest_security_multitenancy` (T018-048, -070) | `deployment:manage` for `REPIN` is **BLOCKED** (T018-079 needs feature 019) |
| **SC-018-011** for every eligible point of a seeded corpus (a 022 permit holder, search-attribute upserts) the reset run's replay equals its projection, outputs and attribute rows equal the source, the 022 holder count equals the open non-restored acquisitions, a prefix with an event lacking a rule is refused, and ineligible, test-tree and purged sources are refused | PARTLY | `reset::at_every_point_the_run_that_is_made_replays_to_its_projection…` (all four points of a sequential run), `fault … reset` (a kill after each of 62 writes: nothing changes, the retry makes one run), `reset::` refusals (`RESET_RULE_MISSING` with the type named, `RESET_NOT_SUPPORTED_FOR_SOURCE` for a child and a chain member), the benchmark (replay equals projection for 45 runs up to 5,000 restored events); this demo 5.4, 5.5, 6.6 | the corpus lacks parallel, saga, collection, loop, child and wait shapes at every point and approvals (G1979 asks the owner); the 022 permit holder and 020 attribute rows, `RUN_PURGED` and test-tree refusals are **BLOCKED** (022 and 020 not approved: T018-053, -055, -077); the database-execution branch of `RESET_NOT_SUPPORTED_FOR_SOURCE` has no test (G1980 (5)) |
| **SC-018-012** `CANCEL_SOURCE` racing source completion, a crash during the reset transaction, reapply order and `NOT_REAPPLIED` behave as specified; the source never reopens | VERIFIED | `cancel_source::` (the pending reset, the races, the fence), `fault … cancel_source` and `cancel_source_expiry`, `cancel_source_rest::` (the engine with the flag makes the run; a step in flight needs the acknowledgement), `reapply::` (order, the first undeliverable stops the rest, `LEGACY_EVENT`, the ceiling of 256, key replay) and `fault … reset`; this demo 5.5, 6.7 to 6.11 | built and **unticked** (T018-056, T018-057: left for the maintainer to tick after reading them, reviews done); a failed source compensation is in `cancel_source::` |
| **SC-018-013** OpenAPI additive, AsyncAPI baseline only grows, IR and worker baselines unchanged, reasons documented; five SDKs pass the shared scenarios | PARTLY | OpenAPI: 27 operations marked `x-implemented: '018'`, `contract_schedules_bulk_operations` (7), `docs/reference/api.md` blessed, the cross-tenant battery over every one; IR: the one additive, capability-gated field `reapplyOnReset` (T018-078); worker protocol unchanged; reasons: `contracts/errors.md`, `docs/runbooks/*`; SDKs: `sdk_conformance` over the whole suite, 75 scenarios in six languages, none skipped, 1,853 s (T018-068, 2026-11-06) and the CI job **sdks** green on `345dd70e` | **the AsyncAPI additions (`scheduleId`, `nominalTime`) were never made and the baseline never regenerated** (T018-010 stays open: its note says "Not done: the AsyncAPI additions and the baseline regeneration"); I did not rerun the conformance suite |
| **SC-018-014** canary schedule inputs, batch payloads and business ids are absent from logs, spans, metrics, audit and outbox; each new quota refuses before work; the real OTel export has fixed labels only | PARTLY | `observability::customer_text_…` (the real exporter to a collector that keeps bodies: eight canaries, every log line, the Prometheus page; the search is shown able to find a planted canary), `privacy::` (a business id through every table, log and metric), `batch::` (a signal payload is in no job row and no audit record), `disclosure::`, `observability::every_ceiling_…` (five quotas: `429`, no table changed), `counted::` (label closure), `audit_rollback::` (8) (T018-071, -039, -033, 2026-11-09); this demo 2.10 (the note is not in the audit) | a batch **signal payload** is checked in the tables, not in the export, the logs or the outbox; the audit rollback does not cover the firing, the backfill, trigger-now, `revoke_authority` or the item audit (G2031); G2028, D2085 are the agent's decisions the owner has not accepted; T018-071 unticked on that |
| **SC-018-015** a real browser runs the schedule, batch and reset flows keyboard-only with WCAG AA contrast and table alternatives; the quickstart demo runs | PARTLY | fourteen keyboard-only journeys (`frontend/e2e/tests`: schedules 4, batch 4, reset 4, business id and delay 2), axe `wcag2a/2aa/21a/21aa/22aa` on the list, the form, the dialog, the error and the empty state; three runs in a row locally (14 of 14) and one run on Linux in CI (job **studio** on `345dd70e`); **the quickstart demo** is this task: run for real with one step FAILED (2.4), two defects fixed and four rows skipped | T018-074 is narrowed pending the owner (O1952): no journey for the preview or the cancel of a backfill (G2038), and the failure states of G2034/G2035; only one Linux run; the demo's outputs are not a person's first day (SC-012-001 is separate) |
| **SC-018-016** a barrier test runs every run-driven transition against each aggregate path; `pg_locks` never shows a run-driven transaction holding a schedule, business-id, job, item, obligation or pending-reset row, and no deadlock occurs | VERIFIED | `fault … run_driven_locks::` (ticked, T018-076): an `ACCESS EXCLUSIVE` lock on every table of the aggregates, found by name, under which a completion, a failure, a lease that runs out, a deadline, a cancellation, a timer, a wait timeout and a hand-off must still finish within 60 s; the controls show the firing and the reset request do wait; hand mutants (an unconditional and a conditional read) turn it red; `batch_locks::` reads `pgrowlocks` and `pg_stat_database.deadlocks` | the proof is by blocking the tables, not by sampling `pg_locks` of the run-driven transaction, as the task's own note says (a stronger form of the same sentence) |
| **SC-018-017** an activation racing a registering instance never leaves it serving; an instance lacking the capability answers `BINARY_CAPABILITY_MISSING` or stops its passes; an instance silent for 30 s no longer blocks an activation | VERIFIED | `instances::` (33), `instances_hostile` (7), `instances_migration` (2), `instances_process` (11), `instances_rest` (7), `fault … instances` (the race in both orders held by a lock and watched in `pg_stat_activity`, the limit under 24 concurrent registrations); this demo 6.18 (the engine without the capability is named and blocks the activation; once stopped it no longer does) | built and **unticked, review pending** (T018-080); the flags of batches and resets are not on the registry (G1999) and what a holder of the runtime role can still do is G2000 |
| **FR-018-046** contracts and compatibility | PARTLY | REST additive under `/v1` (`contract_schedules_bulk_operations`); the worker protocol unchanged (`worker.baseline.json`); the IR gains only `reapplyOnReset`; existing starts, signals, reruns, cron triggers and SDK clients unchanged: the legacy pass and the SDK conformance on `345dd70e`; this demo: the cron trigger fired as before until the handover | AsyncAPI gains no `scheduleId` or `nominalTime` (T018-010); the released-upgrade profile (an older release against the new schema and back) was not run here and CI skipped it on the pull request |
| **FR-018-049** delivery: runbooks, alert rules for late schedules and stalled batches, trigger-owned migration notes, the reproducible demo | PARTLY | `docs/runbooks/{schedules,batch-operations,run-reset,business-ids,instance-registry}.md` (with the capacity and the handover procedure), alert rules `SagatSchedulerLate` (carries the schedule and batch lag, summary names them) and `SagatBatchJobsStalled` with `promtool test rules` (`batch_stalled_test.yaml`, in CI); `infra/observability/tests/instance_registry_test.yaml` run by hand only; **the demo of `quickstart.md`**: rewritten from a real run (this document) | no operator has reviewed the runbooks (T018-073 unticked); a separate `SagatSchedulesLate` was never made; the demo has one FAILED step and four SKIPPED rows; the four lines are not in `.env.example` (G2061); the quickstart's snippets were replayed one by one, not the whole page in one go |
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
| T018-010 | **NOT DONE in part** | the AsyncAPI additions and the baseline regeneration | an intentional baseline regeneration (SC-018-013, FR-018-046) |
| T018-012 | built in part | the slot-order cases of the admission paths of 019, 021, 022, 024, 025, 026 and 030 exist only as those features merge (G2029) | features 019, 022 (not approved), 021, 024 to 026, 030 |
| T018-016 | built in part | the input is not checked against the schema of the target at write and at action (G1955), and the business-id prefix of a schedule's action is refused (`422 business identifiers on schedules are not available yet`: seen in this demo, so it is a stated refusal, not a silent no-op) | building the schema check and the prefix (FR-018-016) |
| T018-029 | built | trigger-owned catch-up done (D2040); unticked with T018-060/-061 until their review | the reviews |
| T018-037 | built, **022 BLOCKED** | the release does not acquire 022's run permit | feature 022 approved and merged |
| T018-040 | built, **022 BLOCKED** | the new run 022 would queue; the timeout race (G1973) | feature 022; a test |
| T018-047 | built, **022 BLOCKED** | `TEST_RUN` skips need 022 | feature 022 |
| T018-051 | **NOT DONE / BLOCKED** | purge of jobs and items; the selector through 020's `RunSelector` | feature 020 approved and merged (G2027: a selector by `businessIdPrefix` is still refused) |
| T018-052, -053, -054, -055 | built in part | an approval and a loop in the corpus (G1979, asks the owner); the 022 permit and the 020 attribute fold; scoped prefixes and a configurable ceiling (G1980); the database-execution branch has no test | owner decision, features 020 and 022 |
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
| `cargo fmt --check` | | | |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | | | |
| `check-design-docs.py`, `check-architecture.py`, `check-agent-config.py`, `check-upgrade-boundary.py` | | | |
| `cargo test --workspace --locked --no-fail-fast` (`RUST_TEST_THREADS=2`) | | | |
| `integration_schedules_bulk_operations` (owner and runtime role) | | | |
| `fault_schedules_bulk_operations` (owner and runtime role) | | | |
| `contract_schedules_bulk_operations`, `e2e_…`, `perf_…` | | | |
| `integration_platform_foundation`, `integration_security_multitenancy` (both roles) | | | |
| `contract_rest_security_multitenancy`, `contract_developer_experience`, `contract_observability_operations` | | | |
| `sdk_conformance` (all scenarios, six languages) | | | |
| the six SDKs' own checks | | | |
| Studio: lint, typecheck, `npm test`, build, `check:api` | | | |
| browser journeys (`e2e_workflow_studio … journeys`) | | | |
| `promtool check rules`, `test rules` | | | |
| `docker compose … config -q` | | | |
| the quickstart, end to end once more on a clean stack | | | |

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
