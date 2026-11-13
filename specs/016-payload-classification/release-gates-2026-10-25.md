# Integrated release gates — 2026-10-25

The owner requested merging implemented tasks and preparing the corresponding release. This
record tracks current integration evidence; it does not complete the eight open feature-014
tasks, enable general nesting, or include the unverified B2 working-tree changes.

## Corrected shared fixtures

The integration includes `d3238914` from feature 017: staged stream work, the documented
16 MiB SDK receive limit in the Rust fixture, and the approval-chain observation after its
pending approval commits. See the 2026-10-25 entries in
[017 verification](../017-database-connections/verification.md).

The classified-tree provider fixture previously reused a completed task at its read barrier.
It now asserts that attempt is stale and leases a fresh task for the revocation race. Its
focused real PostgreSQL/OpenBao RED and GREEN results are in that same record.

## Linux public receipt-gate diagnosis

PR 47 run `40163480113` failed four public API/proxy tests before observing their exact
receipt-INSERT lock. Diagnostic changes retain the five-second observation deadline and every
original predicate: supplied-pool backend identity, database/user/application, active lock wait,
exact gate blocker and receipt/advisory-lock markers. That diagnostic-only revision changed no
TCP setting, timeout, production transaction, authority or receipt outcome.

Safe diagnostics expose only predicate booleans, backend state/wait categories, bounded ages,
relay protocol tags/counts and termination categories. They never print queries, bind values,
frame bodies, RPC messages or status payloads. Observation races early RPC/relay termination;
pending tasks retain abort-on-drop ownership. This prevents a failing observer leaving a
live reporting task detached.

A fresh independent review found no C1/C2 issue. Root reran the integrated tree on macOS arm64,
repository Testcontainers through Colima, private Cargo target, two build jobs, dev debug info
and incremental disabled, and two Rust test threads:

```sh
cargo test -p sagat-tests --test integration_workflow_primitives --locked -- \
  api_reopen::supplied_single_connection_pool_backs_actual_public_completion_after_reopen \
  public_refusal_commit_loss:: \
  capacity_finish::commit_loss::unknown_commit_reconciles_actual_commit_and_abort_without_inventing_a_new_key \
  --nocapture
```

Output: **5 passed; 0 failed; 0 ignored; 549 filtered out; 25.99 s**.
`cargo fmt --all -- --check` passed. Architecture: **24 crates, layers respected, no cycles**.
The macOS pass does not classify the Linux failure. A fresh Linux CI run must supply that
evidence before merging; any remaining failure remains a release blocker.

Root focused strict Clippy also passed (1m13s): `cargo clippy -p sagat-tests --test integration_workflow_primitives --locked -- -D warnings`.


## Linux diagnosis and relay correction

The diagnostic run [40232796470](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/40232796470/job/121287909858)
on `27c8a01b` reproduced **534 passed; 4 failed; 16 ignored** in the workflow-primitives target.
All four public receipt observations reached the unchanged 5,000 ms deadline. Each captured
backend matched the supplied pool's PID, database, runtime user and application; none was yet
waiting on the exact receipt lock, and every relay remained live. No early RPC or relay exit
branch fired. The safe diagnostics were:

| Case | Backend state / wait | Recent query age | Transaction age | Frontend / backend frames; last tags |
| --- | --- | --- | --- | --- |
| Root supplied pool | active / ClientRead | 24 ms | 4,807 ms | 360 / 458; S / Z |
| Aborted reply loss | active / ClientRead | 39 ms | 577 ms | 244 / 348; S / Z |
| Committed reply loss | active / ClientRead | 30 ms | 567 ms | 240 / 348; S / Z |
| Ordinary reopen | idle in transaction / ClientRead | 50 ms | 596 ms | 244 / 348; S / Z |

The relay writes decoded PostgreSQL frames separately; the pinned SQLx/Tokio TCP path leaves
Nagle at the operating-system default, and TCP flush does not combine those frame writes into
an atomic transport operation. The observed ongoing short query cycles support packetization
latency as the minimal transport hypothesis. The test relay now sets `TCP_NODELAY` on both its
accepted client and upstream sockets before startup. This changes no production connection,
frame content, transaction, authority check, receipt assertion, five-second deadline, COMMIT
response-loss control or abort-on-drop ownership.

The existing four public cases and native COMMIT-loss case passed locally after the change,
with formatting and focused strict Clippy also passing. A fresh independent review found no
C1/C2 issue in the change or its validation contract. The earlier macOS pass did not reproduce
the Linux failure, so these local results do not establish the Linux regression fix: the new
integrated heads must pass actual Linux CI before merge or release.

Root repeated the same combined five-case command above on the integrated 0.2.0 candidate
`0d2791a9`: **5 passed; 0 failed; 0 ignored; 549 filtered out; 14.00 s**, 53.03 s overall
including compilation. Formatting, architecture (24 crates), agent configuration, release-version
agreement and Actionlint passed. This primary integrated-tree rerun remains local evidence;
actual Linux CI is still required to close the regression.

The subsequent actual Linux Rust job on `36440cc8`,
[121313905093](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/40241444603/job/121313905093),
passed all five named relay cases. The workflow-primitives target reported
**538 passed; 0 failed; 16 ignored; 680.03 s**, and the complete workspace job
succeeded. This closes the observed relay regression on that tested source;
final integrated-head gates remain mandatory.

## Historical load-harness failures and diagnostic revision

Linux runs [40241230658](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/40241230658/job/121313248319)
and [40241444603](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/40241444603/job/121313905154)
failed `step_transitions_per_second` with `poll: Unavailable` (51.84 s and 53.55 s).
The same scenario passed in run 40241447148. An application-level `Unavailable`
does not distinguish pool saturation, a database timeout or a concurrency defect;
their private driver cause remains unclassified. The failures exposed a synthetic
client that aborted on an outcome the existing SDK contract permits it to retry.
The client-model correction below requires new validation; a passing diagnostic
run alone does not discharge the failed runs.

Diagnostic revision `cdbc8f62` kept all nine configurations, 60 runs per configuration
in CI, the 20-connection pool, two-second acquisition limit, five-second statement
limit, operation calls and completion-count assertions. It adds no retry or runtime
change. A non-printing, harness-owned tracing subscriber reduces driver detail to
fixed cause labels; it never emits the original detail. A separate observer bounds
its database-state snapshot to 500 ms. On failure, pending sibling worker futures
are dropped and the sampler is aborted. External progress buffers preserve all
acknowledged completions; the incomplete row and earlier complete rows are printed
and persisted before the test fails. The incomplete configuration receives no
throughput result or load-level verdict.

Three fresh-context review cycles identified and corrected raw-detail logging,
an unbounded sampler await, waiting for stalled sibling workers, and unbounded
barrier tests. Both barrier scenarios now use named five-second `support::bounded`
limits. Root reran the final source and passed all four diagnostic tests:
**4 passed; 0 failed; 7 filtered out; 0.00 s** (0.52 s including Cargo):

```sh
cargo test -p sagat-tests --test perf_scalability_ha --locked diagnostics::tests -- --nocapture
```

Root also passed formatting, focused strict Clippy, architecture (24 crates, no
cycles), agent configuration and diff checks. These local runs used a private
Cargo target, two build jobs, incremental compilation disabled and
`line-tables-only` dev/test debug metadata on macOS arm64.

The implementer reports two post-implementation mutation checks: restoring the
unbounded sampler await and then restoring `join_all` each produced its named
five-second stage timeout and exit 101. These are mutation proofs, not an initial
TDD RED run. After restoration, root's four-test run above passed.

The implementer's controlled database-outage probe with that diagnostic worker
collection logic exited 101 in 6.78 s, retained 50 partial acknowledgements and
two earlier complete configurations, and emitted a fixed `pool_timeout` label.
This validates diagnostic termination and reporting; it cannot establish the
cause of the original CI failures. Root subsequently passed all nine configurations
with 60 runs each: 1,620 acknowledged transitions, 17.19 s on macOS arm64,
PostgreSQL 18.6. Linux diagnostic run
[40247177253](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/40247177253/job/121331108877)
also passed all nine configurations (38.00 s) and disaster recovery. These passes
do not identify the earlier failures' driver causes. Refs: T010-034, NFR-010-PERF,
T016-054.

## Bounded synthetic-worker correction

The benchmark now follows the existing `PollTasks` retry contract in
`sdks/SPEC.md` sections 5 and 6.2. Only typed `AppError::Unavailable` is retried,
with four total attempts, a 200 ms initial exponential delay, random factor
`[0.5, 1.0]` and 5 s cap. Completion and every other application error remain
strict. Driver logs never select retry policy. Known deadlock observations remain
a post-run integrity failure even if polling recovers.

The initial real-PostgreSQL recovery fixture failed before retries were added:
all 20 execution-pool connections were held, a real poll returned `Unavailable`
after one attempt (2.002 s), and the recovery assertion failed (exit 101, 4.26 s
test runtime). A channel releases the connections only after observing that
failure. The final fixture requires distinct intended completions, terminal
status, journal/replay consistency, no leases and recovery of all 20 slots.
Persistent real-pool exhaustion still fails after four attempts; nonretryable
errors fail after one.

Additional negative controls were observed failing before correction:

- Cancelling during controlled backoff lost the observed attempt/failure
  counters: actual retained attempts 0, expected 1. Caller-owned counters now
  survive cancellation, and the actual sample-recording path has cancellation
  and successful-call controls.
- The exact-state transaction check observed one of two intentionally held
  normal/aborted transaction blocks. A separate aborted-only control then exposed
  the `xact_start`-only check returning success. The oracle now rejects both
  active transactions and aborted blocks, verifies every current backend PID,
  and emits fixed labels instead of database errors.

An intermediate main oracle tried allocating all eight configured 20-slot pools
against the standalone fixture's 100-connection PostgreSQL. It correctly produced
an incomplete report and exited nonzero, but imposed a capacity workload the
benchmark never required. Main now retains every existing idle connection through
`try_acquire`, boundedly reconciles all pool-owned handles, verifies current
backends without creating unused connections, and waits for all handles to return.
The isolated fixture still proves all 20 slots. The declared `run.sh`
`max_connections=400` and standalone fixture limits are unchanged.

All nine workload configurations, 60 runs of three activities in CI, pool size
and acquisition/statement limits remain. Successful-attempt latency per task is
separate from logical-call latency including backoff. Reports expose attempts,
retryable failures, exhausted calls and wall time; throughput includes retry
delays before the final acknowledgement and explicitly excludes final empty-poll
drain. Independent checks cover every started run, unique task/run-node
completions, terminal projections, journal/replay, leases and open/aborted
transactions. Bounded integrity failures preserve earlier rows and acknowledged
progress, print only fixed labels, and fail without a throughput verdict.

The full nonignored target has 16 tests, including the earlier diagnostics:

```sh
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test perf_scalability_ha --locked -- --nocapture
cargo clippy -p sagat-tests --test perf_scalability_ha --locked -- -D warnings
cargo fmt --all -- --check
```

Three fresh-context review cycles found and corrected the accounting, safe-error,
transaction-state and regression-test gaps. The remaining synchronization issue
was isolated: a named five-second observer barrier establishes the exact aborted
backend state with a null transaction timestamp before the negative control runs.
Temporarily removing only the aborted-state predicate then failed deterministically
(`Ok(())` versus `Err(OpenTransaction)`, exit 101, 1.64 s). The core was restored
exactly afterward. This is a post-implementation coverage proof, not an initial
TDD failure or another general review cycle.

Root's final run passed **16 tests; 0 failed; 7 ignored; 11.19 s** with the runtime
role (11.55 s including Cargo). Final formatting, focused strict Clippy (0.35 s),
architecture (24 crates, no cycles), agent configuration and diff checks passed.
The ignored benchmarks and DR scenarios require their explicit scripted runs.
Final integrated-head benchmark/CI/preflight validation remains mandatory before
merge or release; these local results are not a waiver. Historical Linux
`Unavailable` causes remain unknown. Refs: T010-034, NFR-010-PERF, T016-054.

## Primary verification on the integrated 0.2.0 candidate

Root reran the reviewed client on release candidate `5aec324e` using the runtime
role, a private Cargo target, two build jobs and two test threads. The complete
nonignored `perf_scalability_ha` target passed **16 tests; 0 failed; 7 ignored;
11.29 s** (32.76 s including Cargo). The script
`SAGAT_PERF_TRANSITION_RUNS=60 SAGAT_PERF_SCENARIOS=step_transitions_per_second
tests/performance/scalability-ha/run.sh` passed all nine configurations, each
with 60 three-activity runs: **1,620 distinct acknowledged transitions**, with
terminal-run, journal/replay, lease and transaction checks, in **18.03 s**
(236.62 s including compilation). This used macOS arm64 and the script-owned
PostgreSQL 18.6; all rows recorded zero failed attempts and exhausted calls.
This particular run does not establish the cause of historical Linux failures.

`cargo clippy --workspace --all-targets --locked -- -D warnings` also passed on
that integrated tree in **88.86 s**. The full current-head CI gates remain
mandatory; these local results do not replace them.

## Final integrated checkpoint — 2026-10-26

The final candidate `769a7d1d7717f0bc0afeb3731da9328066e3f36a` passed:

- [CI 40259828158](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/40259828158):
  all 11 jobs succeeded, including locked workspace formatting/Clippy/tests,
  architecture, contracts, dependency advisories, five SDKs, Studio/browser,
  complete Compose, native runners and scale/recovery.
- [Independent SDK packaging 40259828175](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/40259828175):
  all five clean consumers passed.
- [Release preflight 40259828227](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/40259828227):
  version agreement, all three native CLI builds, SDK consumers and the full
  recovery-objective drill passed. Publication was correctly skipped on the PR.

The actual final Linux Rust job passed 538 workflow-primitives tests, zero failed,
16 explicitly ignored (1,789.33 s), and all 16 client/integrity tests, zero failed,
seven separately scripted scenarios ignored (15.04 s). The original nine scale
configurations and their integrity checks passed in 51.52 s. The full managed
Compose scenario passed TLS, OpenBao, probe/result/replay, coordinated restore,
restart, retirement fence and outage-without-fallback assertions. This does not
identify either discarded historical startup or private driver failure causes.
Redundant same-head PUSH jobs were cancelled only after the PR replacements
existed; no failing required test was waived.

Actual Gitflow integration:

| PR | Destination | Actual merge |
|---|---|---|
| 45 | develop | `95b5a238367cc4c491dd23e8a4538d8f4b1ec9e3` |
| 47 | develop | `ca18e33ccc3287e85ef70131c75ad332bf642ba5` |
| 50 | develop | `23976ef230fce421346c0bc454ed982b2a895cc6` |
| 51 | main | `bfda342073db71e84f30bdc79ccf3fbc97a7153c` |

Root fetched and compared each feature merge tree with its approved head. The
actual main merge tree equals the tested candidate. Both annotated remote tags,
`v0.2.0` and `sdks/go/v0.2.0`, dereference that same actual main merge. No ref was
overwritten; both tags were pushed atomically.

The back-merge prepared from `develop` integrates that actual main commit. Root
verified its tree before documentation changes equals main, then reran all script
tests with the real controlled Compose case enabled: **43 passed, zero failed,
zero skipped, 2.235 s**. Architecture passed (24 crates, no cycles), agent
configuration passed, and version agreement returned `0.2.0`. The follow-up
changes only task/status/evidence documentation; it closes T016-001 and T016-054
and leaves all eight feature-014 tasks open under ADR-0040.

### Published artifact verification

[Release v0.2.0](https://github.com/ortisun/sagat-octopus-orchestrator/releases/tag/v0.2.0)
was published at `2026-10-26T04:35:04Z` by
[tag workflow 40268722763](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/40268722763),
with all seven jobs successful at source commit `bfda3420`. It is a stable,
non-draft release and was confirmed as the latest release.

Root downloaded every actual attached asset: **15 files**, comprising the three
native CLI archives, all five SDK distributions and their supporting files, SDK
release notes, and `SHA256SUMS`. The 14 artifact entries exactly cover the
downloaded files other than the manifest; all 14 checksums and all 15 GitHub
metadata digests matched. The manifest's SHA-256 is
`d37a7734e43958975aedb39a543bb0afc268f340d71bd6a647b0b8f853229eb3`.

Root ran `gh attestation verify` against **all 15 downloaded files**, including
the manifest: **15 exit codes 0**. Every invocation pinned the repository,
release workflow, actual main merge and tag, and refused self-hosted runners:

```sh
gh attestation verify "$FILE" \
  --repo ortisun/sagat-octopus-orchestrator \
  --signer-workflow ortisun/sagat-octopus-orchestrator/.github/workflows/release.yml \
  --source-digest bfda342073db71e84f30bdc79ccf3fbc97a7153c \
  --source-ref refs/tags/v0.2.0 \
  --deny-self-hosted-runners --format json
```

`FILE` denotes each downloaded asset. Distribution used GitHub Releases only;
no package registry was published and no environment was deployed.
