# Scoped Finish returned-observations draft handoff

Draft only. I did not edit the repository, run tests, build, format, create commits, or write
evidence. The root is the sole integrator and test executor. No result in this handoff is an
executed claim.

## Files

- Draft module: `/tmp/sagat-scoped-finish-observations.rs`
- Draft SHA-256 at handoff: `6e9204f2b3de9794a75ddbc3eb34902e465d52c4bc8ad63e13c971b2272d769c`
- Three `current_thread` tests only:
  1. filtered child span cannot overwrite caller outcome/status;
  2. six actual returned outcomes with exact metrics/spans/logs/redaction and real committed
     unknown-COMMIT reply loss;
  3. dropped native call blocked on the original Run has no returned observation/effect, releases
     the transaction, and the unchanged command subsequently applies through a fresh runtime pool.

## Integration

Copy the draft to
`tests/integration/workflow-primitives/scoped_leaf_finish_observations.rs`, then add this child module
at the end of `scoped_leaf_finish_native.rs` after the root's current `commit_loss` registration:

```rust
#[path = "scoped_leaf_finish_observations.rs"]
mod observations;
```

The draft deliberately imports the established relay directly from
`crate::capacity_finish::commit_proxy`. Its unknown-COMMIT test installs and owns its deferred
advisory COMMIT gate. It does not import or depend on the concurrent
`scoped_leaf_finish_commit_loss.rs` unit.

The canary source is a real published/executed/certified OrdinaryLeafV1 source. The draft locally
renames the public root owner, body and body activity to valid declared identifiers and supplies
distinct input/output canaries before native certification. It does not create an owner-written
certificate or use a schema-forged owner as positive evidence. Capacity group/member creation
remains the explicitly labelled dormant owner fixture already used by this native family.

Suggested focused command for the root after integration, using the repository's required private
target directory and resource checks:

```sh
RUST_TEST_THREADS=1 cargo test -p sagat-tests --test integration_workflow_primitives --locked \
  leaf_certification_native::scoped_finish::observations:: -- --nocapture
```

Then run the root-selected scoped native family, strict Clippy, formatting and architecture gates.
These commands are suggestions only; none was run by this draft author.

## Contract coverage in the draft

- Actual returned labels: `applied`, `replayed`, `busy`, `refused`, `storage_error`, `unknown`.
- Shared fixed metric names; outcome is the only non-histogram label; no identity label; transition
  counter is unlabeled and increments only for the acknowledged first application.
- A committed unknown result leaves the transition metric unchanged, and the exact receipt replay
  also leaves it unchanged.
- Each returned call exports one child span under the supplied caller trace/parent and one fixed
  DEBUG completion event. Span attributes are exactly tenant, namespace, group, member, original
  Run and finite outcome. Chain, owner/index, holder and operation key are explicitly absent.
- `storage_error` and `unknown` set error status; the four semantic/confirmed outcomes leave status
  unset. Exported status messages, events and links are empty.
- Redaction covers a secret-shaped scheduler owner, operation keys, raw PostgreSQL message/detail/
  hint, actual scoped owner/body identifiers, genuine source input/output values, and committed
  audit/outbox IDs across all captured logs, capacity metrics and actual Finish spans. The returned
  application error's Display/Debug is also checked against the raw server canaries.
- The filtered-span test proves a real closed runtime pool still records the storage metric without
  altering caller fields.
- The abandoned-call test witnesses the real original-Run lock wait with `pg_stat_activity`, drops
  the in-flight future, closes the runtime pool, proves the waiter stopped and the Run is lockable
  NOWAIT, observes no receipt/audit/hint or returned metric/log/outcome, and finally applies the
  unchanged command through a new explicit runtime-role pool.

## Fresh source registration

The requested worktree moved while this read-only draft was in progress: it began at
`76df372bd5019012360288270a26b9031fa1d392` and was at
`80fc39176e3440d351eb4d8fc1179c8047a090aa` for the final source read. At that final read the root
owned these uncommitted paths: `tasks/project-completion/todo.md`, the native module registration,
the new scoped commit-loss test, and its evidence directory. I did not modify them.

Final-read SHA-256 values:

| Source | SHA-256 |
| --- | --- |
| `tasks/project-completion/scoped-leaf-finish-native-contract.md` | `c7f4b2b7d278acda07b0fcbb33f31c3c6b16c4454bfb7cb068e7839f84d5a81d` |
| `crates/application/src/scoped_leaf_finish.rs` | `f389cf29a48684f24fa76d9cd4da567b1bea6a81ae52bc1f0a7a06ce0038e2e3` |
| `crates/infra-postgres/src/scoped_leaf_finish.rs` | `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780` |
| `crates/telemetry/src/capacity.rs` | `3eac1a112d0f6046b7888e7f5e7301b3d7c6027471087d039f2b78e4cf962158` |
| `tests/integration/workflow-primitives/capacity_finish_observations.rs` | `951e728babd41e593e899fb1a6c68b4e67d2e865705a4a87e9aa1aee7dcb8f22` |
| `tests/integration/workflow-primitives/leaf_certification_observations.rs` | `111054fb5fc2ef341914ec8118b2500430634d99039ce3dc7319e1827b663bfb` |
| `tests/integration/workflow-primitives/capacity_finish_proxy.rs` | `98838f211342a30320300786c474e46f0cd87005a9d5674e1787d07a5b662b23` |
| `tests/integration/workflow-primitives/scoped_leaf_finish_native.rs` | `d14d3907ed41fde38b7ea2bd0847309f9e821daa73a8df10987e92ce4a2b450f` |
| `tests/integration/workflow-primitives/scoped_leaf_finish_commit_loss.rs` | `fae87f9977bbb78c61fa3d92f01a7d7c326ebd69b11a1b9603cf923845043f73` |

Recheck these before applying the draft because the root's concurrent integration may continue to
move the module tail or helpers.

## Deliberate boundaries

No product code, production caller, runtime grant, public endpoint, source creator, group lifecycle,
general nesting, durable intent, process-restart claim, C0 retention claim, release gate, task check,
or broader B1/B2 acceptance is proposed by this draft.
