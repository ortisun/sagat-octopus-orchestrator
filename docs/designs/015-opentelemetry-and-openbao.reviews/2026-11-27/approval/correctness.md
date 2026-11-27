# Approval read of design 015: correctness (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: a6002f91db92d8df89bb5487d6e9e6cbaa20ef05
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 1, Nit 0

Previous re-read:
- 1 (the retry reopens with only the current wrapped key, so a re-key followed by a retirement fails a healthy row): fixed. In `docs/designs/015-opentelemetry-and-openbao.md`, lines 1286-1290 now say the retry opens the whole current sealed value of the second read (wrapped key, nonce and ciphertext) and returns only that value's plaintext. The Task 015-19 barrier test gains the variant that runs `secrets rekey --issued-before` and then the retirement, with both readers succeeding (lines 1724-1726).
- 2 (the sample-open guard of `begin-migration` cannot run inside `key_authority_set_mode`): fixed. Lines 145-157 make the command's first statement take the state row `FOR UPDATE`. In that same transaction it reads the key ids and one sample per key id and entry through `<entry>_local_sample` (lines 1006-1011). It opens them in its own process, then calls `key_authority_set_mode` with key ids and check values only. The transitions table (line 161) matches, and Task 015-12 adds the barrier test with a `local` re-seal under a new key id (lines 1603-1606). The guard is still incomplete when one key id holds values sealed under two materials (new finding 1).
- 3 (after a full return, a replay of a binding-version-1 intent from before the migration is answered `IdempotencyKeyReused`): fixed. `last_finish_migration_at` is set by `key_authority_set_mode` when it commits `transit` (lines 933-935). A replay of a binding-version-1 intent created before that time is answered as an expired request id, and this check runs before the binding comparison (lines 487-494). Task 015-16 replays a pre-migration intent after a full return under a fresh intent key file (lines 1687-1688). No binding-version-1 intent can be created while the mode is `migrating`, so nothing can fall on the boundary.

1. Major — One sample per key id cannot prove that every `local` value of that id opens, so the stranding of the previous finding 2 is still reachable when an id holds two materials
   Where: `docs/designs/015-opentelemetry-and-openbao.md` lines 148-150 and 161 (one sealed `local` value per key id and entry, "so a key with the right id and other material is refused"), lines 1006-1007 (`<entry>_local_sample(key_id)` returns one value or no row), lines 251-256 (the claim that "no installation can be left in `migrating` with values that no permitted ring may re-seal"), lines 260-262 (in `migrating`, `reseal-secrets --to transit` refuses a ring holding a key whose check value is not recorded), and Task 015-12 at lines 1600-1606.
   What: The guard opens one arbitrary value per key id. It cannot tell an id whose values were all sealed by one material from an id whose values were sealed by two. This is the previous finding's scenario with the serving processes in it:
   - The serving processes and the operator running `begin-migration` share ring A, whose newest key is K3 with material a. The registry check passes, because every published check value is in ring A.
   - A second operator's one-shot `reseal-secrets` (no `--to`, mode `local`) uses ring B, whose newest key is also called K3 but holds material b. It re-seals some rows under K3/b.
   - The serving processes keep sealing new writes under K3/a, so id K3 now holds values of both materials.
   - `begin-migration` takes the lock, and `<entry>_local_sample('K3')` happens to return a K3/a row. It opens, the guard passes, the check value of a is recorded, and the mode becomes `migrating`.
   - `reseal-secrets --to transit` with ring A fails on every K3/b row. Ring B is refused, because the check value of b was never recorded (lines 251 and 260-262). There is no transition from `migrating` back to `local`, and `finish-migration` needs zero `local` values. The installation is stuck in `migrating`.

   Whether the command is refused depends on which row the sample function returns. The tests at lines 1600-1606 put only one material under each id, so they pass with this defect.
   Correction: Do one of two things.
   - Let `reseal-secrets --to transit` add the check value of each key of its ring to `retired_ring_check_values` in its first transaction, as it already adds the key ids (lines 248-249). The record only grows, so this keeps the return's refusal sound, and the K3/b rows can then be re-sealed with ring B. Change line 251 ("the only point at which a check value can be recorded") and lines 260-262 to match.
   - Or have the guard open every `local` value of each key id under the lock, not one sample, and accept the longer lock hold.

   Add a Task 015-12 test that seals values under one key id with two materials and expects either `begin-migration` to be refused whichever row is sampled, or the passage to finish after a re-seal with the second ring.

Confidence: the three statuses are confirmed in the design text. The new finding is plausible: it follows from lines 148-150, 251 and 260-262 read together, and I could not run anything.
