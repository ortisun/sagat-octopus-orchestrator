# Current scoped refusal revalidates exact OpenBao secret versions

Requirement: section11.2 of the accepted collection result/quota correction, current-authority/scanned-secret-vector validation at fresh full restoration. Native runtime role is explicitly enabled in both default/runtime invocations. This is not an owner/runtime privilege matrix.

## Fixture

Actual `openbao/openbao:2.7.1` (local image sha256:6d2b93856e3fcf7b18ad855a0b51eaba474dc8b79cf554379ea32034797d2acf), normal adapter and public managed secret PUT. Publicly produced seventeen-member compensable collection, sixteen retained 250,000-byte outputs, original genuine seventeenth lease and stable done-task-id key.

Owner holds ACCESS EXCLUSIVE on iteration_checkpoints. Exact actual runtime full restoration query is observed blocked by that owner PID with an ungranted AccessShareLock. The barrier is after Run lock acquisition and path/provider scan, not before RunStore.begin. Capture all execution/journal/payload/usage/receipt/outbox/audit state before report. Rotate v1 to v2 publicly while the same query is blocked; verify v1 row unchanged, exact one appended active operation/version, no local ciphertext, only two exact native rotation audits, and exact new provider-value physical quota charge.

After releasing the gate, stale scanned versions must return gRPC UNAVAILABLE with no execution effect. Same key/lease retry rescans actual current OpenBao versions, commits SUCCEEDED plus RUN_DATA_LIMIT without output or a new outbox row, verifies cold journal reconstruction, and exact replay is stable. Captured logs must contain neither submitted secret value. No provider IO is introduced under the full Run lock.

## Actual attempts

- `final-c3`: compiled and reached full restoration after actual public v1 creation and sixteen real reports. Failed before rotation because a helper counted all sixteen earlier receipts sharing task-id-derived key across other instances. Tightened both receipt oracles to exact tenant/scope/key, independently reconstructing lines/16/reserve and original lease hash-prefix. This is an oracle bug; no rotation/stale-version acceptance was reached.
- `exact-receipt-scope`: reached actual public v2 rotation, then failed old-catalog comparison. Type-only read-only PostgreSQL diagnostic shows to_jsonb(version) resolves the numeric version column rather than its similarly named table alias. Changed snapshot alias to version_row so full rows are actually captured. This is an oracle representation bug; no stale-version/retry acceptance was reached.
- `catalog-row-alias`: separate source pins and command logs record the corrected run. Actual focused test passes in default/runtime invocations: 1/1 each (54.38 s including 44.15 s compilation, then 9.67 s; each harness 6.44 s). Both explicitly use runtime_role=true (sagat_app).

The first two attempts each retain 1125 equal before/after source hashes, raw failures and separate revision snapshots. Review artifacts preserve all three bounded review cycles and root reconciliation. No fourth review of the unit. No production code changes, new grant, broader cancellation/blob policy, archived producer, OS process death, fence/terminal race, unknown-COMMIT or general nesting guarantee is claimed. Master Q and canonical014 tasks remain open.

## Final gate inspection

Strict integration Clippy passes 33.65 s, cargo fmt check 5.01 s, architecture 0.39 s. Final native source inputs before/after: 1125, all equal. No extra report outbox, stable exact replay, actual stale-vector UNAVAILABLE, same original lease/key retry and cold fresh store/cache reconstruction all execute in the passing test. Both v1/v2 values are absent from captured trace logs. No production source changes were needed.

Owned disposable PostgreSQL 16.15/max_connections300/pg_stat_statements remains root-owned; OpenBao containers are testcontainer-scoped and released after each test. The earlier type-only diagnostic used a retained failed fixture database; it prints only JSON representation types, never secret metadata/values. Test source snapshots and original C1/C2/C3 patch provenance are retained; raw patch whitespace is not represented as clean source.
