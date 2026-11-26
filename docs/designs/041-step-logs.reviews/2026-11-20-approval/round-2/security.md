# Security re-read of design 041's fixes (approval run, second round)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 4d6fe4cc80aff7d25bd706e47d519e05bcb269a7 (against ff8033cd)
Reviewer: security

Four of the five first-round findings are resolved. S-1 is only partly resolved: it keeps its Major because the bound holds per tenant, but the text treats it as protection for other tenants. The fixes add no new Blocker or Major. They add four Nits.

## First-round findings

**S-1 (Major): partly resolved; Major remains.**

What is fixed:
- The wait slots implement the remedy I named: a per-tenant slot that is tried and never waited for (`docs/designs/041-step-logs.md:1346-1354`, `specs/041-step-logs/data-model.md:253-267`, ADR-0097 decision 4 at lines 146-149).
- The held-lock test with tenant B as the victim is in T041-36 (`docs/designs/041-step-logs.md:2847-2851`).
- The two-tenant regime is in T041-37, with its criterion fixed before it runs (`docs/designs/041-step-logs.md:2943-2948`) and its consequence stated (line 2964).

What is left: the bound is per tenant, but the text presents it as a guarantee to other tenants. The design says "another tenant's reports and heartbeats keep the rest" (`docs/designs/041-step-logs.md:1349-1350`). The runbook says "one tenant's logging cannot take the connections other tenants' reports need" (`docs/runbooks/step-logs.md:59-61`). Neither holds, for three reasons I checked in the text and the code:
- **No cap across tenants.** The Lease class has no per-tenant bound and no bound tied to the pool (`crates/api-grpc/src/limits.rs:548-556`). Three tenants, each in the slot-wait regime on one replica, can hold 21 waiting connections. The default pool of `apps/execution-engine` is 20. A tenant gets into that regime on its own:
  - It keeps itself near its limit, in section 2b's band.
  - There nearly every carry refills, and a successful refill holds the usage row until its report commits (up to 726.7 ms measured).
  - T041-37 models exactly this: the row held 150 ms in every 300 ms.
- **Slots and Logs tickets add up instead of overlapping.** A report's wait sits outside the Logs class. One tenant can therefore hold:
  - 7 tickets, with ingests waiting up to 200 ms on its own stream locks; a tenant controls its workers and can send batches for one attempt at the same time;
  - plus 7 slot waits;
  - plus the row holder.

  That is 15 of 20 connections, against the "10 − 7 = 3 kept" reasoning of `specs/041-step-logs/data-model.md:452-454`.
- **P is fixed while the pool and tickets are settings.** P is a SQL constant (`specs/041-step-logs/data-model.md:251`), but the pool size and the ticket count can be changed. At `SAGAT_DB_MAX_CONNECTIONS = 10`, the Logs cap is 5 and the tickets are 5. One tenant then holds 5 + 7 = 12 connections, more than the pool: that is S-1's single-tenant scenario again. Following the runbook's advice to raise the tickets (`docs/runbooks/step-logs.md:90`) widens the gap.

Smallest fix:
- Add a replica-wide set of Q wait slots, keyed by `application_name` alone and tried after the tenant's slot. Choose Q so that the Logs cap plus Q leaves a reserve of the pool for leases and heartbeats.
- Either derive P and Q from the pool at startup, through the same per-connection channel the key already trusts (see N-1), clamped in SQL; or have the configuration refuse to start when the Logs cap plus P plus a reserve exceeds the pool.
- Add a regime with three or more tenants in fallback to T041-37, and a held-lock case to T041-36 in which B's heartbeat still gets a connection.
- Correct the three claims cited above.

Disposition: fix.

**S-2 (Nit): resolved.** The ensure is gone. The top-up runs only after a report has committed, so after authorization and C0, and a source-text check guards the call site (`docs/designs/041-step-logs.md:1411-1413`; ADR-0097 lines 141-142; `specs/041-step-logs/data-model.md:295`). The negative and cross-tenant tests are at `docs/designs/041-step-logs.md:2844-2846`.

**S-3 (Nit): resolved.** The top-up takes no amount and grants only its share, at most 1 MiB (`specs/041-step-logs/data-model.md:297-303`; ADR-0097 lines 129-132). `sagat_quota_step_log_reserve` takes a size but writes nothing (data-model:316-329). The refill's size is still measured by the trigger.

**S-4 (Nit): resolved.** The keys are built inside the function by the helpers the ingest also calls, and `p_mark_reason` is checked at the top with a fixed text (`specs/041-step-logs/data-model.md:590-592, 614-618`). See N-3 for one leftover mismatch.

**S-5 (Nit): resolved.** The runbook now asks for both parameter-length settings at 0 and for terse errors, set for `sagat_app` (`docs/runbooks/step-logs.md:101-106`). The design says the same (`docs/designs/041-step-logs.md:1973-1974`).

## New findings from the fixes

No new Blocker or Major.

I checked the following and found no issue:
- **Usage withheld.** It fails closed across every namespace and is tested with two namespaces (`docs/designs/041-step-logs.md:2391-2396`, `specs/041-step-logs/contracts.md:285-289`).
- **Top-up tenant.** It always uses the principal's tenant.
- **`notStoredReason`.** It sits behind the streams' PII gate (`specs/041-step-logs/contracts.md:296`).
- **Lock spaces.** The slot locks use the two-`int4` advisory form. Every other advisory key in `crates/infra-postgres` uses `hashtextextended` (bigint), so no other subsystem collides with them.

Nits:
- **N-1:** The reasoning that the functions never read "a setting the caller could change" (`specs/041-step-logs/data-model.md:261-262`, D041-125, `docs/designs/041-step-logs.md:2434`) is contradicted by the slot key. The key reads `current_setting('application_name')`, which any session can `SET`. The effect is limited to slot sharing among `sagat_app` callers. Correct the sentence, because the S-1 fix can rely on the same channel.
- **N-2:** `hashtext` is 32 bits, so tenant keys can collide (by chance, not by choice). Colliding tenants share their slots and see more busy answers. Use `hashtextextended(key || ':' || slot, 0)` with the single-bigint form, as the rest of the codebase does.
- **N-3:** `specs/041-step-logs/data-model.md:19` lists `p_mark_reason`'s values without `failed`. Lines 590 and 614 and D041-128 include it. An implementer who follows line 19 would make the guard's re-run (reason `failed`) raise at step 0.
- **N-4:** G041-018 is called "Closed" (`docs/designs/041-step-logs.md:2022`), but one bit remains. A reader with the PII grant in namespace X still sees `ceiling = tenant_quota` on X's streams when namespace Y's logging fills the tenant's quota. State this as an accepted residual rather than as closed.

Counts: Blocker 0, Major 1, Nit 4