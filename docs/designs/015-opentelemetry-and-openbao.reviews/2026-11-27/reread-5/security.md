# Fifth security re-read of design 015 (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: c06c72e03efa748400b910c630f5cde820bae406
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 0, Nit 1

Previous re-read:
- 1 (the design did not say whether the retired-ring record adds to the stored values or replaces them): fixed. In docs/designs/015-opentelemetry-and-openbao.md, lines 205-208 say the record only grows. `key_authority_record_retired_ring` adds values and never removes or replaces one. The state row's `ENABLE ALWAYS` trigger refuses any update after which either array lacks a value it held before. Task 015-12 at lines 1469-1472 adds the test that a shrinking update is refused. It also adds the two-cycle test: R comes back on the second return, under its old ids and under new ones, and the gate stays closed.
- 2 (`reseal-secrets --to transit` recorded only the ids of the keys it re-seals away from, not their check values): fixed. Lines 209-211 say `reseal-secrets --to transit` refuses a ring that holds a key whose check value is not recorded, the same rule a serving process follows in `migrating`. Task 015-12 at lines 1470-1471 tests it. With this refusal, archived material cannot come back under a new id on the way back. One new gap follows from this text (finding 1).

1. Nit — A key missing from the ring at `begin-migration` can never be recorded, so the new refusal can leave an installation stuck in `migrating`, and the decision record claims a recording that does not happen
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 147 and 202-211 (way-back step 2), line 852 (Storage, the owner-role functions), and specs/015-opentelemetry-and-openbao/research.md line 45 (D015-021, finding 2).
   What: Only `begin-migration` records check values, and it is refused outside `local` (line 147). No passage leads from `migrating` back to `local` (lines 145-150). Failing scenario:
   - The operator runs `begin-migration` with a ring that lacks k0, an older key. Rows sealed under k0 still exist, and the serving processes hold k0.
   - In `migrating`, every process that holds k0 keeps its gate closed, and `reseal-secrets --to transit` refuses any ring that holds k0.
   - So the k0 rows can never be re-sealed, and `finish-migration` (zero `local` values) can never succeed.
   - The only ways out are a schema-owner hand edit of `key_authority_state` with the trigger disabled, or leaving k0 unrecorded. Either one re-opens the hole that finding 2 closed.
   This fails closed, so no secret leaks. But D015-021 says the key "is recorded before it can be re-sealed away from", and nothing in the design records it.
   Correction:
   - Have `begin-migration` refuse, and name the key ids, when any `local` row of any re-seal inventory entry carries a key id that the command's ring does not hold. Every key that sealed a row then gets its check value recorded at the only point where recording is possible.
   - Add that refusal case to Task 015-12.
   - Change D015-021 to say "refused" instead of "recorded".
