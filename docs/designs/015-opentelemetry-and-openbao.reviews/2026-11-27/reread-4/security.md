# Fourth security re-read of design 015 (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: a4617656853158e5d909302682eb1ddf1104bde5
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 0, Nit 2

Previous re-read:
- 1 (the key/value custody listed only `data/` and `metadata/`, so a feature could claim the `destroy/`, `delete/`, `undelete/` or `subkeys/` paths of `sagat/credentials/`): fixed. In docs/designs/015-opentelemetry-and-openbao.md, lines 440-443 (Requirement 015-016) and lines 1089-1098 (Technical Plan 7) declare key/value custody relative to the mount in `kv_custody`. The checker expands every owner's prefix, the platform's included, to all six key/value version 2 path types before the overlap and glob checks. It refuses a `custody` entry that names `{kv_mount}`, and it refuses any key/value path whose second segment is not one of the six. Lines 1115-1118 and 1123 add the negative fixtures: a feature declaring `{kv_mount}/destroy/sagat/credentials/`, the same feature granting `update` under it, and a grant on `{kv_mount}/config`. Task 015-17 matches at lines 1535-1537.
- 2 (the "fresh ring" check compared ring key ids only, and recorded only the ids that still sealed rows): fixed. Lines 198-208 record a check value (the first 16 bytes of HMAC-SHA-256 under the key over `sagat/ring-check/v1`) and the id of every key in the ring the command holds, whether or not it sealed a row. In `migrating`, a process whose ring holds a key with no recorded check value keeps its gate closed. In `returning`, a process whose ring holds a recorded check value or id keeps its gate closed, and `reseal-secrets --to local` refuses that ring. Lines 128, 147, 842-847, 967-968 and 1462-1464 match. The claim "the database enforces the freshness" is gone. The new text has two smaller gaps, findings 1 and 2.
- 3 (the glob-refusal fixtures were already refused by containment): fixed. Lines 1102-1103 state that a wildcard segment never equals a literal prefix segment, and the wrong reason is gone. Lines 1108-1114 call the glob rule a second barrier and test the matcher on its own with unit cases (`cred*` against `credentials`, `+` against exactly one segment, a trailing `*`, a literal). Task 015-17 at line 1537 matches.

1. Nit — The design does not say whether the retired-ring record adds to the stored values or replaces them, so a second migration could erase the first ring's check values
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 202-205 (way-back step 2), lines 842-847 (Storage, `key_authority_record_retired_ring`) and lines 1462-1464 (Task 015-12).
   What: `begin-migration` "records ... in `retired_ring_check_values` and `retired_ring_key_ids`", but the text never says whether it adds to the stored values or replaces them. Failing scenario:
   - An installation migrates with ring R and returns with a fresh ring R2.
   - Later it migrates again. If the function replaces the columns, only R2's values remain.
   - On a second return, the operator can put back the archived R, under its old ids or new ones, and every gate opens.
   - Current values are then re-sealed under a key that also opens the backups taken before the first migration.
   Correction:
   - State that `key_authority_record_retired_ring` only adds values and never removes any.
   - Have the `ENABLE ALWAYS` trigger refuse an update that shrinks either array.
   - Add a test with two full cycles that brings R back on the second return and expects the gate to stay closed.

2. Nit — `reseal-secrets --to transit` records only the ids of the keys it re-seals away from, not their check values, so a key that only the command's ring holds can come back under a new id
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 205-207.
   What: The `migrating` gate rule covers "every key a serving process holds", but it does not cover the command-line ring of `reseal-secrets`. Failing scenario:
   - The operator runs `reseal-secrets --to transit` with a ring that also holds an older key k0, needed for rows that no serving process still opens.
   - k0's id is recorded, but its check value is not.
   - Backups taken before the migration still hold rows sealed under k0.
   - On the way back, k0's material under a new id passes both checks.
   Correction: have `reseal-secrets --to transit` refuse a ring that holds a key whose check value is not recorded, the same rule a serving process follows in `migrating`. The alternative is to record the check value of each key the command re-seals away from, beside its id.
