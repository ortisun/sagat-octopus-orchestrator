Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: e1d438b0ff4e17994feb29517a4d0cd2582b1349
Reviewer: egm-zoom-out (egm-review stage 8, intermediate zoom-out, round 2)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 1, Nit 5

Round 1: answered 1 to 8 and 10 (1 and 4 by accepted contest); 9 not answered (finding 2).

1. **Major.** A revocation feed of its own (a new fetcher, schedule, setting and trust path) rebuilds what feature 028's operator indexes already give. Fix: publish revocations as an index that feature 028's refresh fetches; keep the operator block.
2. **Nit.** The callback SQL and the timing constants are still in the architecture section.
3. **Nit.** The `oauth_passes` lease row is a new mechanism; acquire is already single-flight; use `FOR UPDATE SKIP LOCKED` batches.
4. **Nit.** The builder table schedules hub and declaration tasks in Phase A while Technical Plan 14 puts them in Phases C and B.
5. **Nit.** Whether the rate limits are per control plane or deployment-wide; defer a cross-plane limiter until offers ship.
6. **Nit.** The re-ask table guards against an unobserved load; count in the gRPC limit class or from the audit rows.
