# Architecture review: design 041 revision of 2026-11-20 (hot row), ADR-0097 and D041-111

Reviewer: `architecture-reviewer` subagent, read-only, on `4d74b27a`. Saved verbatim. Paths are relative to the worktree
root `/Users/marcelo/Documents/Projects/sagat-wt-041`.

## Scope

I read the document and prototype diffs, ADR-0097, ADR-0095, ADR-0084, ADR-0027 and the ADR README and template. I also read design 041 (§2, §2a, §2b, §3, §4, the Constitution Check, Complexity Tracking, T041-5 and T041-35), data-model §3, §3b, §4 and §8a, the research entries D041-105 to D041-111, G041-023/024 and O041-026/040, the 2026-11-20 sections of the validation report, `docs/engineering/architecture-principles.md`, the constitution and `docs/owner-decisions.md` (OD-2026-11-20-6). For the code I read `tests/support/lock_order.rs`, `crates/infra-postgres/src/{quotas.rs,content_usage.rs,lib.rs}` and the git refs of the shared repository. Nothing was built or run.

---

## Blocker

### B1. ADR-0097 departs from the accepted ADR-0084 without saying so

**Evidence**
- `docs/adr/ADR-0084-…md:151-153` (Revision, accepted): a batch "MUST NOT take the tenant's `tenant_retained_usage` row before its commit. Over the tenant's allowance, by a read without a lock or by the commit-time finalizer's raise, the batch MUST store no line".
- ADR-0084 decision 11 (`:185-187`), with the ingest reading of its Notes (`:216`, `:235-237`): "then at commit the fences of the charged classes and the tenant usage row".
- `docs/adr/ADR-0097-…md:65`: "A refill … locks the usage row". Lines 60-64: the fences and the charge happen at the insert, not at commit.
- `docs/designs/041-step-logs.md:188-189` (FR-041-008 rewritten): a batch "takes the tenant's `tenant_retained_usage` row only to refill its leases". Lines 831-833 move the refusal of a batch from T10 to T8.
- ADR-0097's "Refines / supersedes" (`:6-12`) names only ADR-0027 and ADR-0095. ADR-0084 gets no Note.

**Why it matters.** Constitution, Governance (`docs/constitution.md:246-249`): a change that diverges from an accepted ADR "MUST open a new ADR … and MUST update the documents that depend on it". README rule 2 says the same. ADR-0095 supersedes ADR-0084 decisions 2, 6 and 11 only for a carry, so for an ingest decision 3 and decision 11 are still in force. The revision breaks them silently. The design's own FR-041-008 now contradicts an accepted MUST.

**Fix**
1. In ADR-0097's "Refines / supersedes", add that it would supersede ADR-0084 Revision decision 3 (the "before its commit" clause and "the commit-time finalizer's raise") and the ingest's reading of decision 11 (fences at the insert, a lease slot, the usage row on a refill).
2. Add a Note to ADR-0084 in the same form as the 2026-11-19 Note for ADR-0095.
3. Mention ADR-0084 in the README index line.
4. Say that on acceptance ADR-0084 is marked "partly superseded by ADR-0097".

---

## Major

### M1. A release locks every lease of the tenant, which brings back the hot row whenever the purge runs, and the deadlock-freedom rule is stated in contradictory terms

**Evidence**
- `ADR-0097:72-74`: a release "folds, in the finalizer's step-log branch **under the usage row**, every lease of the tenant, waiting for each in lease order".
- `ADR-0097:71`: "No transaction ever waits for a lease, so no lock cycle can contain one." This contradicts lines 72-74.
- `specs/041-step-logs/data-model.md:243-246`: leases are folded "after the fences and **before the usage row**", but `:223-224` defines I2 as "G = Σ `granted` … both written only while the usage row is locked". The fold writes `granted` before the row is locked.
- Neither the ADR's Consequences (`:102-103`, "a release (the purge, off the hot path)") nor design `:885-887` describes what happens to charges while a release holds the leases.
- The prototype does not build the release at all (`step_log_spike.rs`, doc comment of `install_leases`: "The release path (the purge) is not part of the spike").

**Why it matters**
- *Hot row returns.* Once a release holds leases 0..i, every charge of the tenant finds no free lease and refills. Each refill takes L4 and holds it to the end of its own transaction, including a whole report's commit. So for the length of every purge transaction (every 500 rows, at the end of the retention horizon, continuously for a tenant in steady state), the tenant's charges queue one at a time on the usage row. That is the queue the ADR exists to remove.
- *Lease growth.* During those windows refills create new lease numbers until the tenant has 256.
- *Policy changes stall logging.* A `tenant_step_log_bytes` change queued behind the purge's shared L3 blocks every new charge.
- *Deadlock risk.* If "under the usage row" is taken literally, a release holding L4 waits for a lease held by a carrying report with output, and that report then waits for L4 in its content finalizer at commit. That is a deadlock. The data-model order avoids it, but the ADR, which is the normative text, is ambiguous and contradicts itself.
- *Untested.* The path is unproved by the prototype.

**Fix**
1. Reword decision 5 as MUST rules: "A release MUST NOT hold the usage row while it waits for a lease; it locks leases, then the usage row, and writes the fold and the delta only after the usage row is locked (I2)."
2. Remove the "no transaction ever waits for a lease" sentence.
3. Better still, stop locking all leases. Under the usage row, fold only the leases that are free (`SKIP LOCKED`), then apply the delta when S ≥ delta. Otherwise raise a retryable refusal that the purge pass retries on its next run; the purge is off the hot path. In practice S ≥ delta for rows past their retention, because refills keep folding.
4. State the cost either way, and add a held-lock test: a release next to K writers that hold leases must not send them to refill (or does so at a measured rate).

### M2. Two different acceptance paths for one decision, and README rule 4

**Evidence**
- `ADR-0097:4-5`: "accepted under the same delegation once design 041's re-review closes".
- `specs/041-step-logs/research.md` O041-040: "Since 2026-11-20 ADR-0097 … would replace its delta and flush for a charge; its acceptance goes with this question". O041-040 is "asked with O041-038 once T041-35 passes".
- `ADR-0027:199-204`: "Nothing above changes until ADR-0097 and this amendment are accepted."
- O041-040's option (a), "accept as written", would accept the deferred-flush bullet (`ADR-0027:183-186`) that ADR-0097 removes.
- `docs/adr/README.md:12`: "An agent may draft and propose; a person accepts."

**Why it matters.** One decision can be accepted when the review closes or when T041-35 passes, by a delegate or through an owner question. Accepting O041-040(a) as written would contradict ADR-0097. OD-2026-11-20-6 delegates "design questions", but it does not mention README rule 4, and nothing records that ADR acceptance falls under the delegation.

**Fix**
1. Pick one trigger, for example "accepted together with O041-040 and O041-038 after T041-35's third run and the re-review".
2. Restate O041-040's options so that (a) means "accept the first and third bullets, with the charge as ADR-0097 states it".
3. Record in the OD-2026-11-20-6 row, or as a README note, that accepting a Proposed ADR is a delegated act until 0.3.0. Otherwise leave the acceptance to the owner.

### M3. ADR-0095 is now inconsistent with ADR-0097, and a cheaper alternative to the downgrade re-run was not weighed

**Evidence.** ADR-0095 is Proposed and was edited only in decision 2 (`:60-61`). These parts are now stale:
- Decision 5 (`:79`): "after any raise of the commit finalizer's step-log branch".
- Decision 7 (`:90-91`): "then at commit the fences of the classes the transaction charges … and the tenant usage row".
- Option 6 (`:111-112`): a savepoint is rejected because "a finalizer raise happens at commit, which a savepoint cannot catch".
- Consequences (`:123-126`): "takes the tenant's usage row at its commit even when its output stores nothing … at commit, one deferred flush".

In addition:
- ADR-0097 (`:11-12`) says ADR-0095's carry is "unchanged except that its rows are charged to a lease", yet the carry now also tries the tenant's three class fences in CA (design `:1011-1013`; data-model §8a CA).
- Design `:1000-1002` still says "Every lock the function takes is tried, never waited for … a carry adds no wait under the run lock", although a refill in CD waits for L4.
- Design `:993` still says "`tenant_quota` after the finalizer's refusal".

**Why it matters.** The superseding chain gives different rules in different texts. More importantly, the refusal now raises inside CD, inside the function. A single PL/pgSQL `BEGIN … EXCEPTION WHEN …` around CD would catch it with no extra round trip and record a mark-only result in the same call. That would remove the downgrade re-run for quota refusals and G041-021's window for that cause, at the cost of one subtransaction per carry. Option 6's stated reason for rejection is no longer true, so the trade-off is real and unrecorded.

**Fix**
1. Update ADR-0095 decisions 5 and 7, option 6 and the Consequences, plus design `:993` and `:1000-1002`.
2. Correct ADR-0097's "unchanged except" sentence.
3. Add "catch the lease refusal inside the function" as a considered option. Give its subtransaction cost, measured by the third run or a diagnostic row, and the reason it is chosen or rejected.

### M4. ADR-0097's evidence describes a different, cheaper mechanism, and misquotes one figure

**Evidence**
- `ADR-0097:121-122`: "a charge on per-tenant rows taken `SKIP LOCKED` costs the same as no charge at all (the completion rate within 1% to 2% paired, its p99 within about 1 ms)".
- What was measured was the 64-slot diagnostic, with slots pre-granted 1 TiB and a single `UPDATE`: `tests/performance/scalability-ha/step_log_diag.rs:259`, "the prototype measures where the lock is, not the refill". It had no fences, no limit read, no S/G read and no refill.
- The direct comparison `f-slot` / `f-none` is 0.988 with p99 +3.4 ms (`specs/041-step-logs/validation-report.md:503-504`), not about 1 ms.
- The chosen charge runs six nested statements (data-model `:253-256`), and its cost has not been measured.
- The strongest evidence for leases is not cited: at 8 × 128 there were 1,075 `canceling statement due to lock timeout` errors, all ingest COMMITs waiting on the usage row (`validation-report.md:559-562`).

**Why it matters.** An ADR's Evidence must support the decision taken (README template; constitution, Governance: "benchmark data"). As written it overstates what is known about the lease charge's cost.

**Fix**
1. Restate the slot figures as "a charge that only touches per-tenant rows `SKIP LOCKED`", with all three paired ratios, including +3.4 ms.
2. Cite the 8 × 128 lock-timeout finding.
3. Say that the cost of the lease charge itself is measured by T041-35's third run. If the `third` diagnostic preview ran (see m5), cite it here.

### M5. The quota usage readers do not know about leases, and the runtime role cannot read the lease table

**Evidence**
- `ADR-0097:77-78`: the usage the quota API reads "is S + Σ `used`, in one snapshot".
- `ADR-0097:51-53`: the runtime role "MUST have no privilege on the lease table". Data-model §4: "none: only the `SECURITY DEFINER` functions".
- The built code reads S only. `crates/infra-postgres/src/quotas.rs:1228-1235` is `SELECT coalesce(step_log_bytes, 0)::bigint FROM tenant_retained_usage`. The "exact usage used by quota views and reconciliation" helper is `content_usage.rs:146-173`.
- T041-5's file list (design `:2170-2176`) does not include `crates/infra-postgres/src/quotas.rs`.
- The prototype carry's allowance also reads S only (`tests/performance/scalability-ha/step_log_spike.rs:430`), so the third run does not measure the Σ `used` read that every carry (CB) and ingest (T6) will pay.

**Why it matters.** ADR-0027 (`:245`): "Byte resources expose exact charged bytes via `used`". With leases, `used` would read low by up to Σ `used` across 256 leases, and any reconciliation would report a drift that does not exist.

**Fix**
1. Add `quotas.rs:1228-1235`, and any reconciliation of the step-log class, to T041-5, reading through `sagat_quota_step_log_allowance` or a new definer accessor.
2. Test that `used` equals S + Σ `used` while a lease is held by an open transaction, and run that test as the runtime role.
3. Make the prototype's allowance read S + Σ `used` so the third run measures what will ship.

### M6. The down migration as stated would refuse forever

**Evidence**
- `ADR-0097:111-112`: "The down migration refuses while a lease holds room".
- Room (`granted − used > 0`) is only reclaimed by a refill or a release (`ADR-0097:109-110`). After every line is purged, leases still hold granted room and nothing will ever reclaim it.
- T041-5 (design `:2147`) says something different: "a down that refuses while a line exists and otherwise restores what it replaced".

**Why it matters.** A migration's `down` must be safe and usable. As stated in the ADR it can never run after real use, and the ADR and the task disagree.

**Fix.** The down refuses while a line exists. Otherwise it asserts S + Σ `used` = 0, drops the leases and `step_log_leased_bytes`, and restores the previous functions. Test it on the pattern of `undo_to`, with leases that still hold room. Correct the ADR's sentence.

### M7. Leases and conservative refusals have no telemetry, and a false refusal is shown to the user as "quota reached"

**Evidence**
- G041-023 (research) and `ADR-0097:106-108`: a refusal "while room exists only in leases that open transactions hold … is a counted drop (`tenant_quota`)".
- Design `:992-994`: the reason `tenant_quota` is recorded as the stream's `ceiling_reason`.
- Design `:834`: the batch's resend "T6 reads the tenant over the limit". In the G041-023 case that is false: T6 reads S + Σ `used`, which is below the limit, so the SDK keeps retrying until its 10 s run out.
- No metric, gauge or log covers refills, reclaims, lease count, the direct charge when all 256 leases are held, or a release's lease wait (design metric list `:2188-2190`; FR-041-018).

**Why it matters.** Constitution VI and AGENTS rule 10 require typed errors and observability. A stream that shows "tenant quota met" while usage is below the limit misleads both operators and readers. The size of G041-023 (up to the number of concurrent charging transactions × 1 MiB) cannot be seen.

**Fix**
1. Give the conservative refusal a constraint name of its own (for example `tenant_step_log_quota_held`), mapped to a closed drop reason such as `tenant_quota_held` and to `step_log_quota_retries_total{reason}`.
2. Have the purge pass publish a gauge of leased room (G − Σ `used`) and of lease rows, both bounded and without tenant labels, as ADR-0027 requires for metric labels.
3. Correct design `:834`.
4. Name the tests in T041-5 and T041-6.

### M8. The lock-order fitness function as specified would reject the order the design defines

**Evidence**
- `tests/support/lock_order.rs:79-80`: `Usage` means "Quota, rate and usage rows, always last". `assert_ascending` (`:85-95`) refuses a plan that returns to a lower slot.
- Design `:877-880`: new slots for "L1, L1b, L2 and L3b placed after its `Bookkeeping` slot and before `Usage`". L3, the quota class fences, gets no slot of its own, so it falls under `Usage`.
- A charge takes L3 and then L3b, which descends from `Usage` to L3b.
- The carry takes L3 in CA before L2 in CC′ (design `:1011-1013`, `:1023`).
- The harness has no way to model a lock that is only tried.

**Why it matters.** The fitness function that ADR-0097's "Enforced by" (`:18`) relies on would either fail, or be bent so far that it checks nothing. ADR-0027 (`:57`) also forbids new entity locks after the usage row. A refill at T8 or CD takes L4 mid-transaction, and the "nothing after L4" rule is not checked anywhere.

**Fix**
1. Give L3 its own slot, before L3b and before `Usage`.
2. Model tries explicitly: tries are exempt from ordering, and a test proves they never wait (`pg_stat_activity`).
3. Add a lock-order case with a refill held: the ingest at T9 and the carry at CE take no new lock on an existing row after L4.
4. Name these cases in T041-6 and T041-36.

### M9. D041-111 drops the stream run index without evidence, using reasoning it rejects for the lines' index

**Evidence**
- `research.md` D041-111: dropping a read index of the lines is rejected ("no measurable gain in rate; the readers' plans depend on them"), yet `step_log_streams_run_idx` is dropped.
- No diagnostic row isolates the stream run index. `c-noidx` concerns the lines (`validation-report.md:548`).
- `(tenant_id, run_id, stream_id)` never changes on update, so the index does not block HOT. Dropping it only saves one index insert per new stream.
- Readers: `listRunLogStreams` pages "with `cursor` and `limit`" (design `:212`), and the delete route locks "`ORDER BY stream_id`" (`:1240`). Both would go through the prefix of `attempt_uniq` plus a sort, which is O(streams per run) per page.
- The justification in data-model `:93-94`, "a run has at most its attempts' streams", has no bound. A `forEach` or loop run can have thousands of attempts.

**Why it matters.** This is a hard-to-reverse storage choice, justified by the same kind of argument the same decision rejects elsewhere, and it risks a read-path regression with no matching gain.

**Fix.** Keep `step_log_streams_run_idx`. Alternatively, measure its removal as its own diagnostic row, state a bound on streams per run, and add a plan test (`EXPLAIN`) for `listRunLogStreams` at that bound.

---

## Minor

### m1. The considered options misstate the owner's rule and leave out two real alternatives
- `ADR-0097:94` rejects striped counters with an overshoot "which the owner's rule forbids". But `:41-42` quotes the rule as forbidding an overshoot "unless bounded, proved and documented", and the overshoot in question is bounded ("the pool size times a 24 KiB carry").
- Missing option: locking only near the limit, with a margin that bounds the charges in flight. ADR-0084 itself once used this ("within 2 MiB of the tenant's log limit", ADR-0084 `:53`).
- Missing option: deferring leases. D041-106 found the queue was not the cause of the 1 × 16 failure.
- **Fix:** correct the rejection reason (complexity and false refusals, not the rule), and add both options with their trade-offs.

### m2. Decisions 2 and 4 to 6 are descriptive rather than MUST rules (README rule 3)
- Rules that the design's safety rests on are only implied: "a charge MUST NOT wait for a lease" (stated), "a release MUST NOT hold L4 while waiting", "after L4 no new lock on an existing row", "readers MUST read S + Σ `used` through a definer accessor".
- **Fix:** rewrite them as MUST / MUST NOT and map each to its check.

### m3. The enforcement switch is ignored by the lease charge
- `quota_accounting_control.step_log_enforcement_enabled` (ADR-0027 Amendment item 3; data-model `:184`) is not read by the charge or the refill (data-model §3b `:231-242`). If an operator turns enforcement off, the leases still refuse.
- **Fix:** define the not-enforced behaviour (grant g = n − room with no limit check; usage still exact), read the flag after the fence, and test it.

### m4. Two lease paths have no test
- The direct charge of S when "all 256 leases are held" (`ADR-0097:70-71`) and the creation of new lease numbers up to the cap are not covered. A property test will not reach 256 concurrent holders.
- **Fix:** take the cap from a constant the test can lower (for example a `quota_accounting_control` column or a function argument), and name tests for both paths and for the `CHECK (lease BETWEEN 0 AND 255)` raise.

### m5. The third run changes its setup without declaring it, and a preview's result is missing
- `tests/performance/scalability-ha/main.rs:2162` now sets `TenantStepLogBytes` to its maximum in `step_log_carry_spike`, which also applies to any second-run rerun. The T041-35 third-run text (design `:2018-2028`) says "Everything else is the second run's".
- The shared repository has `refs/remotes/origin/perf/041-diag-third` at `0b3f14b3`, a push of the `third` preview set, but no result is recorded in `validation-report.md`.
- The gates, thresholds and validity rule are unchanged (D041-109 holds).
- **Fix:** declare the limit change in the T041-35 third-run text. Record the preview's run id and figures, or state that it did not run. Pin the commit that freezes the third run's protocol before its first round.

### m6. Hand mutations and prototype tests are claimed with no recorded evidence
- `ADR-0097:21` and `:126-128` claim "six hand mutations each failing one". Neither `validation-report.md` nor `research.md` records them.
- The tests are `#[cfg(all(test, feature = "perf-spike"))]` (`step_log_spike.rs:499`). No CI step runs them: `perf-spike.yml` builds with `--no-run` and then runs the rounds. Yet `architecture-principles.md:31` says "until then the prototype's … tests".
- **Fix:** record the command and output of each mutation run (AGENTS rule 17), and either add a step to `perf-spike.yml` that runs `leases_tests` or say they are run by hand, with the command.

### m7. Stale or incomplete text elsewhere in the design and data model
- Design `:1200-1205` (Grants) lists no lease functions.
- Constitution Check VIII (`:1677`) still says "a carry refused at commit by the step-log quota" and "G041-001 to G041-022".
- Complexity Tracking (`:1680-1699`) has no row for the lease table, its column and functions, or the release branch.
- The ADR says "three functions" (`:111`), while data-model `:228` says "Three `SECURITY DEFINER` functions" and lists two plus a branch.
- The SECURITY DEFINER and `proconfig` check list in T041-5 (`:2155-2158`) leaves out `sagat_quota_step_log_charge` and `sagat_quota_step_log_refill`.
- T041-5's "Waits for" and `Owner-gate:` lines (`:2168-2169`) do not cite D041-108 or ADR-0097.
- **Fix:** update all of these in this revision.

### m8. Every step-log charge now holds the content and log fences, which is not in the Consequences
- A charge takes the content and log fences shared at the insert (`ADR-0097:61-62`). A change of `tenant_content_bytes` or `tenant_log_bytes` therefore waits for in-flight ingests. While such a change is queued, every carry of the tenant is `contended` and loses its records (design `:1013`).
- **Fix:** state this in the Consequences, together with why all three fences are needed (multi-resource editors through `lock_policies` in resource-name order).

### m9. The cost of the fill factor was not weighed
- `WITH (fillfactor = 70)` (data-model `:92`) applies to a table whose rows are kept until feature 020 deletes runs (design `:1236`): about 43% more heap, permanently. It helps HOT only on long-step updates. The purge's `removed_at` and `purged_at` updates touch columns in partial-index predicates and are never HOT.
- **Fix:** record the cost in D041-111 and have T041-9 measure size per row with and without it.

### m10. No supersession marker is promised for ADR-0027
- ADR-0095 says that on acceptance ADR-0084 and ADR-0027 are marked "partly superseded" (ADR-0095 `:11`). ADR-0097 does not say this for ADR-0027 (README rule 2).
- **Fix:** add "On acceptance ADR-0027 (and ADR-0084, B1) are marked 'partly superseded by ADR-0097'".

---

## Nit

### n1. The lease probe order does not match its description
- `ADR-0097:63-64` says the order "starts at the backend's own slot (`pg_backend_pid()` modulo 256)". `ORDER BY (lease + pg_backend_pid()) % 256` (`step_log_spike.rs:352`) actually starts at −pid mod 256.
- **Fix:** reword, or use `(lease - pid) % 256`.

### n2. The fence order relies on target-list evaluation order
- The three fences are taken in one `PERFORM a, b, c` (data-model `:231-232`; spike `install_leases`). That relies on PostgreSQL evaluating the target list left to right to keep ADR-0027's order.
- **Fix:** say so in a comment, or check the order in the lock-order test.

### n3. The test count is ambiguous
- `ADR-0097:21` says "five tests", but `step_log_leases_tests.rs` has nine.
- **Fix:** say "five lease tests" and name them.

### n4. Round artifacts keep the second run's name
- `carry_spike_rounds.sh` still writes `second-round-$round.*` during the third run.
- **Fix:** rename, or note it in the script header.

---

## Checked with no finding

- **Layering and ports.** Nothing enters the domain. The carry still reaches the step-log tables through one method of `StepLogCarryTx`. The refusal is mapped in the adapter (`StoreError::StepLogRefused`, ingest `RETRY_LATER`). The spike hook is behind a feature gate (`crates/infra-postgres/src/lib.rs:51-53`), and there is no `serde_json` in the domain.
- **Scope.** Leases for the content class are deferred as G041-024, outside 041. No other branch or worktree does this work: the refs are `feature/041-step-logs`, `feature/041-step-logs-server` and `perf/041-*` only.
- **ADR-0027 Note under rule 2.** A pointer Note that changes nothing until acceptance is acceptable and matches the Notes of 2026-11-16 and 2026-11-19. It needs the supersession marker of m10 and the acceptance alignment of M2.
- **Exactness proof (I1 to I5).** The charge, refill and reclaim paths of the prototype keep U ≤ S + G ≤ L. The weak points are the unbuilt release (M1), the readers (M5) and the 256-held path (m4).
- **T041-35 third run.** The gates, thresholds, validity rule, environment and median-of-three rule are unchanged (D041-109), and the judge script is untouched. See m5 for the undeclared setup change and the unrecorded preview.

## Count

| Severity | Count |
|---|---|
| Blocker | 1 |
| Major | 9 |
| Minor | 10 |
| Nit | 4 |
