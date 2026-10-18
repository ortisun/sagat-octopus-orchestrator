# Scoped leaf Finish unit 15e atomic-failure draft

Intended destination:

`tests/integration/workflow-primitives/scoped_leaf_finish_atomic_failures.rs`

Register it at the end of `tests/integration/workflow-primitives/scoped_leaf_finish_native.rs`:

```rust
#[path = "scoped_leaf_finish_atomic_failures.rs"]
mod atomic_failures;
```

The draft is a child module and deliberately reuses the parent’s private native helpers. It creates
four isolated `certified_flow` worlds, one for each failure point: audit INSERT, scoped outbox
INSERT, scoped receipt INSERT, and a real `DEFERRABLE INITIALLY DEFERRED` receipt constraint trigger
that raises SQLSTATE 23514 while PostgreSQL executes COMMIT.

Each fault is installed by the fixture owner. The trigger function and nontransactional marker
sequence are explicitly inaccessible to `sagat_app`; the trigger remains able to execute the
`SECURITY DEFINER` function. The sequence advances before the exception and therefore proves the
exact named trigger ran even though every product write rolls back. The draft inventories user
triggers before installation, permits exactly the clearly named test injector, and requires the
original trigger inventory after removal. It never disables a schema guard or changes a runtime
grant.

For each isolated failure it checks:

- the adapter returns confirmed `Storage` with only SQLSTATE 23514;
- the secret diagnostic canary is absent from the storage detail and safe application Display/Debug;
- the test deadline is a watchdog whose expiry panics and is never interpreted as an outcome;
- exact public-source/certificate/Run/journal/step/topology snapshots are unchanged;
- the exact group, all members, receipts, matching audits, and scoped hints are unchanged;
- the explicit nonselected-member snapshot is unchanged and effect counts remain `(0, 0, 0)`;
- the retained command, operation key, and fingerprint remain unchanged;
- after injector removal that same command applies once, creates one linked receipt/audit/hint,
  preserves source and the nonselected member, and then replays without a second effect.

The chosen A1 case starts with two ACTIVE members, revision 7, epoch 3, and the selected member as
holder. A successful retry must yield revision 8, epoch 4, one active member, and VACANT possession.

Scope limits: this draft covers confirmed atomic aborts only. It makes no unknown-COMMIT,
process-death, caller, nesting, C0 retention, admission/accounting, production-grant, OTLP/log-capture,
or broader B1/B2 claim. Canary checks are limited to the returned error’s storage detail and safe
application Display/Debug; diagnostic telemetry remains unit 17. No sleeps, advisory timing claims,
production changes, migrations, or feature task updates are included.

Repository observation while drafting: requested baseline was described as `c332bffc`, while the
read-only worktree reported HEAD `2426083b769b0e05be5346757485a577eaea9c99`. It was initially clean
after the root’s entry-barrier work. During drafting the root concurrently modified
`scoped_leaf_finish_native.rs` and added/registered `scoped_leaf_finish_shard_expiry.rs`; those are
the root’s changes and were not touched here. At handoff the root had committed those changes at
`76df372bd5019012360288270a26b9031fa1d392` and the repository worktree was clean. Source SHA-256
values read for this handoff:

- `scoped_leaf_finish_native.rs` when helper APIs were read: `470a16bd9a1653c401ab92eff0514ae977c8ea1c0c37e83d4b38cfab7aa6f86e`
- `scoped_leaf_finish_native.rs` after the root registered shard expiry: `a77f9def254ba2bedb9a9023651790dd12e8f0a9df665a6d0974ab11d2e4a3c3`
- root-owned `scoped_leaf_finish_shard_expiry.rs`: `4b786c8b53410b8428b19126087fff7d1e14a3014b3b98613a71f333a7091490`
- `scoped_leaf_finish_entry_barriers.rs`: `e12945e2290d18c1a217b4a4058c0c852104ff1d0476038f9f331c8d4aeac31f`
- `capacity_finish_barriers.rs`: `285f073e0dec5e4b512be4542b297b8f7f1b6a1cb0697017a6e70c40dc1adc31`
- `capacity_finish_races.rs`: `b6653733bfc1d75d494a1a4e7b3285940b4f2ac078cb7c27f2f0c462a282b05d`
- `capacity_finish_commit_loss.rs`: `6c0624fadac91b3cb456faa5da43ba78f0906cc6533aa8880c1088c5162a1424`
- `crates/infra-postgres/src/scoped_leaf_finish.rs`: `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780`
- `scoped-leaf-finish-native-contract.md`: `d1062de0be90a5b098f83e1867ff6a4be6067787d328f43654232494bee113f1`
- draft Rust child module: `33bd189ed750cc3c4d82cf0df748c5748c1ad84447f67fdb0348eb3744166285`

Per instruction, this author did not run tests, builds, Clippy, formatting, or compilation. The root
must inspect the draft against its frozen merged tree, install/register it, and run the real gates.
