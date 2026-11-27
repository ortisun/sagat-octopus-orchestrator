# Sixth security re-read of design 015 (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: a7cd7ac1d3cff9e207cd53336914ae8ddac5329b
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 0, Nit 2

Previous re-read:
- 1 (a key missing from the ring at `begin-migration` could never be recorded, so an installation could be left stuck in `migrating`): fixed in the text.
  - docs/designs/015-opentelemetry-and-openbao.md line 147 adds the refusal to the transition guard. `begin-migration` is refused, naming the key ids, while a `local` value of any re-seal inventory entry carries a key id that ring does not hold.
  - Lines 205-209 say `begin-migration` is the only point at which a check value can be recorded, and give that as the reason for the refusal.
  - Task 015-12 at lines 1475-1476 tests the refusal for `secret_versions` and for a second registered entry, and tests that the command succeeds once the ring holds the key.
  - specs/015-opentelemetry-and-openbao/research.md line 45 (D015-021) now says "refused" instead of "recorded".
  - Two new gaps follow from this text (findings 1 and 2). Both fail closed and leak no secret.

1. Nit — The re-seal inventory has no way to report the key ids of its `local` values, and the design does not say whether the new check runs under the state row's lock
   Where: docs/designs/015-opentelemetry-and-openbao.md line 147 and lines 205-209 (the new refusal). Also lines 916-919 (the parts of an inventory entry), lines 877-878 and 886-888 (the `SECURITY DEFINER` list and the generic privilege-audit row), lines 926-927, and line 1650 (Task 015-34, "its two count functions").
   What:
   - An inventory entry registers only `<entry>_sealed_count(custody)` and `<entry>_count_below(key_version)`. Neither one returns key ids. So the refusal "a `local` value of any re-seal inventory entry carries a key id that ring does not hold" cannot be computed over the registered entries.
   - An implementer will either hard-code a query on `secret_versions`, which misses design 040's `audit_sinks` entry and every later entry, or add a function the design does not list. A function the design does not list is not named among the pinned-path functions at lines 877-878. It is also not a `regprocedure` column of `key_authority_reseal_inventory`. If it lives outside the platform's schemas, the generic audit row at lines 886-888 does not reach it.
   - Separately, line 147 does not say the key-id check runs after the state row is taken `FOR UPDATE`. Line 148 says so for `finish-migration`. Failing scenario:
     - The command checks key ids before taking the lock.
     - A serving process, in the middle of a ring rotation, holds a new key k9 that the command's ring lacks. It seals a `local` value under k9. Its trigger takes `key_authority_share()` after the check, while the mode is still `local`.
     - `begin-migration` then takes the lock and commits `migrating`. A `local` k9 value now exists and k9 was never recorded.
     - Every process that holds k9 keeps its gate closed, and `reseal-secrets --to transit` refuses any ring with k9. The installation is stuck in `migrating`, which is the state the fix says cannot happen (line 208).
   Correction:
   - Add a third part to an inventory entry: `<entry>_local_key_ids() RETURNS SETOF text`. It is owned by the schema-owning role, not executable by `sagat_app`, and uses `search_path = pg_catalog, pg_temp`.
   - Register it as a third `regprocedure` column of `key_authority_reseal_inventory`, so the generic audit row covers it, and add it to the list at lines 877-878 and to Task 015-34.
   - Have `key_authority_set_mode` compute the union of these key ids under its `FOR UPDATE`, as `key_authority_count_sealed` is computed, and say so on line 147.
   - Add a test to Task 015-12 in which a seal under an unheld key holds `key_authority_share()` while `begin-migration` waits. Once the seal commits, the command is refused.

2. Nit — The check compares key ids only, so a ring key with the right id but the wrong material passes and records the wrong check value
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 199-201 (an id is only a label the operator chooses) and lines 205-209 ("every key that sealed a value is then recorded").
   What: Failing scenario:
     - The operator runs `begin-migration` with a ring whose entry `k0:<base64>` holds material other than the k0 that sealed the rows. Causes include a stale file, or a label reused after a rotation.
     - The id check passes, so the command records the check value of the wrong material. The material that really sealed the rows is never recorded.
     - In `migrating`, every serving process holds the real k0, whose check value is not recorded, so every one keeps its gate closed. This contradicts "no serving process's gate closes at any step" (line 163) and makes the secret routes unavailable across the fleet.
     - `reseal-secrets --to transit` refuses a ring with the real k0. The command's ring cannot open the rows, because authentication fails.
     - The only way out is a schema-owner hand edit with the trigger disabled, which is what finding 1 of the previous re-read set out to remove.
   Nothing leaks, but the claim at line 207 holds by label, not by key.
   Correction:
   - For each distinct key id that `<entry>_local_key_ids()` returns, have `begin-migration` authenticate (open and discard) one `local` value sealed under that id, using the command's ring key of that id. Refuse, naming the id, when authentication fails.
   - Add to Task 015-12 a test in which a ring holds the right id with other material, and the command is refused.
