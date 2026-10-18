# B1b.2c.1b ordinary-child incoming parent flags — draft handoff

Status: implementation draft only. The repository stayed read only at `2d86483f03efa683e13df8d78bb1338f5170602f`. I ran no formatter, build, Clippy, integration test, runtime-role test, agent, commit, or external write. The only validation was a read-only `git apply --check` of the helper patch; it succeeded without output.

## Artifacts and install points

- `/tmp/sagat-child-parent-flags.rs` → install as `tests/integration/workflow-primitives/leaf_certification_child_parent_flags.rs`.
- `/tmp/sagat-child-parent-flags-helper.patch` → apply to the accepted `leaf_certification_child.rs`. It parameterizes the real public parent fixture by `CancelPolicy`, retains `ordinary_child_source_boundary(label)` as the Propagate wrapper, adds `ordinary_child_source_boundary_with_policy(label, cancel_policy)`, and registers the new child submodule.
- No change to `leaf_certification_native.rs`, production code, grants, caller wiring, DTOs, SDKs, or public contracts is required.

The policy labels are `pf-p-cf`, `pf-p-pf`, `pf-d-cf`, `pf-d-pf`, `pf-r-cf`, `pf-r-pf`, and `pf-commit`. With the accepted `ocs-b-`, `ocs-c-`, and `ocs-p-` prefixes plus UUID, every generated public Slug stays within 3..63 characters.

## Focused tests

1. `leaf_certification_native::child_source::parent_flags::public_parent_flags_and_first_child_certification_serialize_in_both_orders`
   - Six fresh real public fixtures: Propagate, Detach, RequestCancel × certifier-first and parent-first.
   - The accepted helper creates the actual unchained ordinary child, closes `lines/0`, leases `later/0/work`, leaves it unfinished, and discovers the source command after that lease advanced the child revision.
   - A child-qualified owner transaction records PID H and holds the child Run. Both native certifier and public HTTP CANCEL futures are pinned and polled locally.
   - Bounded observers require H → leader → follower using distinct PIDs, `sagat_app`, `wait_event_type='Lock'`, the exact certifier SELECT or policy writer, and the exact qualified child predicate. While the queue remains blocked, a second exact `pg_stat_activity` read records H/leader/follower and asserts their database, role, state, query, wait type/event, and blockers before emitting one payload-free stderr JSON witness. The parent Run `FOR KEY SHARE NOWAIT` probe must report 55P03.
   - While blocked: flags remain false, parent command state is unchanged, and selected certificate/audit counts remain zero.
   - After rollback of H: both operations succeed. The first certification is not a replay and retains the fixture's post-lease CAS. Exact policy-specific parent status, child flags, kid projection, event sequence/type/node/detail, audit, idempotency record, HTTP 202 body/revision, and `run.finished` outbox are checked.
   - The selected source, child Run excluding only the two flags, entire child journal, all steps, exact leased later row, and child revision remain unchanged. Fact and audit change once. The INSERT grant is revoked before a cold runtime pool/cache/store returns the exact canonical record and leaves the full snapshot unchanged.

2. `leaf_certification_native::child_source::parent_flags::incoming_parent_flag_waits_for_the_child_certifiers_actual_commit`
   - One fresh Propagate fixture and a generated advisory key.
   - A test-only DEFERRABLE INITIALLY DEFERRED constraint trigger gates the real certificate INSERT transaction at COMMIT.
   - The certifier uses an isolated one-connection `sagat_app` pool with a generated unique `application_name`. The test captures C only when `query='COMMIT'`, C is waiting on Lock, and gate PID G is in `pg_blocking_pids(C)`.
   - The public HTTP CANCEL future is then polled until its exact legacy child UPDATE is waiting behind C while C remains blocked behind G. G, C, and parent PID P are distinct. A second exact `pg_stat_activity` read records G/C/P and asserts their database, role, state, query, wait type/event, and blockers before emitting the case's single payload-free stderr JSON witness.
   - Fresh NOWAIT probes prove child `FOR KEY SHARE` and scheduler shard `FOR NO KEY UPDATE` conflict, scheduler shard `FOR SHARE` succeeds, and parent `FOR KEY SHARE` conflicts. Source metadata and the parent command remain externally invisible.
   - Only G's transaction is rolled back. Certifier and CANCEL must both succeed. The full Propagate oracle and cold exact replay then run, the trigger/function are removed, the INSERT grant is revoked, and the dedicated pool is closed.

## Shared oracle and review slices

The source file is intentionally divided into bounded review slices:

- Lines 1–512: immutable-source, execution, parent-command, exact three-PID recording, queue and privilege helpers.
- Lines 513–759: one shared exact positive oracle plus cold canonical replay.
- Lines 760–973: the six policy/order cells.
- Lines 974–1125: deferred COMMIT setup, PID chain observers and guard probes.
- Lines 1126 onward: the real COMMIT-lifetime scenario.

`ChildState` independently fingerprints the exact selected `lines/0` evidence, qualified certification row, qualified certification audit, child Run with only `parent_cancel` and `parent_settled` removed, the full child journal, every child step, and the exact `later/0/work` row. Thus a flag-only writer is allowed to change the two flags, while any revision, `updated_at`, journal, source, lease token hash, lease owner, lease/deadline, attempt, generation, step, fact, or audit drift is observable.

The source record itself is checked through the accepted helper against the actual scope, iteration, checkpoint, returned step, owner step and all five exact journal references. The persisted pin/revision/audit relationship is also checked. Cold replay compares the complete returned record and the complete snapshot, including the source audit.

## Contract boundary

This draft characterizes only the existing legacy incoming-parent flag family for one root parent with one active child and an unactivated End. It proves first native OrdinaryLeafV1 source certification in both child-row queue orders and one certifier-first actual COMMIT order.

It does not establish own-child cancellation/drain, terminal fanout, detached later completion, both COMMIT orders, multiple children, nested parents, chains, scoped Finish activation, capacity behavior, all parent policies in other topologies, fairness, or timing. The test uses no scheduler drive after the accepted source boundary and does not finish the leased later work.

The source INSERT privilege is a disposable test-only grant. All certifier calls and the public API writer run as `sagat_app`; no production grant changes. The owner pool is used only for fixture DDL, snapshots, lock observation, and NOWAIT probes.

## Provenance

Reviewed decision: `/tmp/sagat-child-parent-flags-decision.md`, SHA-256 `6c7b12d6d7d099ac7102623b5d5422909abd48c0640fb5108de06cc1c53baf41`, fresh review C2 `None`.

Committed contract at HEAD: `tasks/project-completion/leaf-child-parent-flags.md`, SHA-256 `ef13a09ce9e6f171e9a1f0e9d3b3ad467e882bcd6df1dad0993393e6332ed7df`.

Draft hashes before this notes file was written:

- `/tmp/sagat-child-parent-flags.rs`: `2fc804439962160668fd282da54f73a971642a65f7425f32e3257cd200247c01`
- `/tmp/sagat-child-parent-flags-helper.patch`: `5e03de7ab8299b328c83ca34ac47f522ae01e5eaedf4e8ffac04adf1ebd599b4`

Mandatory/current read hashes:

| File | SHA-256 |
| --- | --- |
| `AGENTS.md` | `51a8e97403a6eb1b043321dc92e4c3fcbfc90d8f487b59ca49a157ac3095b222` |
| `.specify/memory/constitution.md` | `e086fe6592203707d9a75f381041415c9f3b65d071f7b344c098cadba44c253e` |
| `specs/014-workflow-primitives/spec.md` | `0981e53290d2599919b2b0028ceac85a04d1ec0a8e9b94c5f52d98793b808a2b` |
| `specs/014-workflow-primitives/plan.md` | `33580b895a52c01113eacaf9503b95b88d5cb7b0204136fae62206c2c62ecd91` |
| `specs/014-workflow-primitives/tasks.md` | `c25ca47fb054b6fe99cc3402d1ec7577fd3fa4f8add15d2cf2c5a5a9db230f5e` |
| `tests/integration/workflow-primitives/leaf_certification_child.rs` | `c797a0e6eb95e1db90ca93bf56137f408990c6bc4f1e07f664e1059ee3d532fd` |
| `tests/integration/workflow-primitives/leaf_certification_native.rs` | `679089900793c3e1949c834747aea66900f419f0116a508db61350b6cbb574c7` |
| `crates/infra-postgres/src/execution.rs` | `fccef861b6c07b963a22c8e15d1f01401fc063870dd77d1ed414638cc186f22f` |
| `crates/infra-postgres/src/leaf_certification.rs` | `d39f67bc5a41fdb00b2e2d3eff05c1705f27e22583abcc9f1fb7ecfcbe744669` |
| `tests/support/scope_flow.rs` | `c6722902a332ae5c693f06598b0b21d3652afa5ace63a626aba3931a532fa821` |

The accepted .1a baseline evidence supplied by root was: focused 2/2 default, focused 2/2 least privilege, Clippy, formatter and architecture checks passed, with 1104 frozen inputs unchanged. Those results were not rerun here.

Root's initial integrated default invocation against the pre-amendment draft used 1105 frozen inputs. Both baseline tests passed. Each new test reached and passed its first Propagate queue/COMMIT and policy assertions, then failed only because the draft expected uppercase `command_dedupe.resource_id='CANCEL'`; the existing `RunOperation::name` stores lowercase `cancel`. The amended oracle now expects the observed lowercase value. The six-cell loop stopped in its first Propagate cell, so that invocation is not evidence for the remaining five cells. Root's initial strict Clippy invocation reported only the eight-argument `observe_order` and the nine-field expected-outcome tuple; this amendment uses two operation tuples and named expected event/outcome structs without lint suppression. The amended draft has not been executed or linted.

