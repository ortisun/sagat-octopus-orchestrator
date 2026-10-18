# Collection Finish source metadata replay API proposal

## Scope and provenance

- Repository inspected: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`.
- Branch/HEAD inspected: `feature/016-project-completion` at
  `3710a5b7fc48a9a97334be28176ab4c169eab54c` (including `109908be`).
- API proposal: `/tmp/sagat-collection-finish-replay-metadata-api.patch`.
- Separate incremental test draft:
  `/tmp/sagat-collection-finish-atomic-tests-c3-draft.patch`.
- The API proposal changes only `crates/application/src/primitives.rs`. The C3 draft changes only
  the C2 draft's new `collection_finish_atomic.rs`; it does not include or alter the API patch.
- No repository file was edited. No formatter, build, test, database, commit, subagent or external
  write was run.

## Proposed public contract

The additive constructor is:

```rust
pub const fn from_checkpoint_digests(
    transition: CollectionTransition,
    scope_id: CollectionScopeId,
    result_digest: Digest,
    source_digests: Vec<(u32, Digest)>,
    outbox_id: Uuid,
) -> Self
```

It sets `result_content=None`, `source_digests=Some(source_digests)` and `end_answer=None`. The
caller may add the already prepared End answer with the existing `with_end_answer` method.

The contract deliberately preserves the three existing identities:

- `CollectionFinish::new(...).source_digests()` remains `None` for the legacy unbound command;
- `from_checkpoint_digests(..., Vec::new(), ...)` is `Some([])`, representing an explicitly bound
  empty collection; and
- a nonempty vector remains the ordered `(iteration_index, result_digest)` list.

The source vector is replay identity, not authority. The PostgreSQL adapter already locks and
reads `iteration_checkpoints` and returns `DuplicateMismatch` unless length, order, indices and
digests all match. The constructor does not bypass or weaken that validation, retain content,
change payload accounting, infer sources, or accept aggregate bytes.

The same source file's existing constructor test is extended to assert body absence, an exact
nonempty source list, `Some([])`, and the unchanged legacy `None` case.

## Separate C3 test draft

The C3 patch is incremental to the sealed C2 atomic test patch. Its prerequisites are:

1. apply `/tmp/sagat-collection-finish-atomic-tests.patch` (C2); and
2. apply `/tmp/sagat-collection-finish-replay-metadata-api.patch` after API review.

After the genuine public producer's failed atomic Finish and successful ordinary scheduler retry,
the C3 addition reconstructs the committed Finish identity from observable durable facts:

- the same tenant, namespace, Run, root collection owner and scope;
- the pre-Finish version derived from the committed `collection.finished` sequence;
- the current scheduler fence;
- the committed aggregate digest and finish outbox id;
- the original ordered checkpoint digest list; and
- the successful End answer `17` required by this workflow's declared output expression.

It calls `from_checkpoint_digests`, asserts that the aggregate body is absent and the original
source list is present, then invokes the real `CollectionStore::finish_collection`. The accepted
replay must return `applied=false` and leave the same full tenant payload and retained-accounting
snapshot unchanged. A second command changes the sole checkpoint digest; the existing replay
verifier must return `PrimitiveStoreError::DuplicateMismatch`, again with no snapshot change. This
negative case demonstrates why constructor metadata is not authority.

The C3 patch retains C2's labelled trigger evidence. The writer is observed active and lock-waiting
on the owned gate after the full snapshot, the captured scheduler error must contain
`test-only collection finish abort`, and any `statement timeout` text fails the test. The gate is
released before these assertions.

## Static checks and hashes

- API patch apply check against HEAD `3710a5b7`: succeeded.
- API patch numstat: 57 insertions, 2 deletions in `crates/application/src/primitives.rs`.
- C3 incremental patch apply check against the materialized C2 tree: succeeded.
- C3 incremental numstat: 135 insertions, 1 deletion in
  `tests/integration/workflow-primitives/collection_finish_atomic.rs`.

SHA-256 inputs and outputs:

- constitution: `e086fe6592203707d9a75f381041415c9f3b65d071f7b344c098cadba44c253e`
- spec 014: `eac73844ed31360ac03293474ac5d6213ab0db7bbfc585d5a3155fdf2d4490df`
- plan 014: `ffaa76a78278faaf575278c1425615f6a042dbb2412afb2ad2a690f3fff12930`
- tasks 014: `c25ca47fb054b6fe99cc3402d1ec7577fd3fa4f8add15d2cf2c5a5a9db230f5e`
- input `crates/application/src/primitives.rs`:
  `b093afd25368f32f0aa581491f5fec878c8f60b7dcd7b50203a6c01e9339d3bc`
- proposed `crates/application/src/primitives.rs`:
  `5a6cc8015de7bcb0be444d557091e754e93e046508e8088276fc3261d4b1d171`
- API patch: `2f695c3cca22d6151424c9bcc4b37052c731aedb06cc7f453e9f98b0e9056229`
- C2 input test file:
  `3b0fad9e967e4ceb7eb63b78a92030c5c56ddf48f2f37f39ca8c5c2dda47fdae`
- C3 proposed test file:
  `f015140ef1e812244072e4f1c049142faa5ce80f9ad46f44f5d2a6512ad8b9f5`
- C3 incremental patch:
  `cf9216f88c617de4dd1cb9bd42c9d1b1148c050b7807287ef5f806e1cfb15447`

## Honest limits

These are uncompiled, unformatted implementation drafts. The API shape and call site were checked
against the public fields and current PostgreSQL replay verifier, and only patch application was
checked. They provide no RED/GREEN, build or runtime evidence. The C3 draft proves only the single
root, one-member, successful-End case already bounded by C2; it does not close empty, child,
fan-out, failure-End, instance-limit, general feature 016, or canonical task scope.
