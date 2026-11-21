# Goldfish Review Report

**Target:** `docs/designs/041-step-logs.md` (lines 1-3214, full file, at commit ff8033cd933d0c6adf972706eb1bb38407bdb7c6)

EGM review stage 8, the Intermediate Zoom Out, of the revision of the design approved at f9d4a65f. Two independent instances (A and B) of the zoom-out reviewer read the whole file. Instance A read it front to back. Instance B traced it from the Estimate and the Verification Plan back to the Goals. Their findings were deduplicated here. The findings of stage 5 (`zoom_out_report.md`, Z-n), stage 6 (comprehension) and stage 7 (`critic_report.md`, D-n) are not repeated. A finding that builds on one cites it.

**Verdict at altitude.** The core shape still holds and was measured: the last records ride on the report, long steps stream batches, and storage is one row per line beside the journal. The mechanism holds together. What no longer holds together is the proof and governance layer around it, because each revision added a mechanism without carrying the earlier layers forward:

- The Verification Plan and its gap list were last reconciled before ADR-0097.
- The only task that judges the revision's new mechanism lands last.
- The owner's O041-041 rule does not match the regimes that would trigger it.
- The pooler remedy contradicts the scaling runbook.
- The owner-gate lines miss the 2026-11-20 decisions.

A simpler shape is still available. Z-1 (fallback only, no ensure), Z-3 (one re-run tier) and I-4 (no order guard) together would remove these, with no loss of any guarantee a conforming SDK needs:

- the ensure cache;
- `CarryRoom` and `StepLogRefused`;
- `quota_rerun`;
- `last_stored_xid`, `in_order` and `out_of_order`.

## Reviewer Status

| Reviewer | Status |
| --- | --- |
| `egm-zoom-out` (instance A) | completed |
| `egm-zoom-out` (instance B) | completed |

## Prioritized Findings (Blockers -> Major -> Nits)

No Blockers.

### I-1. The Verification Plan does not cover ADR-0097, though the Constitution Check says it does - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (A and B)
* **Affected Scope:** lines 3154-3200 (Verification Plan, table 3163-3188, gaps 3190-3200), 1936 (Constitution Check, principle VIII), 338-345 (SC-041-001's exactness and lease bullets)
* **Discrepancy Description**:
  - The Verification Plan has no row for the revision's main mechanism. Nothing covers:
    - quota exactness under concurrent charges, refills, reclaims, releases and limit changes (SC-041-001's fourth bullet, ADR-0097 invariants I1 to I5, and the interleaving property test of T041-5, lines 2547-2549);
    - the held-room bound (lines 1318-1332, 2560-2561);
    - the deadlock cases of section 2b (lines 1304-1317);
    - the ensure, its answers, the affinity miss and the at-most-two-waits test (lines 2826-2840);
    - a fallback past its budget answering `tenant_quota_busy` while the report lands;
    - the idle, ended-backend and limit-change reclaims.
  - The gaps paragraph (3190-3200) stops at G041-017. It omits G041-018 to G041-027, the residual risks this revision created: held room, the content queue, the busy loss the owner accepted (G041-026), the pooler (G041-027) and G041-018, which is still the owner's.
  - Principle VIII (line 1936, "Pass") claims that the plan names exactly these tests and that "What is not proven is listed as gaps G041-001 to G041-027". The compliance evidence cites text that does not exist.
  - Row 3171 also still says the duplicate batch "waits on the stream row". It now waits on the stream lock at T3.
  - The diff shows that no 2026-11-20 revision touched the table. This is broader than stage 6's stale-row findings: what is missing is the proof map for ADR-0097 as a whole.
* **Rationale & Recommendation**:
  - Add rows for the lease exactness and interleavings, the held-room bound, the deadlock and reclaim cases (T041-5), and the ensure answers and the busy fallback (T041-36, or their removal if Z-1 is adopted). Add rows for the T041-9 regimes.
  - Extend the gap paragraph to G041-027, with each gap's status (accepted, open, closed).
  - Re-derive principle VIII from the corrected table instead of asserting it.
* **Recommended disposition: fix**. The Constitution Check passes on evidence the document does not contain, so the gate is not met as written.

### I-2. T041-9, the only judge of the new quota mechanism, lands last, though it needs only the server - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (A and B); builds on Z-1, Z-5/D-1 and D-6
* **Affected Scope:** lines 3057-3104 (T041-9, regimes 3073-3101), 2069-2070 (the order rule), 2796-2797, 1267-1268, 1369-1372, 450-451, 2528-2606 (T041-5)
* **Discrepancy Description**:
  - By the design's own account, only T041-9 measures the ensure, the pooler behaviour, the frequency of near-limit fallbacks, the cold start and the run-row holds in a fan-out. ADR-0097 says decision 3b "is measured by task T041-9".
  - By the order rule ("Tasks are listed in the order they land"), T041-9 lands after Slices C, D and E: the routes, six SDKs and Studio.
  - It needs none of them. Its driver calls the application in-process (line 3061), T041-36 ports the driver's mode (2796-2797), and T041-7 and T041-10 land before T041-36.
  - A failure that reshapes the quota machinery would therefore surface after about 20 more pull requests have been built on it. That gives up the measure-before-commit practice that T041-31 and T041-35 applied before the schema.
  - Only the pooler regime states what a miss leads to. Cold start, near the limit, small quota, many low-volume tenants and the fan-out each state a criterion with no consequence, and "Done when the figures are recorded" makes them advisory. This contradicts T041-35, which fixed "How a result is read" before its first round.
  - T041-5 is still one pull request holding the schema, the accounting, about twelve lease functions, the ensure, the reclaims and their property and mutation tests.
* **Rationale & Recommendation**:
  - Split the quota-regime part of T041-9 (and the SC-041-007 runs on the built carry) into a task that lands right after T041-36 and T041-8, before Slice C (T041-14).
  - For each regime, declare the consequence of a miss: a design change reviewed again before a named task, or a named owner question.
  - If Z-1 is adopted, split the ensure out of T041-5 and T041-36 into a follow-up task that the cold-start regime gates.
* **Recommended disposition: fix**. Reordering costs nothing and stops the SDKs and Studio from being built on a mechanism that has not been measured.

### I-3. Forgetting the kept room after every batch makes the ensure run before nearly every carrying report in a fleet with long steps, which is in practice option 1h, the one rejected - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (A); builds on Z-1
* **Affected Scope:** lines 1259-1262, 1147, 1272-1274, 1947 (Complexity Tracking, the option-1h rejection), 401-404 (Goals), 303-304 and 1740-1745 (the script marker), 3059-3101
* **Discrepancy Description**:
  - Lines 1259-1261 say that after a batch's T8 or the script marker, "the store forgets that backend's room for the tenant and its next carrying report runs the ensure".
  - Ingests and reports share one pool, and each long step sends about one batch a second. With a few dozen long steps in flight on a replica of 20 backends, almost every backend serves a batch between two of its carries, so the next carrying report, short or long, pays the extra ensure transaction.
  - In any fleet with long steps, the built behaviour is therefore option 1h ("an ensure on every carrying report"). Complexity Tracking (1947) and ADR-0097 reject that option for its cost. It also gives up the Goals' "no transaction to a short step" (401) for short steps that share the pool with long ones.
  - Nothing measures it. T041-35 had no ensure, and T041-9 records no ensures-per-carry figure and sets no criterion on it.
  - The marker half of the sentence is wrong. The script marker runs in the control plane's process and pool (303-304, 1740-1745), so the execution engine's store never sees it.
* **Rationale & Recommendation**:
  - If the ensure stays (Z-1), have the store subtract the batch's planned charge from the kept room instead of forgetting it. P5 already sizes every record exactly as `sagat_quota_measure` measures it. An underestimate costs a cheap `room` answer, and an overestimate falls to the designed reservation and fallback.
  - Drop the marker clause.
  - Give T041-9 a criterion on ensures per carrying report, in workload (g) and in a mixed short-and-long workload.
* **Recommended disposition: fix**. As written, the built behaviour turns into the alternative the design rejects for its cost, and the marker clause is factually wrong.

### I-4. The order guard is a server mechanism for a corner that one SDK rule removes, and Complexity Tracking names no rejected simpler alternative - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (A)
* **Affected Scope:** lines 851-852 (T9), 1043-1044 (`in_order`), 1088-1089 and 1097-1098 (`last_stored_xid` in CC and CE), 1657-1660 (the SDK at 2 seconds), 1894 (risk row), 1950 (Complexity Tracking), 2529-2530 (T041-5 guard trigger), 2646-2647 (T041-6), 2791 and 2857-2860 (T041-36), 3172
* **Discrepancy Description**:
  - The guard exists for one case: a carry whose transaction id is below that of a batch committed after it (1894).
  - It costs a stream column, a trigger that refuses a lower value, a plan input, a drop reason, a metric label, a cross-language property-test dimension and tests.
  - For a conforming SDK, only a batch whose fate the SDK does not know can begin after the report's transaction: the batch abandoned when the 2-second wait ends, or one dropped after its `UNAVAILABLE` or `DEADLINE_EXCEEDED` retries. Yet the SDK still carries up to 64 records in exactly that case (1657-1660).
  - If the SDK sends a mark-only carry after any batch of unknown fate (counting the records locally, as it already does for `report_deadline`), no conforming client can produce out-of-order lines.
  - What remains is display order for non-conforming clients. The design already leaves that class of client unprotected for redaction (G041-001).
  - The line-1950 row names no simpler alternative that was rejected, and the constitution's Governance rule requires one.
* **Rationale & Recommendation**:
  - Make that the SDK rule (in section 6 and `sdks/SPEC.md`), and remove `last_stored_xid`, its trigger, `in_order` and `out_of_order`.
  - Otherwise, record the SDK rule in the line-1950 row as the rejected alternative, with the reason.
* **Recommended disposition: fix**. Doing at least the second option is a constitutional requirement, and the first removes a server mechanism together with its test dimension.

### I-5. The busy loss and the held refusal are drops the Requirements never permit, and they are reported through a field named `ceiling` - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (A)
* **Affected Scope:** lines 168-178 (FR-041-005), 179-189 (FR-041-006), 237-242 (FR-041-011), 280-284 (FR-041-017), 1294-1296, 1818-1822
* **Discrepancy Description**:
  - FR-041-006 says the server drops records only by "a ceiling", and FR-041-011 reports "the ceiling met".
  - The revision adds a drop that is not a ceiling: a fallback past *b* makes the carry mark-only with `tenant_quota_busy`, which the owner accepted (OD-2026-11-20-14). The `held` case is likewise reported as `tenant_quota` while room exists.
  - Both are stored in, and shown through, the stream's `ceiling`.
  - The binding part of the document (the Requirements, SC-041-002 and SC-041-003) does not permit a loss that the Technical Plan introduces. The contract field's name and the wording of FR-041-011 mislead readers and operators.
* **Rationale & Recommendation**:
  - State in FR-041-006 that a carry's records may be dropped, counted and shown when the quota cannot be checked within its budget (G041-026) or when room is held by open writers (G041-023).
  - Add the state to FR-041-017.
  - Either rename the contract field to a drop reason (1.7 and the REST contract are unreleased), or say in FR-041-011 that `ceiling` carries non-ceiling causes.
* **Recommended disposition: fix**. The Requirements are the binding layer and must permit a loss the owner accepted. The field can still be renamed because the contract is unreleased.

### I-6. The rule the owner accepted for putting O041-041 again does not match what T041-9 measures - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (B)
* **Affected Scope:** lines 542-544, 1302-1303, 3079, 3082-3101; `docs/runbooks/step-logs.md` lines 40-41
* **Discrepancy Description**:
  - OD-2026-11-20-14 accepted the busy loss on condition that the question is put again "above 0.1% of carries answered `mark_only:busy` in any regime task T041-9 measures" (543-544, repeated in the runbook).
  - T041-9 checks that rate in two regimes only: near the limit (3084-3086) and behind a pooler (3097-3098).
  - The cold start uses a stricter criterion of none (3079).
  - The small-quota, many-low-volume-tenants and fan-out regimes record no busy rate at all.
  - The fan-out regime holds the usage row for 400 ms on purpose while carries fall back (3099-3100). That is twice b = 200 ms, so it produces `mark_only:busy` carries by construction.
  - Read as written, the owner's rule is tripped automatically by a regime built to provoke it, and left unchecked in three others.
* **Rationale & Recommendation**:
  - Say in T041-9 which regimes count for O041-041, and exclude induced-fault regimes (the 400 ms hold) by name.
  - Record the busy rate in the small-quota and low-volume-tenant regimes.
  - Correct lines 543-544 and the runbook to the same scope. If the narrower scope changes what the owner accepted, confirm it with the owner.
* **Recommended disposition: fix**. What the owner was promised has to match what the plan measures, or the condition the owner accepted cannot be evaluated.

### I-7. The G041-027 remedy cancels the scaling runbook's reason for a pooler, so the ensure does nothing where fleets are largest - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (B); builds on Z-1
* **Affected Scope:** lines 1264-1271, 1892, 3094-3098, 3108-3111; `docs/runbooks/scaling.md` line 36
* **Discrepancy Description**:
  - The ensure, the room kept per (backend pid, tenant) and the affinity-miss counter all assume that one client connection stays on one server backend.
  - `docs/runbooks/scaling.md` line 36 tells operators to "put a pooler in front of the server" as replicas × pool size approaches `max_connections`. That is the 8 × 128 regime that justified the leases.
  - T041-25's remedy amends that advice to "the session mode for step-log writers" (3110-3111). A session pooler pins each long-lived sqlx connection to one backend, which removes the multiplexing the pooler was added for.
  - So the deployments that need a pooler can either keep transaction mode, where decision 3b has no effect and every short lease falls back under the run lock (1266), or use session mode, where the pooler no longer solves the connection-count problem.
  - Z-1 noted the dependency on affinity. It did not note that the project's own scaling path leads into it, or that the planned remedy defeats the pooler's purpose.
* **Rationale & Recommendation**:
  - Either state the supported topology as a requirement (the execution engine connects directly, or no pooler multiplexes it), or treat the bounded fallback as the baseline in every topology, as Z-1 recommends.
  - Do not amend `docs/runbooks/scaling.md` to recommend session mode as if it kept the pooler's benefit.
* **Recommended disposition: fix**. The planned remedy contradicts operating guidance already published, and it leaves the largest deployments with a mechanism that has no effect.

### I-8. The owner-gate lines leave out the 2026-11-20 decisions the tasks build - Severity: Major
* **Reviewer Lens:** `egm-zoom-out` (B)
* **Affected Scope:** lines 2071-2073 (the rule), 2443 and 2474 (T041-3), 2746-2747 and 2868 (T041-36), 3103 (T041-9), 3117 (T041-25), 6 (header)
* **Discrepancy Description**: The plan's rule (2071) says a task that builds an owner answer "carries an `Owner-gate:` line citing its row". Four tasks and the header break it:

  | Task | What it builds or cites | Owner-gate line lists | Missing |
  | --- | --- | --- | --- |
  | T041-3 | ADR-0095 accepted (OD-2026-11-20-15), line 2443 | OD-2026-11-15-4, -15-8, -15-9, -15-16, -19-1 (2474) | OD-2026-11-20-15 |
  | T041-36 | the one-call carry, ADR-0095 and ADR-0097, the busy loss as built | OD-2026-11-15-5, -15-6, -16-9, -16-10, -16-11, -19-1 (2868) | OD-2026-11-20-1, -14, -15, -16 |
  | T041-9 | SC-041-007 as settled, the 0.1% rule | OD-2026-11-19-1 (3103) | OD-2026-11-20-13, -14 |
  | T041-25 | the runbook's 1 × 16 cost section | OD-2026-11-13-14, -13-18 (3117) | OD-2026-11-20-13 |

  The header's Owner-acceptance line (6) also omits OD-2026-11-20-1, which set the allowance SC-041-007 uses. These lines are how an implementer learns that a behaviour is fixed by the owner. Without them, a change to the busy trade or to the ensure (for example adopting Z-1) would not be flagged as needing the owner.
* **Rationale & Recommendation**: Add the missing OD rows to each Owner-gate line and to the header.
* **Recommended disposition: fix**. The edit is mechanical, and without it an implementer would not know that the owner's decisions cover the busy trade and the ensure.

### N-1. Tasks would create documents that already exist, and the busy text has two wordings - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out` (A and B)
* **Affected Scope:** lines 1295-1296, 1820-1821, 1851, 2439-2440, 2690-2691, 2696-2697 (T041-10), 3113-3114 (T041-25); `docs/runbooks/step-logs.md` line 37; `docs/releases/unreleased.md` lines 136-143
* **Discrepancy Description**:
  - T041-10 "creates" `docs/runbooks/step-logs.md`, which T041-35 already wrote (its evidence is at 2439-2440). The runbook names T041-10 and T041-25 as the tasks that complete it and leaves out T041-30.
  - The runbook's busy text ("not stored: the log service was busy (not your quota)") differs from the design's wording ("... these lines were not checked against your quota"), and asserts something the system did not check.
  - T041-25 requires the release note's first line to say that capture is on by default. The existing section opens with the measured cost instead, and it announces a feature whose release T041-9 can still stop.
* **Rationale & Recommendation**:
  - Use one busy string, the design's.
  - Make T041-10 "modify" the runbook.
  - Restate T041-25's rule against the existing release-note section, and mark that section as conditional on T041-9.

### N-2. The Estimate was not revised for section 2b - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out` (A and B)
* **Affected Scope:** lines 3202-3214
* **Discrepancy Description**:
  - The Estimate keeps the 2026-11-19 server size and still names T041-35, which is settled, as the likeliest mover.
  - It counts none of: the lease table, about twelve definer functions, the plpgsql carry function, and T041-5's test list.
* **Rationale & Recommendation**: Re-estimate T041-5 and T041-36, and name T041-9's regimes as the risk.

### N-3. The ensure policy sits in the adapter, and one interval lives in three untied places - Severity: Nit
* **Reviewer Lens:** `egm-zoom-out` (A)
* **Affected Scope:** lines 1244, 1255-1258, 1337-1340
* **Discrepancy Description**:
  - Line 1244 says "the adapter holds no rule", yet the store decides short, unknown and stale with a 9-minute rule.
  - That 9 minutes depends on the 10-minute idle interval fixed in a SQL function, which depends in turn on sqlx's default pool idle timeout. No test ties the three.
* **Rationale & Recommendation**:
  - If the ensure stays (Z-1), make the decision a domain or application function.
  - Add one fitness test that the store's staleness is below both the function's interval and the pool's idle timeout.

Counts: Blocker 0, Major 8, Nit 3
