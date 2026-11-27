# Approval read of design 042: egm-review (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 4b0248c7b2019be951f38c1e6ec12dd4e662569b
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the reviewer's output as returned, with this header added and machine paths made relative. The EGM review of design
042 ran its lens agents with the accepted EGM plugin (commit f0ba151b): stages 5 to 8 folded into one critique with the
zoom-out and comprehension lenses (`../round-1/egm-design.md`) and stage 9 with the codebase-details lens
(`../round-1/egm-implementation-readiness.md`), each run once as an agent with no conversation history and spawned one at
a time; then this confirmation read of the fixes, run as the `egm-implementation-readiness` agent with all three lenses.

Counts: Blocker 0, Major 4, Nit 6

## Previous reports

**`egm-design.md` (round 1)**

- **Fixed, 22 of 23:**
  - Finding 1: counters off the chain row; `operator_chain_usage`; marks per key; lock modes stated. Lines 361-367, 941, 1042-1050, 1099, 1130.
  - Finding 2: the engine switch and its behaviour. Lines 494-502 and 777-788.
  - Finding 3: design 053, form (a), nullable columns, expand-only migration. Lines 446-447, 503-506, 933 and 1287-1321.
  - Finding 4: the FR-042-031 outcome table. Lines 427-453.
  - Finding 5: `maxHoldMs` at least 90 seconds and 180 by default; CUJ 3 rewritten. Lines 254-256, 718-724 and 817.
  - Finding 6: horizon age and the alerts; risk table corrected. Lines 1149-1153, 1260-1266 and 1362.
  - Finding 7: FR-042-032 shapes. Lines 454-482.
  - Finding 8: per-index fan-out; "all-or-nothing" removed. Lines 305-355 and 1636-1653.
  - Finding 9: targeted delivery retry. Lines 483-493.
  - Finding 10: `duplicate_member`. Lines 440 and 1062.
  - Finding 11: `keyIdleMs` for every keyed chain. Lines 221-222 and 816.
  - Finding 12: ADR-0050 deviation, zero-row guard. Lines 274-277; ADR-0050 line 103.
  - Finding 13: no HMAC. Lines 957-971.
  - Finding 14: random `window_id`. Lines 266-268 and 944.
  - Finding 15: inline inputs only. Line 359; Gap 042-015.
  - Finding 16: the switch refused until Task 042-22; cut order. Lines 506-507 and 1323-1326.
  - Finding 17: the bound is stated. Lines 551-555.
  - Finding 18: outcome vocabulary. Lines 438, 444 and 446-447.
  - Finding 19: stale statements. Lines 128, 909-910, 1460 and 1844-1848.
  - Finding 21: order of the keyed stages. Line 197.
  - Finding 22: unfenced dispatcher start. Lines 270-271 and 1112-1114.
  - Finding 23: chain identity. Lines 384-388.
- **Partly fixed:** finding 20. There is a Terms section (lines 51-71), a Revisions section at the end, and the owner decisions list now includes the 2026-11-27 rows (lines 663-684). The Terms section points to a section that does not exist, and several terms used everywhere are still undefined. Finding 10 below.

**`egm-implementation-readiness.md` (round 1)**

- **Fixed, 19 of 23:**
  - Finding 2: element at stage 0; the `trigger_chain` answer; the input shapes; the check in `DefinitionService`. Lines 200-204, 454-482 and 759-763. The sample it uses brings a new defect: finding 1 below.
  - Finding 3: shared files and order points between the lanes. Lines 1465-1492.
  - Finding 4: the manual acts. Task 042-57; lines 1136-1147 and 1227-1236.
  - Finding 5: lock modes; the `KEY_LIMIT` count row. Lines 947 and 1128-1129.
  - Finding 7: parameter table. Lines 809-827.
  - Finding 8: outcome table and `RunTx::record_window_start`. Lines 279-291.
  - Finding 10: no HMAC key any more.
  - Finding 11: form (a) with `FOR KEY SHARE`, plus a held-lock test. Lines 503-506 and 1774-1775.
  - Finding 12: `SAGAT_TRIGGER_OPERATORS_ENABLED`. Lines 495-502.
  - Finding 13: ADR-0155 deviation; slot test split; harness named. Lines 1561-1562 and 1630-1631.
  - Finding 14: Task 042-1 checks Status. The gates now cite Owner decisions 2026-11-27-420 to 2026-11-27-435, and I checked each `Covers` cell in `docs/owner-decisions.md`.
  - Finding 16: metric table. Lines 1246-1258.
  - Finding 17: no journal addition. Line 1269.
  - Finding 18: late elements, tumbling `opened_at`, `overflow`, shard. Lines 227-245, 256-259 and 934-935.
  - Finding 19: chain lifecycle. Lines 769-775 and 384-388.
  - Findings 20 to 23: references by function, 59 tasks, stale passages, index lifetime. Lines 115-117, 1844-1848, 128 and 952-955.
- **Partly fixed, 4:**
  - **Finding 1.** Identity and key form are fixed (lines 266-269 and 957-960). The index and fingerprint encodings are still open: finding 7.
  - **Finding 6.** Columns, `CHECK`s and NULL for old rows are fixed (lines 446-453). Nothing names how the stateless path writes its outcomes: finding 6.
  - **Finding 9.** The resources are named and `QUOTA_EXCEEDED` is final (lines 368-379). Their plumbing in the code is wrong or missing: finding 3.
  - **Finding 15.** The bound, the failpoints, the target and the thresholds are fixed. The statement budget is still miscounted; that is correctness approval finding 7, not repeated here.

I confirm security approval finding 3 and architecture approval finding 10: a `count` or `onFull` seal of a `flatMap` window evaluates under the apply's locks (lines 1063-1064). I also confirm architecture approval finding 12 (stale "three alerts" and "seven tables").

## Findings

1. Major — The publication check uses a sample whose `value` is `null`, so it refuses every version whose input schema types the element
   Where: `docs/designs/042-stream-operators.md` lines 138-141, 471-474, 1721-1724 (Task 042-43).
   What:
   - FR-042-032 refuses publication unless the version's `inputSchema` accepts "a one-member sample of every shape … with `value` null".
   - A schema that types the element fails on that null. For example, CUJ 3's `event.value.data.orderId` declared as `required`, or `value` given `type: object`.
   - So an author can attach a typed input schema to a debounce or window chain only by making `value` nullable everywhere. That defeats design 043's check of the real input at the start.
   - The two passages also disagree:
     - FR-042-003 (line 139) applies the check only to window emissions (`window`, `debounce`, a keyed stage or `flatMap`).
     - FR-042-032 applies it to "every shape the chain can emit", which includes `{"value": …}` of a `filter`/`map` chain.
   Why: The check meant to catch a wrong envelope instead refuses correct definitions in the common case. Lane A would have to pick a sample by itself to make CUJ 3 publishable.
   Correction:
   - Check only the envelope: validate the sample with every subschema at the `value` positions replaced by `true`, or check only the envelope's top-level keys and types.
   - Alternatively, state as a decision that a schema must accept `null` there, and say why.
   - Make FR-042-003 and FR-042-032 name the same set of shapes. Add to Task 042-43 a test that CUJ 3 with a typed `value` schema publishes.

2. Major — The trigger half of the six SDKs has no designed surface, and adding fields to Rust's public `Trigger` struct breaks callers
   Where: lines 522-524 (FR-042-028), 917-929 (the per-SDK table, collections only), 1273-1276, 1691 (Task 042-36).
   Code:
   - `sdks/rust/src/workflow/primitives.rs:113-125` (`pub struct Trigger` with public fields and no `#[non_exhaustive]`)
   - `sdks/typescript/src/workflow.ts:150`
   - `sdks/go/workflow/options.go:318`
   - `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workflows/WorkflowOptions.cs:347`
   - `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/workflow/Trigger.java:10`
   What:
   - Collections get a table saying where the chain goes in each SDK, and Rust gets new methods so that `CollectionOptions` stays unchanged.
   - Triggers get one line: "the trigger builders reuse the operator factory".
   - Nothing says, per language, where `key`, `operators` and `chainLimits` attach to each SDK's `Trigger`.
   - In Rust, adding a public field breaks every `Trigger { … }` literal, which is the same hazard the collection side avoided.
   - One factory name `window` must build both `{count}` (collection) and `{kind: tumbling|session|count, …}` (trigger). Its signature for each shape is not given.
   - The factory entries for `debounce` and `ignoreElements` are not given.
   Why: Lane A must design six public APIs on its own, and one obvious choice breaks a present caller, against FR-042-028 and NFR-042-004.
   Correction: Add a trigger table beside the collection table, covering each SDK:
   - where `key`, `operators` and `chainLimits` go;
   - for Rust, a non-breaking form, such as a `with_chain` builder method or a new constructor;
   - the factory signatures of `window` per surface, `debounce` and `ignoreElements`.
   Give Task 042-36 a compile test per SDK that every present trigger call still builds.

3. Major — The four new quota resources are pointed at the wrong place in the code, and the held-bytes default is above the value range the CHECK allows
   Where: lines 367-379, 671, 1017-1025, 1065-1069, 1554-1555 (Task 042-13: "with their defaults in `quota_limits` and `quota_ceilings`"); line 418 (FR-042-024 relies on held bytes being visible under `quota:read`).
   Code:
   - `crates/domain/src/quota.rs:400-417`: defaults live in `QuotaResource::default_limit`, not in a table. `quota_limits` rows need a `tenant_id`.
   - `crates/infra-postgres/migrations/20270201000000_step_log_quotas.up.sql:13-96`: every new resource restates `quota_*_resource_valid`, `quota_limits_tenant_scoped` and `quota_*_value_range`, whose `ELSE` allows at most 1,000,000,000.
   What:
   - `operator_held_bytes` defaults to 1,073,741,824. That is above the `ELSE` bound, so an operator cannot set the default value or more without a `WHEN` branch. The design says an operator changes this quota "like any quota".
   - No task adds the four resources to `QuotaResource` (`as_str`, `default_limit`, `limit_name`, ceiling, kind and unit).
   - No task says they are tenant-scoped, which keeps them out of the namespace list.
   - No task exposes their usage in the quota view that FR-042-024 assumes.
   - The design does not say how the apply learns the resolved held-bytes and content limits. If `operator_held_room` returns usage only, reading the limits is another statement, and the budget at lines 1078-1080 does not count it.
   - The resource names `operator_chains`, `operator_keys` and `operator_marks` are also table names, which invites confusion.
   Why: Task 042-13 cannot be built as written. A builder would either put defaults where none can live or miss the domain enum and the restatement, and `the_last_restatement_of_each_quota_resource_list_names_every_resource` would catch that only at the end.
   Correction: Rewrite the quota part of Task 042-13 (and give lane A its domain share):
   - add the four resources to `QuotaResource` with their defaults and ceilings;
   - restate the three CHECK lists, with a `WHEN 'operator_held_bytes'` byte range;
   - mark the four as tenant-scoped;
   - say what the quota usage view shows for each;
   - say whether `operator_held_room` also returns the resolved limits (one statement) or the limits are read separately (and counted).
   Consider resource names distinct from the table names, for example `operator_chain_count`.

4. Major — The REST and publication error contract of the trigger surface is not one closed list
   Where: lines 485-486 (FR-042-033: "the switch's typed error (`OPERATOR_UNSUPPORTED`)"), 779 ("its trigger twin"), 1139-1140 (flush: `409 QUOTA_EXCEEDED`), 1230-1235 (REST table), 1326 and 1723 (`OPERATOR_UNSUPPORTED` for an unshipped operator), 1477-1478 ("new `IrError` variants"), 944 and 1230 (`lastErrorCode`).
   Code: `crates/api-http/src/error.rs:1241-1256` (`CollectionOperatorsDisabled` → `COLLECTION_OPERATORS_DISABLED`; operator refusals go through `IrError::Operator { refusal }` → `INVALID_OPERATOR`).
   What:
   - `OPERATOR_UNSUPPORTED` means two things: a cut-order operator not shipped yet, and the trigger switch being off. Meanwhile section 1 names the switch error as the unnamed "trigger twin" of `CollectionOperatorsDisabled`.
   - The REST table omits two answers the text gives:
     - the HTTP status of `OPERATOR_UNSUPPORTED` on the delivery retry;
     - the flush's `409 QUOTA_EXCEEDED`.
   - The table says nothing about a flush whose `flatMap` evaluation refuses the window or overruns its budget.
   - The windows' `lastErrorCode` has no closed value set or `CHECK`, unlike the closed sets of `event_deliveries`.
   - Task 042-43 calls `OPERATOR_ORDER`, `OPERATOR_NEVER_EMITS` and `OPERATOR_INPUT_SCHEMA` "new `IrError` variants". The code's pattern for operator refusals is a refusal value of `IrError::Operator`. The design does not say which.
   - Paging of the window list ("paged") names no convention.
   Why: Lane A writes the OpenAPI baseline and the error mapping. Every gap above is a choice of code or status that the contract baseline then freezes.
   Correction: Add one error table for the trigger surface: code, HTTP status, retryable, the endpoint or path that raises it, and the `IrError` shape. Use distinct codes for "switch off" and "not shipped". Close `last_error_code` with a `CHECK`. Name the paging convention.

5. Nit — `key` or `chainLimits` on a chain with no keyed stage or window is undefined
   Where: lines 193-199, 206-216, 404-415, 434.
   What:
   - A chain of only `filter` and `map`, or `filter`, `map` and `flatMap`, takes no key row, so a declared `key` does nothing.
   - FR-042-024 still makes any chain with a `key` value-dependent, which hides its outcomes behind `WITHHELD`.
   - FR-042-031 gives `OPERATOR_KEY_INVALID` only on the locked path, so a failing key on the stateless path has no outcome.
   Correction: Refuse at publication a `key` (and `chainLimits`) on a chain without `distinct`, `skip`, `take`, `window` or `debounce`, with a typed code. Or state that they are ignored and that value-dependence follows the stages, not `key`.

6. Nit — The stateless path's delivery outcomes have no named writer
   Where: lines 206-208, 431-433, 444.
   Code: `crates/application/src/execution.rs:1683-1686` (`RunTx::record_delivery(&DeliveryRef)` carries no outcome); `crates/application/src/dispatch.rs:307-312` (`record_refused_delivery`).
   What:
   - A `filter`/`map` chain must now write `SUPPRESSED`, `FAILED` and `STARTED` rows.
   - `STARTED` needs an outcome on `DeliveryRef` or on `record_delivery`.
   - `SUPPRESSED` and `FAILED` happen with no run, so they need a run-less insert. No store method or transaction is named for it.
   Correction: Name the carrier, for example an `outcome` field on `DeliveryRef`, and the run-less method (its idempotency and whether it audits) in Task 042-15.

7. Nit — The window start key's index encoding and the fingerprint encoding are not fixed
   Where: lines 266-269, 349-352; `crates/application/src/dispatch.rs:181-199`.
   What: "The SHA-256 of the window's `window_id` (and, after `flatMap`, of its index)" does not give:
   - the bytes of the index (width, endianness, length prefix);
   - whether `window_id` is hashed as 16 bytes or as text;
   - the encoding of the fingerprint over `(source, event_id)`;
   - whether every index of a fan-out shares one fingerprint.
   Correction: State them in `start_key`'s length-prefixed form.

8. Nit — The default per-key mark ceiling is 10, and exceeding it fails as `KEY_LIMIT`
   Where: lines 365-367, 815, 435.
   What:
   - With the defaults, `floor(100,000 / 10,000)` gives 10 distinct values per key within the horizon. The eleventh fails `FAILED`/`KEY_LIMIT`, even when the chain has three live keys.
   - The code's name also misdirects an operator, since no key limit was hit.
   Correction: State the effective default and its tuning (a lower `maxKeys` raises it) in FR-042-021 and the runbook task. Consider a separate code such as `MARK_LIMIT`.

9. Nit — The cut order's later steps have no capability of their own
   Where: lines 1323-1326, 1723; `docs/designs/053-rolling-upgrades.md` lines 268-279 (Requirement 053-006).
   What:
   - If step 2 or step 3 ships after 0.3.0, a rollback to 0.3.0 meets chains that 0.3.0 cannot process.
   - The design does not say where the `OPERATOR_UNSUPPORTED` refusal lives:
     - In the codec, `parse_stored` refuses stored documents, which goes against FR-042-003.
     - At publication only, an older dispatcher meets a stage it does not implement.
   Correction: State that each step shipped after 0.3.0 comes behind a capability of its own under Requirement 053-006, and that the refusal sits in `DefinitionService`, not in `convert::*`.

10. Nit — Comprehension: a dangling reference, an over-long requirement and undefined terms
    Where: line 62 ("section 5a", which does not exist), lines 305-355 (FR-042-020, one bullet of about 50 lines), lines 51-71, lines 279-291 against 984-985.
    What:
    - The Terms section does not define "value-dependent", "envelope", "pre-charge" or "retry grace", which recur throughout.
    - FR-042-031 to FR-042-034 sit between FR-042-024 and FR-042-025.
    - FR-042-019's outcome table, which claims one outcome per case, has no rows for the seal's own refusals `EXPRESSION_ERROR` and `OPERATOR_FAN_OUT`. Section 4's state list (lines 984-985) names them.
    Correction:
    - Point "Summary" at FR-042-032.
    - Split FR-042-020 into evaluation, held bytes, waiting and start.
    - Add the four terms to the Terms section.
    - Add the two seal-refusal rows to the outcome table.
