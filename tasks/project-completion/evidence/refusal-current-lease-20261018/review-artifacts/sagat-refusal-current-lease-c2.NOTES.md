# Current refusal lease C2 draft notes

- Repository source baseline remained the C1 file from HEAD `d96b943763c93eff68ac4fa145d4c45c43a7d327`; final observed root-owned repository HEAD was `22fe7c2680ab834f1aac6b55f47230256b2311c7` after unrelated legacy coordinator work.
- C1 patch remains unchanged at SHA-256 `1e2f03b508dfabba49585b14a8c55af1c7ffc552bc4c65b7f36fd15c7d089406`.
- Sealed C2 full patch SHA-256: `53be8294e87d168789760187091c5f12f194290633282d9665b858fcae1b36ca` for `/tmp/sagat-refusal-current-lease-c2.patch`.
- Sealed C1-to-C2 delta SHA-256: `14dd7a50b51d46d7e8250710b5f964cd4bd34f0ea1cb321d6a753e237f27a4d0` for `/tmp/sagat-refusal-current-lease-c1-to-c2.delta.patch`.
- C2 mirror SHA-256: `25626e71241310ec87bfe83974756663009c4ff1c09ca476dcfaa680bdcea912`.
- Repository path changed by both patches: `tests/integration/workflow-primitives/refusal_secret_versions.rs` only.

## C2 correction ranges

- Complete tenant authority snapshot helper: lines 656-676.
- Exact retained-binding timestamp reader: lines 678-689.
- Exact binding/session mutation oracle: lines 691-762.
- Existing independent testcase, extended authority capture and assertions: lines 1104-1611; C2-specific call sites are lines 1281, 1415-1420, 1443-1452, 1539, 1563 and 1581-1584.

## Accepted finding corrected

The C1 execution snapshot intentionally excludes worker sessions and binding metadata, and its final authority assertion used projected pattern/actor/pool/membership/catalog readers. C2 additionally captures complete `to_jsonb` rows for every tenant binding and worker session, plus complete tenant binding-pool and membership rows.

The mutation oracle partitions the complete binding rows by the exact namespace and `scope-worker` subject, requires two original target rows and one replacement target row, and requires every other complete binding row to remain equal. It partitions complete session rows by the original identified session UUID, requires that exact row to exist once before and be absent after, and requires every other complete session row to remain equal. Complete binding-pool and membership arrays remain equal. Existing exact pattern, updater, pools, physical quota delta, trace-bound audit and execution snapshot assertions remain in place.

The retained `line.reserve` binding row's native `updated_at` must fall within the actual public PUT request bounds. The raw complete postmutation authority snapshot is retained and compared again after the original RPC returns `STALE_ATTEMPT`, excluding session recreation or any later binding/session metadata change.

## Scope and execution

This is a temporary correction only. C1 artifacts were not edited. No repository file was modified. No tests, builds, formatter, containers, database mutations, commits, external writes or subagents were run. No runtime result or acceptance is claimed.
