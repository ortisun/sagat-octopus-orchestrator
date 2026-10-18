# Dormant native source certification review

Bounded artifact: native transaction/driver and source proof, separately. No production caller,
released INSERT, producer preservation, restart, uncertain-COMMIT or observations acceptance.
Root is the sole writer and test executor. Fresh agents ran no tests and changed no files.
The user's earlier choice to continue with the current review applies; no external CLI ran.

## Driver cycle 1 — fresh Astra/xhigh

Two valid, actionable findings:

1. Shared chain/scheduler guards called identity::store_error before local sanitization, logging
   raw driver diagnostics. Actual PostgreSQL clock fault with a diagnostic canary reproduced the
   leak (guard-log-red.txt). Shared SQLx-error guard cores now preserve the same SQL and lock order;
   existing wrappers retain their error behavior, while the certifier sanitizes at its boundary.
2. PostgreSQL finite year10000 satisfies the schema but SQLx's OffsetDateTime decoder panics.
   Actual owner canonical fixture reproduced the panic (canonical-time-red.txt). Canonical SQL
   now projects only finite timestamps below year10000; NULL refuses as ContradictorySource.
   The green case includes upper representable, far future and BC dates. Owner fixtures establish
   decoding behavior only, never source success or audit provenance.

diagnostic-green.txt: all3 actual native tests passed after those changes. Runtime mode also
passed3. These observations precede the additional scheduler-expiry finding below.

## Source proof cycle 1 — fresh Astra/xhigh

No concrete issue found in the bounded exact scope/iteration/checkpoint/local-step/journal
projections and saga exclusion. This is static review, not producer preservation acceptance.

## Driver cycle 2 — fresh Astra/xhigh

One valid, actionable finding: the shared scheduler guard still directly decodes expires_at.
The lease renewal port accepts a sufficiently large Duration and can write a finite year beyond
9999, causing the same binary decoder panic before canonical replay. An actual ShardStore::renew
with a large Duration reproduced the panic (scheduler-expiry-red.txt). The first draft assertion
expected a fence refusal, but execution never reached it. The fix preserves valid lease semantics:
lock the qualified scheduler row first, then compare expiry against one materialized fresh clock
in PostgreSQL, returning only that clock if live. No unrepresentable expiry is decoded. The final
test expects SourceNotClosed for the still-open scope, proving a live lease remains live.
Final-native.txt and final-runtime-native.txt each pass4tests after this correction. Other final
gates are reported by commands.json rather than inferred from this review.

## Driver cycle 3 — fresh Astra/xhigh

No concrete issue found in the bounded driver, shared guard changes, native test or exact dormant
privilege exception. Stop condition met after3bounded driver cycles. This does not accept the
pending producer preservation/recovery/observations or release activation.

## Test selection correction

The initial shared-guard regression command used capacity_finish_native::, selected0tests and
provides no regression evidence. Its raw output and command remain retained. The actual module
filter is capacity_finish:: and must be run before this increment closes.
