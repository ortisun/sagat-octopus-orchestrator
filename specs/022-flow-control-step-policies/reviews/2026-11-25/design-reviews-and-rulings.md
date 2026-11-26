# Reviews of the design of the rest of feature 022, and the coordinator's rulings (2026-11-25)

The three read-only repository reviewers (`architecture-reviewer`, `correctness-reviewer`, `security-reviewer`) read the
documents of commit `14b9e40b` (ADR-0135, ADR-0136, ADR-0137, decisions D022-200 to D022-221, the amendments of
`spec.md`, `plan.md`, `data-model.md`, `contracts/`, tasks T022-200 to T022-223) and the code they cite. They ran
nothing. Totals: architecture 2 Blockers, 8 Majors, 16 minors, 4 nits; correctness 1 Blocker, 9 Majors, 8 minors,
4 nits; security 1 Blocker, 4 Majors, 10 minors, 5 nits. No ADR is accepted as written; each is accepted once the
rulings below are written into it and the three reviewers find no Blocker in a re-review.

Every ruling is delegate (OD-2026-11-24-4): taken by the coordinating session for the most architectural and business
advantage, recorded as decisions in `research.md` (D022-222 upward) and written into the documents before any code.

## Blockers

| Id | Reviewer | Finding | Ruling |
|---|---|---|---|
| Architecture Blocker 1 | architecture | ADR-0137 decision 3 locks the draft workflow row `FOR SHARE` at slot 4b; publication locks `workflows FOR UPDATE` then the namespace's `lock_secrets` (`definitions.rs:618-642`, `quotas.rs:306-312`, `database/admission.rs:209`), a test admission of a draft with a managed database node takes `lock_secrets` at 4a then the row: a cycle. Version activation also locks the row first (`versioning.rs:534-537`) | The draft row is read without a lock and its `draft_revision` compared (a moved draft is `CONFLICT`); the private definition is an immutable copy. The handler workflow's head in T9 is a plain read too. A case in `slot_order.rs`: publication of a workflow with a database node against a test admission of its draft |
| Architecture Blocker 2 | architecture | The terminal commit inserts the dispatch row, a slot-3 aggregate, which ADR-0050 decisions 3 and 7 forbid on a terminal transition | A recorded deviation in ADR-0050 decision 7 with its no-cycle argument (the row is invisible until commit, its key is the run, serialized by the run lock), and a rule in `scripts/check-architecture.py`: only the terminal path of `persist` inserts into `failure_handler_dispatches`, only the dispatcher locks it |
| Correctness Blocker 1 (same as Architecture Major 4) | correctness, architecture | The handler phase cannot run under the reducer: a stopped run drains (`plan.rs:306-308`, `:523`, `:602-606`), a second stop is refused (`state.rs:4636-4639`, 409 at `runs.rs:3344`), a stopped run is `CANCELLING` (`state.rs:433-434`), `check_finish` refuses `run.finished` while a step is active or a non-`SUCCEEDED` status without a stop (`state.rs:5133-5163`) | The phase is its own reducer state: `failure_handler.started` moves the stop into `pending_terminal_status` and clears the stop; the run reads `RUNNING`/`WAITING` in the phase; a region step that fails without `onError` ends the phase `FAILED` and never opens a second phase; a cancellation or the run deadline during the phase is a handler cancel: a draining sub-state (`failure_handler_state` `TIMING_OUT` or `CANCELLING`) asks leased region attempts and the `onFailure~h` child to stop, and its last settle commits `failure_handler.finished`, the permit's release and `run.finished` with the pending status; the handler timer is conditional on the phase and cancelled by `failure_handler.finished`. Domain tests in `crates/domain/tests/failure_handler.rs`, one per case the reviewer lists (a gated region step, a region step failing without `onError`, a cancellation, the run deadline, the timeout with a leased attempt) |
| Security Blocker 1 (same as correctness minor 1) | security, correctness | A hit can hand out a result another publisher's secret produced: the fingerprint (`data-model.md:20-22`) and the entry row (`:533-547`) carry no producer secret, and the recheck (ADR-0135:39-41, FR-022-004) reads the consumer's declared secrets only | The fingerprint includes the producing node's sorted logical secret references; the entry records them; a hit requires the consumer's publisher to be able to use every one of them now (metadata only), else the outcome is `unauthorized`. Test: two publishers, one access list |

## Majors

| Id | Finding | Ruling |
|---|---|---|
| Security Major 1 | Shared scopes can be poisoned: the input mapping is not in the fingerprint; standalone steps may write `namespace` scope without publication review | The canonical input-mapping expression and the worker binding enter the fingerprint; runs of internal definitions (standalone, test trees) use only the `version` scope of their own private version |
| Security Major 2 | Draft test runs skip publication approvals outside `prod`; standalone steps exempt every activity and script from the four-eyes check, undecided | Test runs are refused in `prod`, and in a namespace whose effective `required_approvals > 0` unless the namespace's publication policy enables draft test runs (a namespace-admin setting, audited, off by default). Standalone activity and script steps follow the same rule (feature 017's database executions keep their accepted behaviour). Rejected: refusing all test runs where approvals are required (it removes the review use of draft tests the product sells) and allowing them silently (unreviewed code with real secrets) |
| Security Major 3, Correctness Major 6 | Handler recovery and handler descendants can loop without bound; "under its publisher's authority" is ambient authority | A run carries `handler_tree` (inherited by children and by resets and reruns whose initiator is a handler-tree run or whose source is one); a handler-tree run is never dispatched and never enters a workflow handler phase on behalf of a recovery; a namespace handler is dispatched at most once per lineage (the source chain of resets and reruns), by a unique index. No built-in reset authority in 0.3.0: a handler that recovers calls the public API with a machine identity it is given as a secret, audited as that identity; no row of the authorization table grants a run its publisher's operator rights |
| Security Major 4 | Cache keys are outside the key classification rules | FR-022-057 extends to `cache.key`: `CACHE_KEY_SECRET` refused, `pii` warned; the label rule applies to every surface that shows a cache key or a hit |
| Correctness Major 1 | A result computed before an invalidation is stored after it and served live | The entry is stamped with the producing completion's instant (the obligation's `created_at`) and expires from it; T4 inserts the scope row when absent and sets `invalidated_at = clock_timestamp()` after its lock; T5 also deletes pending obligations of the same scope and key digests. Barrier test of T3 against T4 and T5 |
| Correctness Major 2, Architecture Major 3 | T18 (erasure) and T3 (store) lose a delete and can deadlock | T18 locks the run's obligations first, then the entries by source run; the rule reads "after a cache row, only usage rows" (ADR-0135 decision 7 and the check of T022-211). Barrier test |
| Correctness Major 3 | A dead entry blocks a re-store until the purge | T3 deletes a conflicting dead entry under its lock, then inserts |
| Correctness Major 4 | The obligation's key digest has no defined source | The lookup's key digest and key version are kept with the miss (on the step projection or a row written with `step.cache_missed`); T2 copies them; only `miss`, `expired` and `invalidated` write an obligation |
| Correctness Major 5, Architecture minor 2 | A hit beyond the run's retained-output limit fails the commit and retries forever | The lookup round checks the payload's size against the run's retained bytes and fails the step `RUN_DATA_LIMIT` through its route, as `evaluate_run_result` does; the hit's bytes enter through the outputs path so charge, digest and projection come from one place |
| Correctness Major 7 | `handler_for` on an `onFailure~h` child is unspecified | `handler_kind` (`DISPATCHED`, `REGION_CHILD`); `handler_for_run_id` and its unique index for dispatched runs only |
| Correctness Major 8, Architecture Major 5 | The finish paths outside the reducer are listed incompletely (`primitives.rs:3495`, `:4026-4045`, `:4061`, `loop_progress.rs:778`, `:3409-3449`, `chain_progress.rs:2246-2248`) and decision 6 has no check | Every site listed in task T022-207; a rule in `scripts/check-architecture.py` refuses a `run.finished` journal or outbox write, a `finished_at =` or a direct `task.ready` outside `persist`, with an allow-listed `chain_progress` exception citing the `continueAsNew` refusals; T022-207 adds a loop and a collection that end `FAILED` with `onFailure` and with a namespace handler. Decision D022-207 gets its own short ADR, cross-referencing ADR-0030 and feature 014's plan (architecture minor 15) |
| Correctness Major 9 | Upstream pins would activate their successors | Upstream pins commit `step.routed` with no activation beside `step.mocked`; `startAt` is recorded in `run.started` so replay reaches the same entry. Domain test with A to X and A to B to C, `startAt = C` |
| Architecture Major 1 | The documented commit order is not the code's | The ADR-0056 sentence at line 54 is replaced, not contradicted: the transaction holds the run's row lock; a commit that ends the run locks its active-run counter row before it gives back any key; the `runs` row is written by `persist` after the rounds. The order is restated as built; a domain property test that no progress emits a filed gate decision and `run.finished` in one commit |
| Architecture Major 2 | No slot for cache rows at admission | ADR-0050 decision 7 slot 7 reads: run-scope semaphores and windows, then cache scope, then entry (`FOR SHARE`), then step-scope semaphores and windows |
| Architecture Major 6 | Slices are not safe to activate one at a time | A binary serves a capability only once its last behaviour slice has merged; a test per capability that it is not served before |
| Architecture Major 7 | Hot-path budget gaps | The handler policy is known in memory (the namespace's handler revision read with the run's definition, or a cached flag fenced by its revision), so a plain failing run's line does not move; the cache hit's content read is a budget line; `test_tree` immutability is a column privilege or `BEFORE UPDATE OF test_tree`, never a row trigger on every update |
| Architecture Major 8 | Behaviour at the pending-dispatch ceiling is undefined | The terminal commit always inserts its dispatch row; the ceiling refuses new handler configuration and raises an alert, never a terminal commit |

## minors and nits

All are accepted as the reviewers propose, written into the documents, except where noted:
- Architecture minor 1, correctness minor 2: if no path first-releases a root step while the run is paused, the paused
  exception of ADR-0135 decision 6 is removed; `step.cache_missed` joins the task-ready hint arms.
- Architecture minor 3, security minor 4: the secret-usability recheck calls the existing lock functions
  (`sagat_lock_secret_access`, `sagat_lock_memberships`) at a documented slot, as feature 017 fences admission; a
  plain read would leave a revocation window the platform does not accept elsewhere.
- Correctness minor 6: a run cancelled during its handler phase is not dispatched to the namespace handler (the
  operator decided); "qualifies" means no workflow handler phase ended `SUCCEEDED`.
- Security minor 1: `UPDATE (expires_at)` only, with the trigger that refuses updates kept; never a blanket grant.
- Security minor 2: `KeyPurpose::StepCache` with a bound per purpose (1 KiB by default, 4 KiB ceiling for cache keys).
- Security minor 3: counts and the hit's source run only to callers who may disclose every location the key reads.
- Security minor 5: the highest floor of a test run's pins applies to every disclosure of that test run.
- Security minor 6: a test-tree run whose version is not its own private one bypasses the cache.
- Security minor 8: an internal workflow is refused as a handler with the same opaque `NOT_FOUND` as a missing or
  foreign one (`errors.md:35` changes from 422).
- Security minor 9: payload foreign keys are `RESTRICT`; erasure of shared bytes is a feature 020 obligation (G022-021).
- Architecture minor 16: decision OD-2026-11-24-4 is on `docs/owner-delegation-2026-11-24`; this branch merges after it.

The other minors and nits, each accepted as proposed:
- Architecture minor 4: ADR-0135 names the interim `COLLECTION_FINISH_ROUTING` refusal for `cache`, as ADR-0094's
  amendment and FR-022-001 do.
- Architecture minor 5: T9 locks the dispatch row inside `RunStore::create` through a `NewRun` field, following the
  `pending_reset` and `resets::lock_pending` precedent.
- Architecture minor 6: T15 and T16 place the retained definition where `create` takes it (`ad_hoc::retain` and its
  workflows quota lock before the slot 4a locks, `execution.rs:1531` against `1619-1640`).
- Architecture minor 7: a future run deletion (feature 020) cycles with the dispatcher's order through the dispatch
  row's foreign key: recorded under G022-021.
- Architecture minor 8: a handler child with its parent's run-level group and key can never be granted while the
  parent holds its permit: refused at publication.
- Architecture minor 9: a handler region refuses `compensateWith` (no new compensable work after the stop,
  constitution III).
- Architecture minor 10: the database enforces at insert that a child's `test_tree` equals its parent's.
- Architecture minor 11: the `origin` CHECK is extended `NOT VALID` then `VALIDATE`; the unique-index build's lock is
  planned; `down` refuses while rows with the new origins exist.
- Architecture minor 12: once a journal holds the new events an older binary cannot decode the run
  (`event_codec.rs:490`): rollback is switching the capability off, never a binary downgrade.
- Architecture minor 13, correctness nit 1: one spelling of the handler timeout (`FAILURE_HANDLER_TIMEOUT` in the
  journal and `TIMED_OUT` in the column, with the mapping stated).
- Architecture minor 14: the evidence of a seeded `DRAFT_TEST` waiter moves from T022-204 to T022-217.
- Architecture nits: ADR-0135 decision 2 reads "the transaction's data, including outputs staged in the same commit";
  decision 9 says the writer takes no lease or fence; "obligation" is qualified everywhere (018's, store, dispatch);
  ADR-0050 slot 6 says `persist` writes the journal after slot 7.
- Correctness minor 3: invariant 14 admits `step.mocked`.
- Correctness minor 4: SC-022-002 and US01 scenario 4 are restated against the cache-entries ceiling.
- Correctness minor 5: a full pending-obligations ceiling at T2 is skipped and counted, never a failed completion;
  expired, unpurged entries count against the entries ceiling until purged; counts are usage rows kept by
  `SECURITY DEFINER` triggers with `SET search_path = pg_catalog, pg_temp`, as `flow_control_counts` (security minor 10).
- Correctness minor 7: a rerun of a handler run does not inherit `handler_for` either.
- Correctness minor 8: a dispatch counts its attempt before it admits, so a crash in admission still reaches `DEAD`.
- Correctness nit 3: ADR-0135 decision 7 states the exclusive-taker argument rather than an order a run-start grant
  does not keep; ADR-0050's amendment gives cache rows their slot at admission (Architecture Major 2).
- Correctness nit 4: `FOR SHARE` on a hot entry writes a multixact per hit: measured by task T022-062.
- Security minor 7: the separate draft-test pool adds its own load on a shared external system: documented in the
  runbook.
- Security minor 10: `keyValue` and `mock` are bounded (4 KiB, the JSON depth limit) in the schema.
- Security nits: the check order of test runs and pins is specified (namespace from the header, 404 for the workflow,
  then the `prod` or approvals 403, then the `fromRun` 404 before `PIN_NOT_DISCLOSABLE`); the `test_tree` guard is
  `ENABLE ALWAYS`; the shared standalone default group is documented; standalone script nodes accept only artifacts of
  published script versions; the cache listing's exposure of private workflow ids and `sourceRunId` to `ops:read` is
  stated.
