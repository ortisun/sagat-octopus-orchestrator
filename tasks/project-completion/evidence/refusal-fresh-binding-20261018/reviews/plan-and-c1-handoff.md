# Fresh scoped refusal authority: bounded test plan

Read-only design against `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`, branch `feature/016-project-completion` (initially observed HEAD `7ea90d2f`; root subsequently reports HEAD `21583d62` plus two uncommitted fault tests). No repository files, tests, builds, formatting, grants, or production hooks were changed/run. This is a proposed test, not passed evidence. Existing uncommitted `byte_ceilings.rs` work belongs to root; preserve it byte-for-byte and add the binding case outside its nested compensation module.

The first unit should prove **a binding revoked after the checked bounded report transaction has ended is rejected by the actual fresh full refusal transaction**. Use one existing test file: `tests/integration/workflow-primitives/byte_ceilings.rs`, plus the normal evidence record if root chooses. No generic store wrapper is necessary for this property.

## Why the existing provider pause is insufficient

`worker/scoped_reports.rs::checked_path_report` opens a path transaction, checks authority/receipt/lease, captures secret versions, commits, performs the actual provider scan, then opens another path transaction and rechecks authority, live fence and versions (approximately lines 230–285). `bounded_report_provider.rs::PausedRead` pauses within that provider scan. Its role/binding revocation cases therefore prove the second **path** check, before refusal selection.

The new gap starts later: `apply_path_report` reads the effective byte total, creates `RefusalCandidate`, and returns without committing its owned path transaction (approximately lines 356–403). `report_path` then calls `refuse_scoped_output`. The helper calls the existing full `RunStore::begin`, checks the pinned definition and current authority through `replayed`, then checks token/fence and exact `scanned_versions` before computing the budget again (`scoped_output_refusal.rs:30–77`). The new test must visibly reach this full entry before changing authority.

## First unit: public binding revocation at native full restoration

Suggested test name: `a_scoped_output_refusal_rechecks_binding_after_the_checked_path_is_dropped`.

### Existing fixture and entrypoints

- Reuse `collection_at_seventeenth_member(name, false)` in `byte_ceilings.rs` (approximately lines 105–178). It publicly publishes/starts a one-level collection, completes sixteen real 250,000-byte outputs, and publicly leases member 16. The checkpoint return is the scalar `1`; the collection remains OPEN. Retained member output bytes are 4,000,000 and the candidate raises that to 4,250,000, above 4,194,304. Assert these premises from the stored ledger/scope and output length rather than merely trusting the helper.
- This is a current public fixture. It makes no old-binary or legacy-overlay compatibility claim. There are no fabricated output, status, accounting or scope-source rows.
- Submit the unchanged lease/output/key through `Flow::try_complete`, which uses public gRPC `CompleteStep` (`tests/support/scope_flow.rs:332`). The real inline path is `worker/output.rs::complete_inner` → `report_path` → `checked_path_report` → `apply_path_report` → `refuse_scoped_output`.
- Obtain the namespace with a read-only lookup of this run or the existing public namespace listing. The worker subject is `scope-worker` from `Flow::start`.

### Deterministic native barrier

1. After leasing member 16, take the baseline using existing `refusal_transaction_snapshot`. Keep the scheduler idle for this entire report test.
2. On the test owner's own connection, begin a transaction, record `pg_backend_pid()`, and execute `LOCK TABLE public.iteration_checkpoints IN ACCESS EXCLUSIVE MODE`. This only gates a read; it does not mutate rows or alter privileges.
3. Poll the public report concurrently with an observer, following the existing late-receipt test's `tokio::pin!`/`tokio::join!` pattern. Do not await the report before running the observer.
4. Wait for the **exact full restoration query** on the API runtime backend, using `eventually` and a bounded deadline. Witness all of:
   - `datname=current_database()`, `usename='sagat_app'`, `state='active'`, `wait_event_type='Lock'`;
   - the gate PID belongs to `pg_blocking_pids(writer_pid)`;
   - query contains `SELECT scope_id, iteration_index, return_instance_id, result_digest, journal_sequence` and `FROM iteration_checkpoints WHERE tenant_id = $1 AND run_id = $2`;
   - preferably also an ungranted `AccessShareLock` for that PID on `public.iteration_checkpoints` in `pg_locks`.
5. Before revocation, assert the comprehensive snapshot still equals the original baseline. It does not query `iteration_checkpoints`, so it can run while this barrier is held.
6. While that same full-restoration waiter remains blocked, issue the actual admin HTTP `DELETE /namespaces/{namespace}/worker-bindings/scope-worker` as `alice`/`acme`. Assert HTTP 200 and `{sessionsClosed: 1, leasesReleased: 1}`. Bound the operation; a timeout is a failed barrier/lock assumption, not successful revocation.
7. Read and assert the intended withdrawal: zero matching binding rows, removed matching session, the exact member's lease expiry shortened by the native unbind operation, and one corresponding `worker.unbound` audit. Status, token hash, attempt and generation remain those of the leased member. Do not edit those fields in SQL.
8. Capture `after_unbind = refusal_transaction_snapshot(...)` while the full transaction is still paused. Use this as the no-report-mutation baseline: the public unbind legitimately changes the step expiry/update timestamp and appends its audit. Explicitly compare stable run/journal/head/output/accounting/scope/receipt/outbox state against the pre-unbind baseline as well, so those legitimate differences do not conceal a report effect.
9. Recheck the same PID/query/blocker witness, then roll back the owned gate transaction. Await the public report result.

### Why this is specifically the fresh full refusal transaction

- `PgRunStore::begin_in_transaction` (`crates/infra-postgres/src/execution.rs:176–263`) locks the run, reads its step metadata, then calls `load_collections`. Its `iteration_checkpoints` query is at approximately line 1941. It has not returned `PgRunTx` yet, so `refuse_scoped_output` has not executed its new current binding/membership checks.
- `read_path_snapshot` is in `execution/path_read.rs`; it does not read `iteration_checkpoints`. Existing `bounded_reports.rs` cases withhold SELECT on that table while ordinary bounded completion/replay succeeds, which corroborates this separation without requiring any grant changes in the new test.
- The path's `logical_output_bytes` delegates to `collection_outputs::inspect`. This fixture has no SUCCEEDED root collection; `SUCCESSFUL_ROOT_COLLECTIONS` yields no candidates and `verify_journal` returns on empty sequences (`collection_outputs.rs:177–203, 307–333`). It cannot accidentally hit this full restoration barrier while calculating the path budget.
- Arrival at the full-only query implies the bounded report passed its initial authority checks, selected refusal, and relinquished its transaction; otherwise the full entry could not acquire the same run lock. Revocation then exercises the new full-transaction recheck.
- This is a pause **inside full transaction entry**, after the full transaction has acquired the run lock, not literally before `RunStore::begin`. Describe it accurately. It is sufficient for current binding/membership/secret catalog revalidation and is insufficient for a competing run mutation.

### Why public unbind can commit here

`crates/infra-postgres/src/bindings.rs::replace_binding` takes the binding advisory lock exclusively (approximately lines 76–84), deletes/replaces the binding, locks the subject's sessions, expires their leased steps and removes those sessions (lines 145–185). It does not lock the run row or read `iteration_checkpoints`. The earlier path binding/namespace/membership locks have been released. The paused full transaction has only read the step rows, without row locks.

This is source-backed feasibility, not a runtime result; the test must demonstrate the actual HTTP unbind completes while the exact full query stays blocked.

### Oracle

- Public completion returns `tonic::Code::PermissionDenied`; `support::worker::machine_code` returns `NOT_BOUND`. Source: `worker.rs::replayed` → `ensure_bound` → `ensure_activity_bound` (approximately lines 2093, 2202, 2229); `api-http/src/error.rs:537`; `api-grpc/src/status.rs:18–44`.
- This is discriminating: native unbind changes expiry but preserves token/attempt/generation/status. The no-secret inline refusal selects `RefusalFencePolicy::LeaseMatch`, and `PgRunTx::lease_matches_instance` checks the exact path/token and leased/running status, not session existence or expiry (`execution.rs:3968`). An implementation that skipped the fresh binding check could proceed; a generic stale result is not the intended assertion.
- After the rejected report, the complete snapshot equals `after_unbind`: no new root stop, scoped completion, report token, report receipt, refusal audit, telemetry head, payload/output/accounting charge, scope/saga transition, or outbox row.
- Explicitly assert no receipt for the tested report key and no `run.data_limit_exceeded` audit for its refusal. Existing sixteen successful members are preserved, the seventeenth remains leased, and no RUN_DATA_LIMIT stop is added.
- Release all test-owned locks and call `flow.verify()` for cold journal/projection consistency. Do not run the scheduler while checking this no-mutation result; unbind intentionally made the lease expire and a later scheduler pass may legitimately mutate it.
- The existing positive successful-refusal test establishes that this fixture normally reaches the refusal writer. A separate no-revocation barrier control is optional if implementation discovers ambiguity in the witness; do not broaden the first unit into unrelated drain or protocol coverage.

## Separate later proofs

### Current membership after the same checked-path gap

Reuse the same native barrier in a distinct test. Preserve the binding and withdraw `scope-worker`'s database membership to viewer while full restoration is blocked. Existing tests use an owner-only `UPDATE memberships SET role='viewer' WHERE tenant_id=$1 AND subject=$2`; there is no discovered public role-change endpoint, and `PgIdentityStore::insert_membership` explicitly cannot replace an existing role. This is an authority withdrawal fixture, not execution-state fabrication.

Expect the already-authenticated in-flight call to fail `PermissionDenied` / `FORBIDDEN` from native `current_worker_authorized`, not initial gRPC authentication. The native full helper locks the namespace secret catalog and uses `sagat_lock_memberships` before reading the role (`execution.rs:768–791, 3648`). Snapshot equality and cold verification follow the same pattern, with the intentionally changed membership checked separately.

### Exact scanned secret versions after the *second path* check

Do this in a later unit, using existing support instead of widening `Flow`:

1. Start the real pinned `openbao/openbao:2.7.1` container and official `OpenBao` adapter following `bounded_report_provider.rs:74–112`. Preserve temporary token-file cleanup through `TempDir`.
2. Use `support::worker::platform_with(Options { runtime_role: true, secret_provider: Some(provider), ..Default::default() })`. `Options` already configures both managed-secret REST and worker scanning (`tests/support/api.rs:226, 426, 519`). `Platform::{namespace_id,rest,publish,start}` plus `support::blob_collections::{scheduler,scoped_worker}` provide public publication, actual scheduler ownership and protocol 1.5 without modifying shared fixtures.
3. Create version 1 through public PUT `/namespaces/{namespace}/secrets/scoped-key` with `requiredProvider: "openbao"`, `expectedVersion: 0`, a UUID `requestId`, and a test value absent from all output bytes. Publish the same one-level 17-member collection with `secrets: ["scoped-key"]` on its forward activity. Publicly complete sixteen actual outputs and lease member 16. Use the existing supported public shape; do not reuse `scoped_worker::fixture`, which inserts a trusted definition/run/steps with SQL.
4. Hold the full-checkpoint barrier and submit the seventeenth clean public completion. Reaching that full-only query proves provider scan **and the second path version comparison** have already completed. The provider need not be paused. If read evidence is desired, a small counter wrapper must delegate every read/create/delete to the actual provider, following `PausedRead`; never return canned values or scan-success flags.
5. While the exact full query is blocked, publicly rotate to version 2 (`expectedVersion: 1`, fresh `requestId`, a different value also absent from the output). Assert provider/version metadata and the actual version-ID set changed. Both native managed prepare and activation lock the secret namespace/current role but no run row (`infra-postgres/managed_secrets.rs:120, 235`), so this should commit before full entry resumes.
6. Capture the baseline after the legitimate secret operation, release the gate, and expect gRPC `Unavailable` / `UNAVAILABLE`, from `scoped_output_refusal.rs`'s comparison of the exact scanned `Vec<StoredSecret>` to `tx.secret_versions` (approximately lines 49–54). Native `all_versions` returns every stored version in deterministic name/version order (`infra-postgres/secrets.rs:143–164`). No refusal effect/receipt/head/accounting change may occur.
7. A same-key retry, without another rotation and with both secret values absent from the candidate, must scan current real versions and commit the genuine successful-output refusal once. This establishes that the first error was stale scan evidence rather than an incapable provider. Then cold replay/verify. Keep this follow-up at at most five files: one test module, optional module registration, optional test-local fixture helper, evidence; no production API or trait changes.

### Fence, terminal scope, and a literal before-full-open barrier

The chosen table barrier **cannot prove** a new attempt/generation, terminal scope, or competing report committed after path selection: it already holds the run lock, so genuine scheduler/report state transitions cannot win during the pause. Wrong namespace/fingerprint passed initially also tests earlier rejection unless a real intervening receipt or authority change is established. Keep those claims out of this first unit.

If a later fence test needs a precise pause before the full transaction is opened, a test-local `PgRunStore` wrapper is feasible, but materially larger. It must implement/delegate all nine required traits: `RunStore`, `WorkerStore`, `DefinitionStore`, `PathExecutionStore`, `QuotaLookup`, `TelemetryStore`, `NamespaceDirectory`, `SecretAccessLookup`, and `ScriptLookup`. Keep `RunStore::Tx = <PgRunStore as RunStore>::Tx` and `PathExecutionStore::Tx = <PgRunStore as PathExecutionStore>::Tx`; return the real native transactions unchanged. Pause only the armed target `(tenant, run)` in `RunStore::begin` **before** delegating; all other methods, including protocol-support flags and default-capable methods, delegate with fully qualified traits. `complete_inner` sends a scoped inline report straight to the bounded path, so this armed full begin is the exceptional reopen. No fabricated `Ok`, no caught/ignored native error, no synthetic Tx/state/output, no skipped authorization, no new production hook/grant.

While that wrapper is paused, an independent actual worker/scheduler can commit the tested authority/fence transition; then the wrapped call delegates to native full entry and revalidation. Merely constructing a wrong fence at initial submission would not prove this gap. This adapter should be reviewed as its own bounded test unit, not added to the first binding proof. No current proof of terminal-scope/fence-race feasibility is claimed here.

## Prospective validation and scope

Root should run the exact new integration test first, then the affected existing successful-refusal test, on its merged tree; preserve the actual commands/output. Run the focused test again with `SAGAT_TEST_AS_RUNTIME_ROLE=1` as required. `Flow` already uses the runtime pool even in the ordinary invocation. Before any build, check `df -h`, use a task-owned private `CARGO_TARGET_DIR`, and respect the two-build machine limit. No test outcome or genuine RED is claimed by this read-only design.

This closes only the named current-authority-after-path-gap property once the test passes. It is not an additional whole-protocol review, an old compatibility claim, a full-data-hydration proof, or completion of every §11.4 acceptance case.

## Draft implementation handoff — binding unit only

The focused one-file patch is `/tmp/sagat-refusal-fresh-authority.patch`. It applies only to
`tests/integration/workflow-primitives/byte_ceilings.rs`; the later membership, secret-version and
fence units above remain unimplemented.

- Observed target HEAD after root committed the two refusal fault tests:
  `e51b33901a0cb9dad3758644aaa3dbaf1bc70962`.
- Target file Git blob: `3b6ff21a2dbeff7df18511004acdcd8285891bb5`.
- Target file SHA-256:
  `6db85054b0644b0954cea9fca136288ba530892b7a0880a9341a8fab3ea34556`.
- Patch SHA-256:
  `6cc78363e3550f4147e0bcd6ad955280498af42da3c3a247f77acc77968dc8ae`.
- Patched-file SHA-256:
  `b7c05896ed76b0f5504c7f72add93df8ee236d33849663414aef23a6887011cb`.
- `git apply --check /tmp/sagat-refusal-fresh-authority.patch` exited 0 against that target.
- `git diff --no-index --check` emitted no whitespace diagnostics; its exit 1 means the files
  differ, as expected.

The patch adds one `current_thread` integration test and focused state/query helpers. The test uses the
existing public 17-member fixture and public gRPC completion, gates the exact full restoration
query with an owner-held `ACCESS EXCLUSIVE` lock on `iteration_checkpoints`, and requires an
ungranted relation `AccessShareLock` from a named `sagat_app` PID. While that same PID remains
blocked, it executes the native admin DELETE route and checks the real camelCase response, exact
binding/session removal, worker-unbound audit, and lease-expiry withdrawal with status/token/
attempt/generation/holder preserved. It separates the legitimate audit/timestamp differences from
the stable execution snapshot, then requires `PermissionDenied` / `NOT_BOUND`, exact post-unbind
snapshot equality, explicit zero receipt/refusal-audit/output/report effects, unchanged journal
head/outbox/accounting, and cold `flow.verify()`.

No repository file was modified by this draft. No build, test, formatter, migration, grant, hook,
commit or scheduler tick was run. A root-owned untracked
`tasks/project-completion/evidence/collection-sdk-regression-20261018/` appeared while drafting and
was left untouched.
