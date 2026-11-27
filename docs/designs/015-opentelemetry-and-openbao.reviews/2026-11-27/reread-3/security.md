# Third security re-read of design 015 (2026-11-27)

Doc: docs/designs/015-opentelemetry-and-openbao.md
Reviewed-commit: 1064182d04e05a15fb28ab441c4adcdcfd17c527
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 1, Nit 2

Previous re-read:
- 1 (the custody containment check compares text while the overlap check compares path segments): fixed. In docs/designs/015-opentelemetry-and-openbao.md, lines 428-432 and 1077-1088 compare every path by whole segments and never as text. Every declared prefix must end with `/` and hold no wildcard. A grant is contained only when the prefix's segments are a leading run of its own segments. A grant whose glob could reach another owner's prefix is refused. The fixtures for `{kv_mount}/data/sagat/cred` are at lines 1092-1094, and Task 015-17 at line 1500 matches. The way the platform's custody is listed under segment comparison is new finding 1. The glob fixture is new finding 3.
- 2 (the search-path audit row covers only this design's functions): fixed. Lines 864-871 make the row generic. It covers every function of the platform's schemas and every `regprocedure` of `key_authority_reseal_inventory`. It requires `pg_temp` last and no schema where `sagat_app` holds `CREATE`, checked with `has_schema_privilege`. Line 1433 adds a fixture that proves the row fires.
- 3 (step 9 keeps the ring under the same custody as the backups, and says the way back needs it): fixed. Lines 183-187 hold the archived ring under a separate custody with the backups' retention, and say it is never put back. The phrase "the way back needs it too" is gone. Way-back step 2 (lines 194-202) requires a fresh ring, enforced through `retired_ring_key_ids`, and line 1266 matches. How strong that enforcement is is new finding 2.

1. Major — Under segment comparison, the platform's KV custody lists only `data/` and `metadata/`, so a feature can claim the `destroy/`, `delete/`, `undelete/` or `subkeys/` paths of `sagat/credentials/`
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 1074-1075 (the platform's declared custody), lines 1083-1084 (the overlap rule) and lines 430-432 (Requirement 015-016, "the KV prefix `sagat/credentials/`").
   What: Key/value version 2 exposes one secret under several path types, not just two: `data/`, `metadata/`, `delete/`, `undelete/`, `destroy/` and `subkeys/`. The platform declares only `{kv_mount}/data/sagat/credentials/` and `{kv_mount}/metadata/sagat/credentials/`. Paths are now compared whole segment by segment, and nothing in Requirement 015-016 refuses the other key/value path types. Failing scenario:
   - A feature's allow-list declares the custody prefix `{kv_mount}/destroy/sagat/credentials/`.
   - Its second segment is `destroy`, not `data` or `metadata`, so it overlaps neither platform prefix.
   - The fragment grants `update` on `{kv_mount}/destroy/sagat/credentials/*` and lists that grant in its own allow-list. The grant is contained in the declared prefix, and the glob cannot match a platform prefix, so every check passes.
   - That feature's token can then permanently destroy every version of the platform's credentials.
   - The same works with `delete/`, which hides the latest version, and with `undelete/`, which brings back versions the platform deleted. With `subkeys/`, the token reads each credential's key structure without the values.
   This is the attack of the earlier finding: a fragment grants paths of another custody.
   Correction:
   - Declare key/value custody relative to the mount (`sagat/credentials/`). Have the checker expand every such prefix, the platform's and each feature's, to all six key/value version 2 path types before the overlap and glob checks.
   - Refuse any `{kv_mount}` path whose second segment is not one of those six.
   - Add a negative fixture: a feature declaring `{kv_mount}/destroy/sagat/credentials/` and granting `update` under it.

2. Nit — The "fresh ring" check compares ring key ids, which the operator chooses, not key material, and it records only the ids that still sealed rows
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 194-202, line 128 and line 831. The ring format is in crates/infra-secrets/src/lib.rs lines 66-80.
   What:
   - `KeyRing::parse` reads `id:base64,...`, and the id is any lowercase label.
   - Suppose the operator puts the archived key material back under a new label, for example by copying the ring and renaming `k1` to `k2`. No id is then in `retired_ring_key_ids`, so the gate opens. Current values are re-sealed under a key that also opens the pre-migration backups and may already have been exposed.
   - The recorded ids are only "every ring key id it re-seals away from". So an older key that stays in the archived ring, but sealed no row at migration time, is never recorded, although older backups can depend on it.
   - Line 199 says "The database enforces the freshness", which claims more than the check delivers.
   Correction:
   - Record a fingerprint of the material of every key in the ring (for example a key check value: HMAC-SHA-256 under the key over a fixed label), reported by each process when the migration begins.
   - Close the `returning` gate and refuse `reseal-secrets --to local` when the fingerprint of any key in the ring matches one recorded. Keep the id check as well.

3. Nit — The glob-refusal fixtures are already refused by containment, so they do not prove the glob rule, and the text gives the wrong reason
   Where: docs/designs/015-opentelemetry-and-openbao.md lines 1085-1088 and lines 1093-1094.
   What:
   - Containment compares segments whole, so a grant segment `cred*` or `+` never equals a literal prefix segment. Both example grants are therefore refused as "not contained" before the glob rule runs. Their literal segments do not "sit under its own prefix", as line 1087 claims.
   - A checker with no glob rule, or a broken one, still passes the fixture at line 1094.
   - The sentence also suggests that containment treats a glob as matching, which an implementer could copy. If they did, the checker would accept a grant on `{kv_mount}/data/sagat/cred*` under the prefix `cred/`.
   Correction:
   - Drop "even though its literal segments sit under its own prefix", and state that a wildcard segment never equals a literal prefix segment.
   - Test the glob matcher directly with unit cases (`cred*` against `credentials/`, `+` against one segment), so the second barrier is proven on its own.
