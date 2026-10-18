# Residual current-schema decoder inventory

Read-only Sol/high inventory atcbfb620d, no tests executed by agent. Current implementation
and contracts are unchanged at21139428. The prior integrated native12/12 pass and its exact
commands/reviews are retained separately, not inferred by this inventory.

| Boundary | Existing enforcement/evidence | Residual current-schema gap |
| --- | --- | --- |
| Canonical source | UUID/types/profile/digests/sequences/time/null CHECKs; sharedreader21pass/1privateprobe; actual source3 missing/wrong/future pins | None; unsupported profile/malformed shapes require changed schema |
| Member binding | Total variants, ordinary owner/index and qualified Run FK; native qualification3 | None after opaque discovery controls |
| Run/header/state | Typed columns, counters/trace/state/possession CHECKs and holder FK; native4 orderedCAS/A1/overflow | Impossible NULL/unknown/negative cases require schema surgery |
| Receipt fingerprint/variant | Exact32byte fingerprint and total variants; differing-FP crosskind control | Exact-FP ChildRoot row must reach decoder and refuse ContradictoryProjection |
| Scoped receipt binding/counters | Exact binding FK, retained counter/result CHECKs; schema-valid counter/acceptedRun/coherent binding controls | None under current schema |
| Receipt time/IDs | Actual10000 refusal,9999/BC replay; nonnullUUIDs, no audit/outbox row FK by design | Infinity shares guarded-null branch; no extra test justified |
| Diagnostics | Closed SQLSTATE-only/fixed storage helpers, safe app Display/Debug | Actual canaries/failures/COMMIT/OTLP remain units15..17 |

One schema-valid ChildRoot INSERT with the exact scoped fingerprint fills the concrete gap;
no trigger disabling or constraint removal is needed. This does not assert cryptographic
collision or native ChildRoot finality. After its actual test/review, local14 can close for
current-schema decoding/qualification/metadata-only evidence. Defensive behavior after
arbitrary owner schema surgery or future schema changes is not a production guarantee.
Canonical runtime tasks, release/C0 and units15..17 remain pending.
