# PgScopedLeafFinishStore draft notes

Source baseline: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion` at
`3f6b2973d538c921c5f87ddf3d2707da333321d0`.

Draft module: `/tmp/sagat-native-scoped-finish-store.rs`.

This is a design/implementation draft only. Per assignment, it was not copied into the repository,
formatted, compiled, linted, or tested. It makes no executable or release claim.

## Exact eventual registration

In `crates/infra-postgres/src/lib.rs`, adjacent to the existing capacity module declarations:

```rust
mod scoped_leaf_finish;
```

and adjacent to the existing capacity store export:

```rust
pub use scoped_leaf_finish::PgScopedLeafFinishStore;
```

The module itself is intended to be placed at
`crates/infra-postgres/src/scoped_leaf_finish.rs` unchanged except for fixes found by review and the
root agent's test-first integration.

## Deliberately unresolved integration and release work

1. No production caller or scheduler discovery path exists. The draft does not add one.
2. `sagat_app` cannot insert the four scoped extension columns in
   `capacity_group_finish_receipts` (`member_kind`, `original_run_id`, `root_owner_node`,
   `iteration_index`). Migration `20261212000000_scoped_finish_binding.up.sql` intentionally keeps
   them unavailable. Native positive tests need only the reviewed disposable-fixture grants; no
   migration or startup grant should be inferred from this draft.
3. The canonical source certificate remains runtime read-only. This consumer calls only
   `read_canonical_leaf_success`; it does not call the certifier or grant certificate INSERT.
4. Capacity group/member creation, admission, ownership transfer, usage accounting, general nested
   context support, source retention release gates, and mixed/old-writer compatibility remain out
   of scope.
5. The source tree must contain the current application `ScopedLeafFinishStore` API, the
   crate-private canonical reader, the raw chain/scheduler helpers, and the canonical scoped hint.
   They were present at the inspected commit, but the root agent must verify the final merged tree.
6. The root agent must write the required failing tests first, review any resulting edits, then run
   formatting, strict Clippy, focused real-PostgreSQL/runtime-role tests, architecture checks, and
   the required regressions with a private external `CARGO_TARGET_DIR`. None was run here.

## Review-sensitive points in the draft

- Fingerprinting occurs before `BEGIN`; every later error owns and rolls back the transaction.
- Discovery is namespace-qualified and the immutable member binding is checked again under G.
- Lock order is applicable chain, original Run, selected G `NOWAIT`, binding reread, current
  scheduler fence, then flat-context/source/receipt work.
- Parent or any actual chain membership returns `UnsupportedRunContext`; the post-guard probe takes
  no late chain lock.
- The canonical reader's documented emitted subset is mapped explicitly. Every other currently
  existing source error becomes a fixed sanitized storage diagnostic, and the match has no wildcard.
- Receipt fingerprint is decoded and compared before unrelated receipt fields. Matching receipts
  undergo the complete scoped discriminator, counter, possession, safe timestamp, and UUID checks.
- Selected/header mutable state is not decoded until after a receipt miss and ordered CAS checks.
- The original Run trace is consumed only for first application from the two safe SQL projections.
- All five writes use the reviewed order: G, selected member, audit, scoped hint, receipt.
- SQLx failures expose only a valid five-character ASCII-alphanumeric SQLSTATE or a fixed message;
  neither the generic `identity::store_error` nor `identity::insert_audit` is used.
- COMMIT classification matches the existing conservative ChildRoot boundary. All other COMMIT
  failures return `CommitOutcomeUnknown` with the original operation key.

## Items that must be resolved by compilation/review

The draft intentionally was not compiled. The first integration pass must confirm formatting and
workspace lint compatibility for the generic projection helpers, async trait implementation syntax,
SQLx bind inference for explicit NULL receipt columns, and the exact current public paths of imported
types. These are compile-level questions, not settled evidence here.

The root review should also verify the 23 positional receipt binds against the 24-column INSERT
(one literal `SCOPED_PATH` plus 23 parameters), and exercise the deferred holder FK at COMMIT under
the actual disposable PostgreSQL schema.
