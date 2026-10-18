# Refused output compensation worker test C1 notes

## Artifacts

- Patch: `/tmp/sagat-refused-output-compensation-worker-tests-c1.patch`
- Patch SHA-256: `3473238b93be5c8d6c6055bd00b2fea38e0f549d6e80182b63f660092c7191d1`
- Decision input: `/tmp/sagat-missing-output-worker-test-decision.md`
- Decision SHA-256 verified: `0e0a30b922079dfd68d674d138902664a7859830b862e678cbf7674e9cebbe9f`

The patch changes one test file only:
`tests/integration/workflow-primitives/byte_ceilings.rs` (`+388/-2`).

## Drafted behavior

- Preserves the existing string-output fixture through a private output-factory wrapper.
- Extracts the existing stopped RUN_DATA_LIMIT drain assertion into one private helper used by the
  old and new scenarios.
- Adds one nested focused test module with a fixture-owned reservation set and one release handler.
- Produces 17 real public forward reports whose JSON reservation objects are each exactly 250,000
  serialized bytes. The first 16 are retained as 4,000,000 actual payload bytes; the seventeenth
  real success is accepted without its refused output.
- Parses each actual compensation `TaskLease.input_json` through one strict typed fixture parser.
  It requires `steps.reserve.output.reservationId`, removes that parsed ID from the test-owned set,
  and completes only after the removal succeeds. Malformed input and an unknown reservation are
  distinct errors and cannot be reclassified as the missing-output diagnosis.
- For the refused member, asserts valid JSON with no `steps.reserve`, exact compensation target
  identity and keys, then reports the fixture application code
  `FIXTURE_RESERVATION_OUTPUT_REQUIRED` through public `FailStep` with `retryable=false`.
- Processes available retained compensations with the same handler before the missing-input wait.
  After the authorized resolution it processes any retained compensations that were held back, so
  the proof does not depend on which independent iteration is selected first.
- Observes persisted `WAITING`/`OPERATOR`, the fixture failure code, `MANUAL_REVIEW`, the open exact
  saga entry, absent forward output, no fabricated completion/skip, no `run.finished`, idle
  scheduling, and journal replay.
- Uses the existing authorized `RESOLVE_STEP` operation at `lines[16]/reserve~c`, outcome `FAILED`,
  the same code, and the fixed reason `missing reservation output prevents fixture release`.
  It checks the exact `saga.step_resolved` audit and `step.resolved` journal event, then verifies
  final `COMPENSATION_FAILED` while the refused reservation remains unreleased.

This is a current-version worker application contract test. It does not claim an engine input
precheck, old-binary compatibility, a process restart, external provider execution, exactly-once
effects, or release acceptance.

## Source preservation and apply checks

The root-owned byte-ceiling file was read at and remains at:

`7b28f7f21de920acb75474e7359eee45b49ad8d125f9e97968f7c7109ca18e26  tests/integration/workflow-primitives/byte_ceilings.rs`

This hash includes the uncommitted receipt-fault C1 supplied by root. The drafted post-apply file is:

`1b0c02de0779453ac5b5c4655c7775ff0aa28a1d5c65ea01c8532ede711451c0  tests/integration/workflow-primitives/byte_ceilings.rs`

Checks performed without editing the repository:

```text
git diff --no-index --check <base> <draft>                       PASS (no output)
git apply --check /tmp/sagat-refused-output-compensation-worker-tests-c1.patch
                                                                  PASS (no output)
isolated temporary copy: git apply --check                        PASS
isolated temporary copy: git apply                                PASS
isolated applied SHA-256                                          1b0c02de0779453ac5b5c4655c7775ff0aa28a1d5c65ea01c8532ede711451c0
isolated applied file vs drafted file                             byte-identical
```

The repository HEAD advanced during drafting from the decision baseline to
`1310b09b06e0fcf04e26c4b23a3ea8c4cbb2e6db`; the target byte-ceiling file did not change, and the
final `git apply --check` passed against that current workspace.

No repository file was edited. No test, build, formatter, commit, provider I/O, blob read, grant,
public API change, subprocess worker, restart, or external-effect proof was performed or claimed.
