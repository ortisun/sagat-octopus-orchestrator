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

## Load-harness failure: diagnosis pending

Linux runs [40241230658](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/40241230658/job/121313248319)
and [40241444603](https://github.com/ortisun/sagat-octopus-orchestrator/actions/runs/40241444603/job/121313905154)
failed `step_transitions_per_second` with `poll: Unavailable` (51.84 s and 53.55 s).
The same scenario passed in run 40241447148. An application-level `Unavailable`
does not distinguish pool saturation, a database timeout or a concurrency defect;
the cause is unresolved and these failures remain release blockers.

The diagnostic revision keeps all nine configurations, 60 runs per configuration
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

The implementer's controlled database-outage probe with the final worker
collection logic exited 101 in 6.78 s, retained 50 partial acknowledgements and
two earlier complete configurations, and emitted a fixed `pool_timeout` label.
This validates diagnostic termination and reporting; it cannot establish the
cause of the original CI failures. The final positive nine-configuration run and
actual Linux cause collection remain pending. Refs: T010-034, NFR-010-PERF,
T016-054.
