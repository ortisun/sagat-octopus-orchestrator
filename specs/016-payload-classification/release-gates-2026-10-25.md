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
exact gate blocker and receipt/advisory-lock markers. No TCP setting, timeout, production
transaction, authority or receipt outcome changes.

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
