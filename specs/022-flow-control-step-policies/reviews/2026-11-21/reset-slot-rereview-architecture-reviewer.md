# Architecture re-review of task T022-086 (reset of a rate-limited start), 2026-11-21

Reviewer: `architecture-reviewer` (read-only), given the diff of commits `8dcd7f37` and `cf4f858e` (review files left out),
the specification, its first review and `reset-slot-responses.md`. Saved verbatim.

---

I ran nothing. This review is read-only and had no shell. Every claim below comes from the files at `/Users/marcelo/Documents/Projects/sagat-wt-022` and the diff, not from the authors' answers.

# Re-review: T022-086, reset of a rate-limited start

**Verdict:** both Blockers and both Majors of my first review are resolved, or acceptably redirected, in the tree. T022-086 may be ticked once minors B, C and G below land with it. They are small: two contract sentences, one gap row and the evidence.

## Part 1: the first review's Blockers and Majors

### Blocker 1 (018's normative registry contradicts the code): resolved
- `specs/018-schedules-bulk-operations/contracts/journal-and-events.md:139-140` now holds the two rows the code installs (`crates/domain/src/execution/reset.rs:130-137`):
  - `queued`, `granted`, `returned` are `REMATERIALIZE`, with the queued-to-granted point refused;
  - `wait_timed_out` and `dequeued` are `REFUSE` until the end.
- `:15` adds the fresh `rate_limit.granted`, and `:61-65` adds the "022 run slots" bullet, including the `CANCEL_SOURCE` consequence.
- What remains is that no test reads this registry. That is minor A.

### Blocker 2 (Studio client behind the OpenAPI): resolved in the tree
- The new descriptions in `frontend/apps/studio/src/api/schema.d.ts` (hunks at about `:3474` and `:3762`) match `specs/001-platform-foundation/contracts/openapi.yaml:5034-5037` and `:5331` word for word.
- The third changed line (`estimatedGrantAt`, about `:5751`) catches up with text already in `openapi.yaml:7318`, so it belongs in the regeneration.
- I could not run `check:api`. The authors' run (49 Vitest passes) is recorded only in `reset-slot-responses.md:12`, not in the task's evidence. That is minor G.

### Major 1 (`CANCEL_SOURCE` on a full key cancels and never replaces): acceptable as a follow-up; it need not be built before the tick
Reasons:
- **The behaviour is the approved contract.** ADR-0088 decision 8, ADR-0056 decision 3 and the "fresh slot" rule of `contracts/journal-events.md:68` all say this.
- **The outcome is recorded, not silent:**
  - the entry ends `FAILED` with a closed reason;
  - a `system:engine` `run.reset` audit record is written;
  - `workflow_reset_completions_total{outcome="failed",reason="RATE_LIMIT_SLOT_UNAVAILABLE"}` is counted.
- **"A cancelled run and no replacement" is a class 018 already accepted.** See `docs/runbooks/run-reset.md:265`, ADR-0081, and the same trap for permits at `:258`.
- **Exposure is limited.** `CANCEL_SOURCE` is off by default.
- **The alternative is no guarantee.** The advisory refusal T022-114 is racy (D022-069 item 3 says so), so building it first would not turn this into a promise.
- **The alternatives are recorded.** D022-069 item 3 (`research.md:~1229-1239`) records both, and the owner question on "take over the source's slot" is named, not proposed.

Two conditions are still open (minors B and C). Today the warning reaches only operators, and the follow-up is bound to nothing.

### Major 2 (only the reset part built): the move to T022-113 is acceptable
- The split is a recorded delegated decision (D022-069 item 7, `research.md:~1257-1262`).
- The task text is amended (`tasks.md:480`), and T022-113 (`tasks.md:569`) names its creators and evidence.
- The structural guarantee exists, but the decision cites the wrong check (minor D, which I introduced in my first review). The real enforcers are:
  - `run_start_errors` (`scripts/check-architecture.py:383`): one start point, `Engine::start`;
  - `run_insert_errors` (`:334`): one admission point, guarded by `declares_gates()`.
- Commit `cf4f858e` widened `run_start_errors` to accept `start_gated` (`:380`). That is safe today: `start_gated` is private (`crates/application/src/execution.rs:2137`) and its only caller is `start` (`:2133`). Nothing keeps it that way (nit 3).

## Part 2: the current diff, reviewed afresh

**What holds:**
- **Layering.** The domain gained only registry rows. `SlotUnavailable` is in the application layer, and `json!` only in `api-http`. No empty mapper.
- **Contracts kept in step:**
  - 001 OpenAPI `Error.details` and `Reset.failureReason`;
  - Studio types and the Studio map;
  - 018 `errors.md` row, pinned by `tests/contract/schedules-bulk-operations/errors.rs:58`;
  - 018 data model and REST fragment;
  - 022 `errors.md`, `journal-events.md`, spec and plan.
- **Migration.** The up is additive (`NOT VALID`, then validated). The down refuses while an entry carries the reason, and has a test.
- **Telemetry.** Labels stay closed: `refused_reset` is pinned, and `FAILURE_REASONS` has 8 entries, matching the 8 rows at `run-reset.md:249-258`.
- **No duplicate work.** `sagat-wt-022-sdk` has no migration after `20270207000100` and no T022-113, T022-114 or D022-069.

### Findings

**minor A: no check ties 018's registry to `RESET_RULES`.**
- Where: `journal-and-events.md:131-140` against `reset.rs:120-137`. `run_creators::the_reset_rules_of_the_rate_limit_events_are_the_contracts` hard-codes the expectations rather than reading either document.
- Scenario: the next change of a rule drifts exactly as Blocker 1 did.
- Fix: one contract test that parses the `| 022 |` rows (event names to `REMATERIALIZE`, `REFUSE` or `EXCLUDE`) and compares them with `rule_by_name`.
- Confidence: high.

**minor B: the `CANCEL_SOURCE` trap is not in the contract callers read.**
- Where: `specs/001-platform-foundation/contracts/openapi.yaml:1082` (`resetRun`) names neither the slot refusal nor the permit refusal. `specs/018-schedules-bulk-operations/contracts/rest.openapi.yaml:445-450` names the 409, but not that a cancel frees no slot.
- Scenario: an API or SDK user who never reads the runbook asks for `CANCEL_SOURCE` on a full key and loses the run.
- Fix: one sentence in both descriptions. It is a condition of accepting Major 1.
- Confidence: high.

**minor C: T022-114 is bound to nothing.**
- Where: `tasks.md:570`. There is no gap row in `research.md`, and it is not in the gate list at `docs/runbooks/flow-control.md:195` and is not declared outside it.
- Fix: register a G022 gap, and state in section 6 that it is deliberately not a production gate (off by default; the outcome is recorded), or add it to the list.
- Confidence: high.

**minor D: D022-069 item 7 cites the wrong enforcer.**
- Where: `research.md:~1258` cites `run_creator_errors` (`check-architecture.py:724`), which is design 043's checked-input rule.
- Fix: cite `run_start_errors` and `run_insert_errors`.
- Confidence: high.

**minor E: ADR-0088's "Enforced by" is stale.**
- Where: `docs/adr/ADR-0088-...md:8-12` says "None exists yet" and names "the run-creator test (T022-086)". T022-079 and T022-081 are ticked, the reset tests exist, and part of the creator check moved to T022-113.
- Fix: name the existing tests and scripts, and list T022-080, T022-113 and T022-076 as pending.
- Confidence: high.

**minor F: rolling back the binary after a rate-limited reset is not covered.**
- Where: D022-069 item 6 and `flow-control.md:198` cover the forward rollout only.
- Scenario 1: an engine is rolled back to a build from before T022-086 after such a reset. It cannot decode the `restored` `rate_limit.*` rows, so the replay of those runs fails.
- Scenario 2: an earlier 022 slice (for example #237) is released without this commit. The claim that no released build advertises the capability without reading the marker then stops holding.
- Fix: one runbook sentence ("do not roll an engine back past T022-086 once a rate-limited run was reset"), and one line in item 6 that this commit must ship in the same release as `FlowControlRateV1`.
- Confidence: medium.

**minor G: the task's evidence is incomplete for a tick.**
- Where: the built note at `tasks.md:~483-499`.
- Missing:
  - `npm run check:api` and Vitest;
  - `python3 scripts/check-architecture.py`, which was red before `cf4f858e`;
  - results of `integration_flow_control_step_policies` (`run_creators`, `migrations`);
  - `SAGAT_TEST_AS_RUNTIME_ROLE=1` and `runtime_privileges`;
  - the fifth test, `a_reset_after_a_rate_limited_step_restores_its_slot_as_history`.
- Fix: add the commands and their output before ticking.
- Confidence: high.

**minor H: what `refused_reset` counts is not written down.**
- Where: `readiness_gate.rs:1659-1680` counts the refusal in each family the start declares. A reset refused by the rate gate with a free permit also increments `concurrency_acquisitions_total{outcome="refused_reset"}`.
- Scenario: a dashboard or alert reads that count as "concurrency refused".
- Fix: one sentence in `plan.md:230` and `:454` (or in the runbook's metrics) saying the count is per declared family, not per refusing gate.
- Confidence: high.

**minor I: open minors from the first review, carried over unchanged.**
- The command line has no sentence for the new reason (`apps/cli/src/reset.rs:271`, `apps/cli/tests/reset_and_with_start.rs:995`).
- Nothing checks that the database CHECK equals `FAILURE_REASONS` plus the two engine-only reasons.
- `BEFORE_RESET_SLOT` is hard-coded (`tests/integration/flow-control-step-policies/migrations.rs:1443`).
- 018's `spec.md` and `plan.md` rows 17 and 18 still mention only the permit.
- No end-to-end case with both gates refusing (the unit test covers it).

**nits:**
1. **Dates.** The nit is marked answered, but two places still say 2026-11-20: `contracts/journal-events.md:47` and `run-reset.md:376`. The task was built on 2026-11-21.
2. **Stale runbook sections.** `run-reset.md:373-378` lists built behaviour under "What is not built yet", and the section 5 table (`:326-344`) has no 409 rows for the slot or the permit.
3. **`start_gated` stays private by convention only.** Nothing checks that it is private with one caller. Assert it in `run_start_errors`.
4. **Wording and alignment.** "grows only" still reads as scoped to the new reason (018 `rest.openapi.yaml:916`). The permit row of 022 `errors.md:31` still says "per 018" while the slot row at `:28` says 409 `CONFLICT`. "then" still reads as both reasons given (`execution.rs:2236`, `runs.rs:1994`, `readiness_gate.rs:1198`).
5. **Rate first lives in the adapter** (`readiness_gate.rs:1673-1679`) rather than on `DeclaredGates`.
6. **The codec accepts the marker on `REFUSE` gate types** (`event_codec.rs:1085-1089`).
7. **Out-of-scope changes in this series.** `cf4f858e` also carries the `docs/runbooks/README.md` row, and commit `8dcd7f37` carries the rustfmt-only change to `future_sizes.rs`. `cf4f858e`'s checker fix is D022-070 work. Harmless; reference D022-070 in its commit footer.

## What enforces the rules this change relies on

| Rule | Enforced by |
|---|---|
| A reset never waits | domain guard; `start_may_wait`; `a_reset_without_a_free_slot_is_refused_rate_first_and_never_waits` (no waiter row) |
| Rate first | `readiness_gate::tests::a_refused_reset_is_named_by_its_first_refusing_gate_and_counted_by_each_declared_family`; integration covers each gate alone, not both at once |
| One start point, one admission point | `check-architecture.py` `run_start_errors`, `run_insert_errors` |
| Restored grants not counted | `verify_run` (`execution.rs:~893`) through `assert_consistent` in the reset tests |
| Error row, remediation sentence and OpenAPI agree | `tests/contract/schedules-bulk-operations/errors.rs` |
| Down migration keeps evidence | `migrations::the_reset_slot_reason_rolls_back_only_once_no_entry_carries_it` |
| 018 registry equals the code | **nothing** (minor A) |
| Database CHECK, CLI and Studio lists equal `FAILURE_REASONS` | **nothing** (minor I) |

Blockers: 0, Majors: 0
