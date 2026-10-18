# Frozen old producer / current legacy-output reader

The actual old writer is immutable commit `6c44a0e49670cf19660792cd1518d241e6c99b3b`.
Current production base is `52d2c572` plus the reviewed one-line owner discovery compatibility
amendment. The sole old overlay adds a test producer and its registration; no old production or
migration bytes change. Complete old source and migration manifests match before/after overlay
removal. Both real executables are built and hashed before either process phase starts.

## Reproduction

Use an exclusively owned PostgreSQL 16 server. This run used the owned PostgreSQL 16.15 container
`sagat-project-completion-scoped-schema` on loopback port 65117. From this worktree:

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
export SAGAT_Q5_OWNED_POSTGRES=1
export SAGAT_Q5_ARTIFACT_ROOT=/tmp/sagat-project-completion-q5
scripts/test-legacy-collection-output-view.sh
```

The coordinator creates its own archive and two independent old-producer databases, waits for the
old process to exit, migrates and executes the current reader, then removes only its validated
owned databases and private receipts. It leaves existing databases untouched. Shared-target Rust
source timestamps are invalidated in both build directions; archived contents remain identical.
Executable archives/binaries and credential receipts are not committed to this evidence.

## Actual result

The complete coordinator passed in **133.91 seconds**, exit 0. Each role has exactly one passing
old producer test and **one current matrix test containing five ordered cases**, not five tests.
Both old producers execute through the real public HTTP/gRPC/scheduler path using `sagat_app`.
The role distinction applies to the current store pool: database owner versus `sagat_app`.

- Owner producer: 1/1, 9.45 s harness; current matrix: 1/1, 0.42 s.
- Runtime producer: 1/1, 9.19 s; current matrix: 1/1, 0.39 s.
- Strict integration/PostgreSQL Clippy: passed, 28.91 s.
- Workspace formatting: passed, 4.40 s; standalone inert producer formatting: passed, 0.05 s.
- Architecture: passed, 0.60 s.
- All 1,117 frozen native inputs remain unchanged through the final process and static gates.
  The manifest separately pins the coordinator, consumer, producer and copied executables.

The matrix proves verified read-only missing-owner aggregates per owner (including equal digests),
exact source/projection/journal/payload witnesses before/after migration and read-only commit,
isolated namespace/missing-owner/routing/payload contradictions, hash rejection on hydration,
fitting positive charge added only to L, zero-delta writes above the limit, unclamped E>L scalar,
typed full-hydration refusal and positive-delta rollback above 4 MiB. Positive/zero-delta writes
exercise the actual native RunTx persistence adapter; they do not claim worker-service authorization
or fresh scheduler recovery behavior.

## Counterexamples and fixture fixes retained

The first real old/current attempt rejected a valid later owner whose stored discovery marker was
`legacyControl`. `old-owner-metadata.txt` observes actual old output; `owner-kind-diagnosis.md`
explains the bounded correction. Fresh review found no remaining issue: pinned root ForEach and
all identity/namespace/scope/counters/finish→route journal predicates remain required. No backfill
or historical result repair occurs. Source amendment SHA-256:
`202282e9bf2522bd37ccf0349d85af21bd5231cdb9b627ff49e9387a79c789a1`.

Earlier attempts are separate from the passing result: borrowed SQL text compilation, redundant
closure-drop lint, stale same-version build-cache artifact, old UUID wire serialization, an overly
specific missing-owner error oracle (the earlier restore layer correctly refuses corruption), and
test-only deferred quota triggers preventing restoration of the payload append-only guard. The
last fixture sets constraints immediate before mutation and restores ENABLE ALWAYS in that same
transaction. The failed trigger attempt rolled back; it is not a production defect or a passing
negative test. Each actual failed command and available source/diagnostic log is retained.

The coordinator's child-publication interruption probes reproduce SIGINT/SIGTERM orphaning before
the C3 fix and verify termination/reaping after it. The initial probe harness pipe timeout is
retained separately. Fresh bounded review C3 found no remaining issue; the later timestamp
invalidation is tooling only. Three whole-coordinator reviews are complete; none is repeated here.

## Limits

This closes Q5 reader/guard acceptance. It does not prove the complete control-plane/execution-engine
old-binary upgrade, mixed writers, rollout quiescence, scheduler ExistingOutputs or Stopped drain
production, nested runtime/capacity, cold OS restart or uncertain COMMIT. In particular, second root
owners also retain legacyControl in the current dedicated route; the separate Q8 native drain
qualification still needs its matching compatibility correction and genuine consumer acceptance.
Canonical open tasks remain open until their own remaining gates pass.
