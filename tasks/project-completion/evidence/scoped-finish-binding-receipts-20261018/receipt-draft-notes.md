# Scoped Finish native receipt decoder controls — handoff

Read-only source worktree: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`

Frozen HEAD observed at start and end: `7b0ced121d5c2cb4bbc735d513be4894bebd5923` (`feature/016-project-completion`).

Draft artifact:

- `/tmp/sagat-scoped-finish-receipt-controls.rs`
- SHA-256: `5092f5342314ebbd2516ee47325cbd7d18d2763ed9756a4dedf87cf811196b24`
- Intended eventual repository name: `tests/integration/workflow-primitives/scoped_leaf_finish_receipts.rs`
- Intended parent registration, owned by root: `#[path = "scoped_leaf_finish_receipts.rs"] mod receipts;` in `scoped_leaf_finish_native.rs`

No repository file was written by this task. The repository was already being changed concurrently by
root: while this draft was prepared, the parent file gained only the source-controls child registration,
changing its SHA-256 from
`dce6ca3080a7c7a6048346702627322aa05e6d8b8d1d726cff6cb2f15bdb46c1` to
`bdff7b147b0899217f5d78619a7d151564338431e792b2be4f83047f61f7254d`.
The new sibling module does not alter the private helpers used by this draft.

## Frozen relevant inputs

| Input | SHA-256 |
| --- | --- |
| `tests/integration/workflow-primitives/scoped_leaf_finish_native.rs` | `bdff7b147b0899217f5d78619a7d151564338431e792b2be4f83047f61f7254d` |
| `crates/infra-postgres/src/scoped_leaf_finish.rs` | `832811d8977b92ed8ac6d518603bc23e9f5f048c8c140730fa92f29f18143780` |
| `crates/application/src/scoped_leaf_finish.rs` | `f389cf29a48684f24fa76d9cd4da567b1bea6a81ae52bc1f0a7a06ce0038e2e3` |
| `crates/infra-postgres/migrations/20261210000000_capacity_finish_storage.up.sql` | `1e3d93bc918544219fe3c8230f278447c2b1b3aeed23255afe2c55041b3b2dc0` |
| `crates/infra-postgres/migrations/20261212000000_scoped_finish_binding.up.sql` | `a13861e74b7993f3411605538b6552a610db0e3afa973fa6b906f43f0002212c` |
| `tasks/project-completion/scoped-leaf-finish-native-contract.md` | `309184c1126574857230dc2aa305fcb8f4ba803ed98a895260295bce5edf9912` |
| `.specify/memory/constitution.md` | `e086fe6592203707d9a75f381041415c9f3b65d071f7b344c098cadba44c253e` |
| `AGENTS.md` | `51a8e97403a6eb1b043321dc92e4c3fcbfc90d8f487b59ca49a157ac3095b222` |

Recheck these hashes before integration because root is the sole writer and the worktree is active.

## Exact bounded coverage in the draft

The file contains two meaningful tests and reuses the parent's private genuine `certified_flow`,
`seed_group`, command, snapshot, linked-effect and count helpers.

1. Genuine certification and native Finish create one real receipt. A labelled owner fixture then
   writes a schema-valid finite year-10000 timestamp plus a different exact 32-byte fingerprint.
   Native replay must return `FingerprintConflict`, showing fingerprint precedence over timestamp
   decoding. With the genuine fingerprint restored and year 10000 retained, replay must return
   `ContradictoryProjection`. Fixed text binds and `::text::timestamptz` produce year 9999 and
   `0001 BC` positive controls, each of which must return the exact stored `OffsetDateTime`. The
   genuine timestamp and fingerprint are restored with the original `OffsetDateTime` bind, followed
   by exact original-result replay.

2. Genuine two-member selected-holder Finish creates the real `VACANT` result. With the genuine
   fingerprint unchanged, separate schema-valid owner fixtures change the expected/result revision
   pair, expected/result epoch pair, and accepted Run revision; each must return exactly
   `ContradictoryProjection`. A fourth fixture coherently rebinds every scoped discriminator/binding
   field to the existing nonselected genuine scoped member. The result remains VACANT with no holder,
   so the holder/member CHECK remains satisfied; the scoped FK is valid; replay of the original
   command must still return `ContradictoryProjection`. Exact native fields are then restored and the
   original result must replay.

For every owner rewrite, the draft:

- opens a labelled owner transaction;
- disables only `capacity_group_finish_receipts_append_only`;
- leaves every receipt CHECK/FK present, validated and active;
- leaves `capacity_group_finish_receipts_no_truncate` in `ALWAYS` mode;
- reenables the append-only trigger with `ENABLE ALWAYS` before commit;
- proves the exact CHECK/FK and user-trigger catalog inside the transaction before commit and again
  after commit;
- uses fixed SQL statements and positional binds, with no dynamic identifier or parameter assembly.

For every native refusal/replay call, the exact capacity snapshot is unchanged across the call. The
source/certificate/Run/journal/steps/topology snapshot is unchanged. Final receipt/audit/hint counts
remain `(1, 1, 1)`, and the nonselected member remains byte-for-byte unchanged. The native store is
constructed from the parent's explicit runtime pool, whose fixture already asserts `current_user =
sagat_app`; temporary source INSERT is revoked before Finish.

The owner rewrites are decoder fixtures only. They are not native facts and do not prove corruption
resistance against a table owner.

## Evidence status and exact remaining gaps

Per task boundary, no test, Cargo command, formatter, review agent, commit, or external write was run.
The input baseline states that the first store's four actual PostgreSQL tests passed; this draft did
not rerun or extend that evidence. Compilation and all runtime assertions in this draft remain
unverified until root integrates and executes them on one frozen merged tree.

This unit does not cover and must not be used to claim:

- unsupported stored source profile or malformed/impossible stored decoder branches;
- tenant/namespace/originalRun/owner/index qualification beyond the coherent binding refusal here;
- source absence, wrong actual pin or future certified revision precedence;
- metadata-only read qualification or denied source/history probes;
- complete context/chain membership qualification;
- real lock/wait/race barriers, failure injection or deferred-COMMIT rollback;
- unknown COMMIT outcome or process-death recovery;
- raw diagnostic/log/span canary coverage or returned OTel outcome coverage;
- full receipt decoder coverage, general corruption resistance, production caller/runtime grants,
  C0 retention, or broader B1/B2 semantics.

Root owns parent registration, integration, any review, actual PostgreSQL execution, formatter/lint
gates, and the final evidence record.
