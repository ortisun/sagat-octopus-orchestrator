# Goldfish Review Report

**Target:** `docs/designs/041-step-logs.md` (`lines 1-2063`, everything before "## Detailed Implementation Plan"; commit `ff8033cd`)

EGM stage 7, Critic Review, of the revision since the approved commit `f9d4a65f`. A finding about unchanged text is kept only when the revision made it wrong. Line numbers are those of the target at `ff8033cd`.

## Reviewer Status

| Reviewer | Status |
| --- | --- |
| `egm-design` | completed |
| `egm-structural` | completed |
| `egm-style` | completed |

The synthesizer checked the load-bearing quotations of D-1, D-2, D-4, D-6, D-8 and D-9 against the target, `specs/041-step-logs/data-model.md` and `specs/041-step-logs/validation-report.md`.

## Prioritized Findings (Blockers -> Major -> Nits)

### D-1 SC-041-007 states no criterion that T041-9 can pass at 1 × 16, while the release still stops on its failure - Severity: Blocker
* **Reviewer Lens:** `egm-design`, `egm-structural`, `egm-style`
* **Affected Scope:** `Lines 374-386`, `405-408`, `438-452`, `1884`, `2038-2042`
* **Discrepancy Description**: SC-041-007 still states the original criterion: at least 90% of the capture-off rate, and the p99 within 10% or the larger of 10% and 2 ms. It then appends "Settled by the measured cost ... at 1 × 16 the rate of workload (a) is 82.0% ... gate (2b) is +8.6 ms over the allowance". This is a record, not a pass/fail rule. Other passages still treat the original figures as binding:
  * The Goals (405-408) promise "the rate within 90%".
  * The Open Question (450-451) says "T041-9 measures all three before the release, and a failure there stops the release".
  * The risk row at 1884 promises "the completion p99 within 10%".
  * NFR-041-PERF (2038-2042) binds SC-041-007 to T041-9.

  T041-9 runs the same workloads at both sizes, so at 1 × 16 it will fail the stated criterion and stop the release. The design never says what T041-9 must meet there: 90%, the accepted 82.0% / 87.1%, or the fourth run plus a tolerance. That breaks the design's own rule of fixing the criteria before a run.

  The paragraph has three further problems:
  * It understates the latency cost. `validation-report.md` records the p99 of workload (a) at about +32% to +37% over capture off at 1 × 16 (34.6 to 46.2 ms), not just "+8.6 ms over the allowance".
  * It contradicts itself on the environment: "on one machine, the same day" against "four hosted runners of 4 cores".
  * It uses labels this document never defines: "workload (a)", "gate (2b)" and "the allowance".

  The Open Question at 438-452 also reads as still open. It keeps "If it fails, the feature stops for a design change" next to "so no design change follows".
* **Rationale & Recommendation**: Split SC-041-007 into three parts:
  * The criterion T041-9 is judged against at each size, fixed now and before T041-9 runs. For example: 8 × 128 keeps 90% and the allowance; 1 × 16 must be no worse than 82.0% / 87.1% and the measured p99 cost within a stated tolerance, or is recorded only.
  * The fourth run's result, with the p99 cost given in percent and the environment stated correctly.
  * The accepted deviation (OD-2026-11-20-13).

  Then align the Goals, the Open Question (moved to an answered form), the risk row at 1884 and NFR-041-PERF with the first part.
* **Recommended disposition: fix** The design leaves the release gate unverifiable, and only the design can name the threshold before T041-9 runs.

### D-2 The admission defaults are still placeholders, because the step that sets them never ran - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** `Lines 766-778`, `957-969`
* **Discrepancy Description**: Both admission defaults "are placeholders until T041-35 measures the ingest's service time ... and records the defaults it sets in data-model.md §6" (777-778, 963-967). They are a cap of a quarter of the pool (5 of 20) and 2 tenant tickets. Only the passing branch of T041-35 sets them. T041-35 closed by owner acceptance instead, and `data-model.md` §6 (line 408) still says "placeholders until then".

  The fourth run measured that workload (g) uses 7 tickets at 1 × 16 and 5 at 8 × 128. That is above the 2-ticket default. Under the sizing rule ("the cap at least the tickets"), 7 tickets also need a cap above the quarter-of-pool default. Shipped as written, a tenant at the benchmark rate would meet repeated `RETRY_LATER`, and its batches would be dropped after 10 s. T041-9 runs "with T041-7's defaults", which are undefined.
* **Rationale & Recommendation**: Apply the stated sizing rule (tickets ≥ rate × S p99 / 0.7, and the cap at least the tickets) to the fourth run's (g) figures now. Record the result in `data-model.md` §6 and remove "placeholders" from sections 1 and 2. If that is not done, name the task that sets the defaults before T041-7.
* **Recommended disposition: fix** The measured figures already exist, and the shipped defaults are below what the benchmark used.

### D-3 Capture on by default was decided before its cost was known, and no risk row weighs it - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** `Lines 383-386`, `470-473`, `1881`
* **Discrepancy Description**: O041-001 (capture on by default at `info`) was answered when SC-041-007 promised at most a 10% rate cost. The accepted cost at 1 × 16 is now about 18% of the rate and about +35% completion p99. Every tenant pays it on upgrade, without opting in. The only risk row about the default (1881) covers privacy. OD-2026-11-20-13 accepts "the measured cost" but does not say that the default was weighed against it.
* **Rationale & Recommendation**: Add a risk row for the throughput and latency cost of default-on capture at small deployments, with its levers: the namespace switch and the deployment switch. Either record that OD-2026-11-20-13 covers the default, or put O041-001 back to the owner with the measured figures.
* **Recommended disposition: fix** A one-row risk entry and a sentence tying the default to OD-2026-11-20-13 make the accepted trade visible at almost no cost.

### D-4 "None is open", but G041-018 still waits for the owner, and a settled item sits under Open - Severity: Major
* **Reviewer Lens:** `egm-design`, `egm-structural`, `egm-style`
* **Affected Scope:** `Lines 458-465`, `1899`, `1922`, `1933`
* **Discrepancy Description**: Line 460 says "None is open since 2026-11-20". Three other passages still leave G041-018 open for the owner:
  * The risk row at 1899: "Open as G041-018; for the owner when the revision is approved".
  * The row at 1922: "the residual ... is G041-018, open".
  * The Constitution Check, principle V (1933): "One condition not yet accepted: G041-018 ... put to the owner with this revision's approval".

  `research.md` G041-018 frames it as an owner choice: show `used` only with the grant in every namespace, or accept the residual in writing. Principle V's "Pass with conditions" therefore rests on a question that has no number and is not listed as open. Meanwhile, the only bullet under "### Open" is O041-026, which the text calls "Settled 2026-11-20 without being put".
* **Rationale & Recommendation**: Either list G041-018 under Open as an owner question, with its options and a recommendation, or record the owner's answer and update 1899, 1922 and 1933. Move O041-026 to Answered, and reword line 460, for example "No question is open as of 2026-11-20".
* **Recommended disposition: fix** The owner-decision register must match the Constitution Check, or the approval hides a pending owner choice.

### D-5 The ensure delays a report with no hard bound below the pool's 5 s statement timeout - Severity: Major
* **Reviewer Lens:** `egm-design`
* **Affected Scope:** `Lines 24-28`, `171-178`, `929-933`, `1227-1239`, `1284-1291`
* **Discrepancy Description**: The ensure, new in this revision, runs in its own best-effort transaction. Its only limit is "each lock acquisition ... at most 200 ms", and line 1289 concedes that "a wait spanning several holders can exceed *b*". `data-model.md` gives the ensure no statement timeout of its own, so the pool's 5 s applies.

  A report can therefore wait up to about 5 s in the ensure, then up to about 5 s more in the fallback under the run lock. FR-041-005 accepts that this delay can push a report past its lease's expiry and get it refused as late. The step is then retried, so logging can cause an activity to run again. G041-021's "the window is the length of one failed report transaction" no longer describes this.
* **Rationale & Recommendation**: The ensure's failure is already harmless ("proceed as if no ensure had run"). Give its transaction a `SET LOCAL statement_timeout` of a small multiple of *b* and map `57014` to `Error`. Then state the real worst-case delay before a report in FR-041-005 and in "What a report can wait for".
* **Recommended disposition: fix** One local setting on a transaction that may already fail removes a multi-second wait that can re-run activities.

### D-6 The fallback is called "rare" and bounded by 200 ms, without the fourth run's measured frequency or its 726.7 ms carry - Severity: Major
* **Reviewer Lens:** `egm-design`, `egm-style`
* **Affected Scope:** `Lines 26-28`, `935`, `1291-1293`, `1367-1368`, `1890-1891`, `1909`
* **Discrepancy Description**: The Overview (27), section 2 (935) and the risk rows (1890, 1891, 1909) call the fallback "rare far from the limit and with session affinity". They cite "each lock acquisition at most 200 ms" as its mitigation. Line 1292 says the harness records the longest refill-written carry, but the design never gives the result.

  `validation-report.md` (lines 794-802) does give it. The longest refill-written carry was 726.7 ms in (h) at 1 × 16, 3.6 times *b*, with the run row held throughout. Refills there were 2.0% of carries. The design itself says fallbacks are "no longer rare" near the limit (1367-1368).
* **Rationale & Recommendation**: Replace "rare" with the measured rates and their regimes. Report the 726.7 ms maximum and the worst run-row hold in the risk row at 1891 and in section 2b. Give T041-9's near-limit and fan-out regimes a criterion on the longest run-row hold, not only on the count.
* **Recommended disposition: fix** The figures are measured and in the repository, and the mitigation as written is contradicted by them.

### D-7 The Goals, an Open Question and a risk row misstate what capture adds to a report - Severity: Major
* **Reviewer Lens:** `egm-design`, `egm-style`
* **Affected Scope:** `Lines 401-404`, `448`, `1885`
* **Discrepancy Description**: These passages predate the one-call carry:
  * Line 402 says "one statement to the report ... (six for a long step ...)".
  * Line 448 says "the round trips (five for a short step)".

  Section 2a and its table (1029, 1145, 1181-1182) say one call and one round trip for any carry. Line 1950 confirms that "five round trips" belonged to the first carry spike.

  Two further claims leave out the ensure:
  * Line 401 and the risk row at 1885 say a short step adds no transaction.
  * Line 403 says a short step "reports without waiting".

  The ensure adds a transaction under no run lock whenever the kept room is short, unknown or stale (1147). It also waits on the server side before the report.
* **Rationale & Recommendation**: Restate these lines from the section 2a table: one call for any carry, plus one ensure transaction in the cold case, and the wait that ensure can add (see D-5).
* **Recommended disposition: fix** The Goals are the contract readers check first, and they now contradict the Technical Plan.

### D-8 The latest revision note says "nothing in the Technical Plan" changed, but section 2b did - Severity: Major
* **Reviewer Lens:** `egm-structural`, `egm-style`, `egm-design`
* **Affected Scope:** `Lines 32-36` (claim), `1209-1210`, `1252-1254`, `1302-1303`
* **Discrepancy Description**: The note "Revision of 2026-11-20, after the owner's answers" ends "nothing in the Technical Plan". Section 2b cites decisions that only that revision could have written:
  * "(accepted 2026-11-20, OD-2026-11-20-15)" at 1210.
  * "("Accept the visible loss", OD-2026-11-20-14)" at 1253-1254.
  * "accepted as built: OD-2026-11-20-14" at 1303.

  Under the amendment rule (64-65), re-reviewers scope their work by this list, so a wrong list hides changed text from them.
* **Rationale & Recommendation**: List the section 2b edits, and any other Technical Plan edits, in the "Changed:" sentence, or remove "nothing in the Technical Plan".
* **Recommended disposition: fix** The approval rule depends on this list being accurate, and correcting it is a one-line edit.

### D-9 The pg_stat_statements sentence of the budget measure is garbled and contradicts itself - Severity: Major
* **Reviewer Lens:** `egm-structural`, `egm-style`
* **Affected Scope:** `Lines 1159-1169`
* **Discrepancy Description**: New text reads "in the carrying cases the test sets `pg_stat_statements.track = 'all'` for nothing: the test support already starts its PostgreSQL with `pg_stat_statements.track=all` ...; the carrying cases first assert ... and change no server setting) and asserts, as an equality, ...". The parenthesis has no opening. "Sets ... for nothing" contradicts "change no server setting". The T041-36 implementer cannot tell whether the test changes the setting. The 26-statement breakdown that follows nests three levels of parentheses.
* **Rationale & Recommendation**: Rewrite it as three sentences. First, the test sets no server setting and asserts `current_setting('pg_stat_statements.track') = 'all'`, failing with a message naming the setting otherwise. Second, what it measures. Third, the pinned figure. Give the 26 statements as a list.
* **Recommended disposition: fix** This sentence specifies a test, and as written it gives the implementer two opposite instructions.

### D-10 "The ensure" and its companion terms are used from the Overview on but defined only in section 2b - Severity: Major
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** `Lines 24-30`, `171-178`, `211`, `635`, `1227`
* **Discrepancy Description**: The Overview's sentence at 24-30 depends on "the ensure that makes its writer's step-log lease's room before the report", a chain of possessives around a noun the reader has not met. The Terms list (635, "used from the Overview on") defines *Lease*, but not *ensure*, *fallback refill*, *reservation* or *kept room*. The ensure is first explained at line 1227.
* **Rationale & Recommendation**: Add these four terms to Terms. In the Overview, write the ensure out, for example "a separate transaction, run before the report, that tops up the writer's quota lease".
* **Recommended disposition: fix** These terms carry the revision's main mechanism, and defining them costs four Terms entries.

### D-11 "What a report can wait for" is a 27-line paragraph, though it is the normative latency bound - Severity: Major
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** `Lines 929-955` (and the restatements at `24-30` and `168-178`)
* **Discrepancy Description**: The bound on report latency is one paragraph of chained semicolon clauses. It covers the ensure wait, the fallback wait, the waits at commit, the try-only locks and the effect on SC-041-007, and it repeats itself ("While it waits ... queue behind it meanwhile", 938). FR-041-005 and the Overview restate it in prose, so the three versions can drift.
* **Rationale & Recommendation**: Make it a numbered list of waits. For each wait, give when it happens, which lock, whether the run lock is held, and its bound. Have FR-041-005 and the Overview point to the list instead of restating it.
* **Recommended disposition: fix** D-5 changes this bound anyway, and a list keeps the three restatements from drifting.

### D-12 The Overview opens with about 40 lines of revision history before the design - Severity: Major
* **Reviewer Lens:** `egm-style`
* **Affected Scope:** `Lines 32-75` (also `1202-1221`, `1399-1413`)
* **Discrepancy Description**: Four stacked revision notes come before the reader learns the design. The Technical Plan also tells spike history: "The shared pool of 256 leases this section first had collided" (1218), "As built in the spike's second run" (1204), "T041-35's third run, which states what follows a third failure" (1412-1413). Inline decision identifiers make many sentences hard to read.
* **Rationale & Recommendation**: Move the revision notes to a "Revision history" section at the end. Keep the plan in the present tense, and leave the reasons for each change to the cited decisions.
* **Recommended disposition: accept** The repository's amendment rule asks re-reviewers to scope by these notes in the Overview, so moving them is a convention change for `docs/designs/`, not a defect of this revision. The stale history sentences are fixed under the Nits.

### Nits

1. **Stale date.** Line 5 says "Date: 2026-11-19", but three revisions dated 2026-11-20 follow. (`egm-structural`, `egm-style`)
2. **Garbled ADR list.** Lines 81-84: "ADR-0085 and, ADR-0095 and ADR-0097, both accepted" has a stray "and,". (all three)
3. **Stale ADR statuses.** Lines 51 and 63 still say ADR-0097 and ADR-0095 are "Proposed" and should mark that as their status at the time. Line 1140, "superseded by ADR-0095 decision 2 when accepted", should drop "when accepted", since ADR-0095 is Accepted. (all three)
4. **Unclear governing acceptance.** Lines 73-75, "Before this revision, the owner's acceptance ... decides scope", mixes tenses and does not say which of the four revisions is meant, or which acceptance governs now. (`egm-structural`, `egm-style`)
5. **Text written before the fourth run.** These should state the outcome (D041-122, D041-123):
   * Line 1888: "T041-35's fourth run measures the result against the unchanged gates".
   * Line 1893: "its fourth run closes G041-025 only with none". G041-025 closed even though there was one `mark_only:busy` answer.
   * Lines 1412-1413: "T041-35's third run, which states what follows a third failure".

   (all three)
6. **Two wordings of the p99 allowance.** O041-039 (b) (538-539) says "the capture-off p99 plus 2 ms". SC-041-007 (377-379), line 355 and line 406 say "the larger of 10% of it and 2 ms". Name the decision that widened it, or align the wordings. (all three; `egm-style` rated it Major)
7. **Two bounds for the ensure's `invalid` answer.** The design (1233-1234) bounds it by the largest valid carry. `data-model.md` uses the attempt ceilings, which are far larger. Use one bound, preferably the carry's own, in both documents. (`egm-design`)
8. **Undefined *K*.** Line 338, "K − 1 more ingests of T", never defines *K*. (`egm-design`)
9. **ADR-0027 left out and cited at the wrong amendment.** The Overview's ADR links (77-86) leave out ADR-0027, which this revision amends. FR-041-008 (216) and section 3 (1423) cite only its 2026-11-15 amendment, though the per-statement measurement now comes from the amendment of 2026-11-20 (OD-2026-11-20-16). (`egm-structural`)
10. **Repeated subject.** Line 1264: "the ensure needs the ensure and the report to run on the same server backend". (`egm-structural`, `egm-style`)
11. **Answered list out of order.** In 466-560, O041-036 (2026-11-19) comes after the 2026-11-20 entries. Lines 561-562 hold a double blank line. (`egm-structural`, `egm-style`)
12. **Diagram border.** Line 699 is missing the right-hand `|`, and line 690 shifts the right column. (`egm-structural`, `egm-style`)
13. **Ambiguous revision.** Line 1202 says "The revision of 2026-11-20", but three revisions carry that date. (`egm-style`)
14. **Vague quantifiers.** "As almost every one does" (166-167), "a little longer" (968) and "well under" (979) should give a bound or go. (`egm-style`)
15. **Garbled clause.** Lines 1185-1186: "and the batches that what does not fit its carry needs". (`egm-style`)
16. **Re-run count.** The risk row at 1889 says the report "re-runs ... once", but 1116-1119 allow up to four transactions. (`egm-style`)
17. **SC-041-001 table.** In 327-335, "carries a mark-only carry" repeats a word, the "51, 1, 11" cells do not say they are statements, transactions and rows, and the pointer to "What a report pays" should point to "The budget's measure changes" (1154-1157). (`egm-style`)
18. **Duplicate alert.** Line 1934 names `SagatStepLogQuotaHeldRefusals` twice. (`egm-style`)
19. **Grants written twice.** The SECURITY DEFINER and grants sentence appears at 1433-1443 and again at 2043-2051. Keep one and refer to it. (`egm-style`)
20. **Informal wording.** "Settled ... without being put" (462), "outwaits" and "the report always lands" (543), "the question is put again" (544), and "or short sends it to the fallback" (949). (`egm-style`)
21. **Run-on requirement.** FR-041-005 (168-178) is one 11-line sentence followed by the fragment "The residual:". Split it, or point it to the list proposed in D-11. (`egm-style`)

Counts: Blocker 1, Major 11, Nit 21
