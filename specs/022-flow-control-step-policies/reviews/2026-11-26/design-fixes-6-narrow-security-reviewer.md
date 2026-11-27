# Narrow security re-read of the fifth re-read fixes (2026-11-27, commit `83d30cd75`)

Narrow re-read (OD-2026-11-25-2 step 4) by the security reviewer of the fixes of
[design-fixes-5-rereview-security-reviewer.md](design-fixes-5-rereview-security-reviewer.md), and of the open point on
`ScheduleFiring::pay_obligation`.

## Verdict
All five fixes hold, and the code they rest on matches what the decisions say. No security finding blocks accepting the design's ADR amendments or starting the build. The `pay_obligation` point is bounded and acceptable. It needs one sentence of design text, not a design.

## The five items

| Item | Verdict | Evidence |
|---|---|---|
| Minor 1 (D022-344, T022-270, T022-258) | Fixed | `recheck_authority` checks only `role.grants(Permission::RunStart)` and the database permissions, under `sagat_lock_memberships` (`crates/infra-postgres/src/schedules.rs:181-200`). Its three callers are as cited (`crates/infra-postgres/src/execution.rs:1532`, `:1542`, `:1653`). The authority it checks is the right subject on every path: `record.authorized_by` for a firing (`crates/application/src/schedule_firing.rs:812`, `:1042`, `:1068`), `backfill.requested_by` for a backfill (`:555`, `:589`), and the caller for a trigger-now (`:328`). Each `AUTHORITY_REVOKED` branch is as cited (`:1306`, `:612`, `:342`). The three held-lock cases would fail without the new condition, because `run-recovery` holds `run:start`. A pending reset has the same kind of gap, and D022-288 already covers it (`data-model.md:858`). |
| Minor 2 (D022-345, T022-269) | Fixed | The inline `*_exists_outside` calls and `record_cross_tenant` are as cited (`crates/application/src/api_admission.rs:412-424`, `:441-453`, `:457-479`). `CrossTenantAccessDenied` is `access.denied.cross_tenant` (`crates/domain/src/audit.rs:370`) and is withheld from the tenant (`:262`). For a bound principal, the three outcomes share one response path, and the audit difference stays where the caller cannot see it. |
| Nit 3 (D022-346) | Fixed | `record_denial` counts first, then tries a slot with `try_acquire_owned`, counts `dropped` and warns (`crates/infra-postgres/src/identity.rs:588-598`). `insert_denial_once` deduplicates and takes no slot (`:304-349`), so the task does not take a second one. |
| Nit 4 (D022-347, T022-266) | Fixed | The key `{tenant_id}:{subject}:{namespace_id}` cannot be ambiguous, because both ends are fixed-length identifiers. The trigger is a non-deferred `AFTER DELETE ... FOR EACH ROW` (`research.md:3231`), so it fires at the end of the route's delete statement, before the clear. The route's own delete is therefore skipped once, and a later direct delete in the same transaction is audited. |
| Nit 5 (D022-348) | Fixed | The exemption is now only `crates/domain/src/identity.rs`, with one entry. |

## The open point: `pay_obligation` acting on a stale principal

This is verified in the code, and I rate it low and acceptable.

**The window.** `fire` reads the membership without a lock (`schedule_firing.rs:759-768`) and builds a tenant-wide principal for `system:schedule:<id>` from that read (`:774`). It then pays one obligation and returns (`:778-785`). The cancel goes through `RunService::operate_item`. That function checks `RunOperate` against the stale role and takes no membership lock anywhere (`crates/application/src/runs.rs:2859-2922`). So a demotion to `run-recovery`, or to Viewer, that commits inside the window still lets one owed cancel commit.

**Why it is bounded:**
- **What can be cancelled.** Only a run the schedule itself started, in its own tenant. `pending_obligation` selects by tenant and schedule (`crates/infra-postgres/src/schedules.rs:1770-1772`), and obligations come only from the schedule's own runs (`schedule_firing.rs:1108-1115`, `:1205-1218`).
- **Who decided it.** The cancel was owed by a decision taken under valid authority. Either the firing's filter required `run:operate` for `CANCEL_OTHER` (`:766-767`), or a trigger-now caller passed `authorize_cancelling` (`:283`).
- **How much.** At most one cancel per window. Each firing pays only one obligation. The cancel is idempotent under the obligation key, and the shard fence guards the record that marks it paid (`schedules.rs:1795`). The next firing reads the demotion and pauses the schedule.
- **What the subject gains.** Nothing they can see or use. The actor recorded in the audit is the schedule, and the run is one already ordered cancelled.
- **Not new.** The same window exists today for any demotion, including to Viewer, which holds no `run:operate` at all. The `run-recovery` case is no wider.

Closing it would mean putting a membership re-check inside `RunService::operate_item`, the cancel used by every caller. That change is out of proportion to the effect.

**The one remaining finding (low, design text only).** T022-270's evidence says "its obligation never recorded paid (the run still running)" (`tasks.md:1285`). That is true only when the demotion commits before `fire`'s read. The fix: add a sentence to D022-317 or T022-270 saying the cancel path takes no membership lock, so a demotion inside the window allows at most one owed cancel, and the next firing pauses the schedule. A held-lock test of that bound in T022-270 is optional.

## Files

- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/schedules.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/execution.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/application/src/schedule_firing.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/application/src/runs.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/application/src/api_admission.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/infra-postgres/src/identity.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/crates/domain/src/audit.rs
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-022-policies/specs/022-flow-control-step-policies/tasks.md
