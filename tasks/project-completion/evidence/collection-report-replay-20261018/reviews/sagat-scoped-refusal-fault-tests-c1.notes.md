# Scoped successful-output refusal late-fault test draft — C1

## Provenance and scope

- Repository: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`.
- Inspected branch/HEAD: `feature/016-project-completion` at
  `3710a5b7fc48a9a97334be28176ab4c169eab54c`.
- Sealed patch: `/tmp/sagat-scoped-refusal-fault-tests-c1.patch`.
- Patch scope: one existing file,
  `tests/integration/workflow-primitives/byte_ceilings.rs`; 224 additive lines.
- No repository file was edited. No formatter, build, test, database, commit, subagent or external
  write was run.

## Bounded case

The added current-thread test reuses the private `collection_at_seventeenth_member`,
`assert_successful_refusal`, `refusal_identity` and compensation helpers. The fixture publishes and
starts the existing compensable 17-member collection, then completes the first sixteen genuine
public gRPC worker leases with actual 250,000-byte results. It leases the real seventeenth member
and does not seed any status, journal event, payload, usage ledger, receipt or audit row with SQL.

The report uses an inline JSON output. It uploads no blob, reads no blob body and performs no
provider operation.

## Exact late fault and observation

The injector is an `AFTER INSERT` trigger on the real `command_dedupe` receipt statement, restricted
to the refusal ACK shape `SUCCEEDED|CANCELLING|...`. The production writer reaches this statement
only after:

1. the root-only RUN_DATA_LIMIT stop phase was persisted;
2. the scoped genuine `StepCompleted` phase, projection, reported lease and final journal/head were
   persisted; and
3. the refusal audit was inserted.

The trigger waits on a test-owned advisory transaction lock and then raises SQLSTATE 23514 with the
fixed message `test-only scoped output refusal receipt abort`. While the runtime-role writer is
observed active and lock-waiting in the actual `INSERT INTO command_dedupe`, a second owner-pool
connection compares the exact pre-report snapshot. The snapshot includes the complete Run, every
scope/member/step, journal rows and their payloads, run usage, telemetry row, every tenant payload,
the full tenant retained-usage row, every tenant audit and receipt, every tenant outbox row, and the
Run's saga headers and entries. Thus root stop, scoped success, reported lease, journal/head, output
or physical/logical accounting, payload, audit, receipt, saga or outbox changes cannot be visible
independently.

The observer checks the same writer PID is still blocked at the receipt after the snapshot before
releasing the gate. The gRPC request must return `INTERNAL` and no StepAck. Captured storage logs
must contain the fixed trigger message and must not contain `statement timeout`. The complete
snapshot must again equal the pre-report state after rollback.

## Retry, replay and drain

After removing only the test trigger/function, the test sends the identical public gRPC request,
including the same idempotency key, lease, fence and output. It must return a newly applied
`SUCCEEDED` / `CANCELLING` ACK. Existing assertions require:

- the seventeenth step is genuinely SUCCEEDED with its exact reported lease;
- no output digest or logical output charge is added;
- the root stop is FAILED/RUN_DATA_LIMIT and the collection remains OPEN;
- no forward successor is released;
- exactly one typed fixed-reason refusal audit and one original receipt exist;
- the report adds zero outbox rows; and
- the telemetry head equals the Run/journal head.

The same request and key then return the original command id with `already_applied=true`. The full
snapshot and refusal identity remain byte-for-byte/JSON equal, proving no duplicate stop, success,
reported lease, event, audit, receipt, payload/accounting, saga or outbox effect.

Finally the ordinary scheduler discovers the durable CANCELLING/OPEN scope, starts the native
RUN_DATA_LIMIT drain and drives all seventeen genuine `line.release` compensation leases through
public gRPC completion. Existing helper assertions require one drain outbox, a compensated saga,
the final failed scope identity and successful journal reconstruction.

## Explicit boundary

This single test covers the first-stop, report-before-drain, ordinary successful forward report.
Byte-for-byte preservation of a pre-existing different stop, drain-before-report ordering, Applied
reconciliation, refused-output-dependent compensation, restart, frontier races and other section
11.4 cases remain separate pending evidence. No canonical 014/016 task or general collection quota
closure is claimed.

## Read hashes (SHA-256)

- `.specify/memory/constitution.md`:
  `e086fe6592203707d9a75f381041415c9f3b65d071f7b344c098cadba44c253e`
- `specs/014-workflow-primitives/spec.md`:
  `eac73844ed31360ac03293474ac5d6213ab0db7bbfc585d5a3155fdf2d4490df`
- `specs/014-workflow-primitives/plan.md`:
  `ffaa76a78278faaf575278c1425615f6a042dbb2412afb2ad2a690f3fff12930`
- `specs/014-workflow-primitives/tasks.md`:
  `c25ca47fb054b6fe99cc3402d1ec7577fd3fa4f8add15d2cf2c5a5a9db230f5e`
- `tasks/project-completion/collection-result-quota-correction.md`:
  `9603130b2b3b2e926835f03500562f45db0741173abba786bc60f661a0e9c9c8`
- `tasks/project-completion/scoped-output-refusal-discovery.md`:
  `7a45e34ea1f2e3642776ec870b3f226d793c953ba983eeedd97bb93104acc417`
- `crates/application/src/worker/scoped_output_refusal.rs`:
  `5c9d80bf363360d76c6764fee582f6a6a1045c947ce0b6cc68b973e71413affb`
- `crates/application/src/worker/scoped_reports.rs`:
  `a6b2eba23746c4a1c67fa51bb8597280935357209084dcc53e1f567e4748120f`
- `crates/infra-postgres/src/execution.rs`:
  `0e5990275678cc160924f7866ab8ebfb9c3f354ac80406f0e56a9cee930f1395`
- `crates/infra-postgres/src/definitions.rs`:
  `d317418ac96847d617b6c6e925840f0eb1bdf8844362e225d5ebd5812b3d7774`
- `crates/infra-postgres/src/identity.rs`:
  `89499a455aa991cf7c87c2402ba1a127af842b0dcd128ad41c3adffef1314b00`
- `tests/support/scope_flow.rs`:
  `c6722902a332ae5c693f06598b0b21d3652afa5ace63a626aba3931a532fa821`
- `tests/support/logs.rs`:
  `43abd3b3f404a98dbb1958ee169359a71335c7ba5ca62e7b978722ea40c15e1c`
- `tests/support/worker.rs`:
  `13e601d63bf5b5b837bd3d9a9410d1a099b467604e37dcbfa8b900a07e3f3be5`
- input `tests/integration/workflow-primitives/byte_ceilings.rs`:
  `77d7e35e582d1b28de515af3e9d46ab5723b6701f00287aeb9cf8c70f33ac526`
- proposed output `byte_ceilings.rs`:
  `04cc8698fa55d2588762681e9d0300b7ae76dc024e3795c1792e83c11c348ad0`
- sealed patch:
  `4f78ad2c2e6d6030fbab1c7fba3e95a3a25eb9c006d54a37c86785b28916f0d4`

## Static gate and unverified evidence

- `git apply --check /tmp/sagat-scoped-refusal-fault-tests-c1.patch`: succeeded against the
  inspected working tree.
- `git apply --numstat`: `224 0 tests/integration/workflow-primitives/byte_ceilings.rs`.
- Repository status/diff for `byte_ceilings.rs` remained empty.
- No compilation, formatting or runtime evidence exists. An eventual focused command would be:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives \
  byte_ceilings::a_late_receipt_fault_rolls_back_the_whole_successful_refusal_and_same_key_retries \
  -- --exact --nocapture
```
