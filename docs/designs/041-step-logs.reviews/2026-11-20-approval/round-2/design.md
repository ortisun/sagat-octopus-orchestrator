Design 041: short re-read of the first-round critic findings

I read the design up to "## Detailed Implementation Plan", the critic report, `resolution.md`, OD-2026-11-20-17 to -19, D041-124 to D041-133, the fourth run's record, ADR-0097 decision 3b and ADR-0095 decision 4. I read the working tree; I had no git access, so I couldn't confirm HEAD is `4d6fe4cc`. Line numbers are those of `docs/designs/041-step-logs.md` unless a file is named.

Overall: the Blocker is resolved. Four Majors are still open, and the fixes brought in three new ones.

## Per finding

- **D-1 (Blocker): resolved as a Blocker.** There is now a pass/fail rule at each size, fixed before T041-9 runs.
  - SC-041-007 (397-431) defines the workloads (400-403), the gates and the allowance (405-408), and the environment (408-410).
  - It gives the p99 cost in percent (423-426) and the accepted deviation (427-428).
  - The Goals (450-452), the settled Open Question (482-493), the risk rows (2003, 2006, 2010), NFR-041-PERF (2168-2172) and T041-9 (3182-3185) all cite it.
  - Three problems with the new floor remain; they are reported as N-1, N-2 and Nit n-1 below.
- **SC-041-007 figures against `specs/041-step-logs/validation-report.md` (717-802):**
  - These match:
    - the valid rounds (1, 2, 3, 4, 6, 7, 9, 11; void 5, 8, 10, 12) and the rounds used (1 to 3);
    - the 1 × 16 medians: 82.0%, +8.6 ms, 87.1%, 7 tickets;
    - the 8 × 128 medians: 93.1%, −17.0 ms, 91.5%, 5 tickets;
    - the p99 at +31.9 / +36.9 / +35.2%, and 34.6 against 46.2 ms;
    - round 2's 80.5%, which agrees with 80.46%;
    - the capacity arithmetic: 6.44 and 4.22 tickets.
  - These are **not in the validation report**, only in `specs/041-step-logs/research.md` D041-126 and D041-127:
    - +29.03 ms in round 11 (2b);
    - 83.52% in round 11 (3);
    - 10 tickets in rounds 4 and 6.
  - The run record gives per-round values for rounds 1 to 3 only.
- **D-2: partly resolved.** The defaults are set (848-850, 1045-1054; `data-model.md` §6, 446-452). One stale sentence is left: 835-836 still says "default a quarter of `SAGAT_DB_MAX_CONNECTIONS` … 5 of the … default pool of 20". It is now a Nit, because T041-7 (2662-2664) and `data-model.md` both say half.
- **D-3: resolved** (risk row 2003). Its last clause overclaims; see N-1.
- **D-4: partly resolved.** The Open list now says no question is open (501), O041-026 moved to Answered (517), and the risk rows (2022, 2045) and principle V (2056) are updated. What is left is a Major:
  - Section 5, "Contract" (1701-1703), still says `tenant_step_log_bytes` "shows no `used` to a caller without `run:logs:read`", citing ADR-0085 decision 6.
  - That contradicts OD-2026-11-20-19 (`run:logs:read` and the PII reveal grant in every namespace), section 8 (1939-1941) and T041-4 (2393).
  - It also leaves out `usageWithheld`.
  - Section 5 is missing from the new revision note's "Changed:" list (37-41).
  - **Fix:** restate 1701-1703 by D041-133.
- **D-5: resolved (moot).** No transaction runs before a report (1008-1009). The fallback runs after the fence checks, with the statement timeout as its hard bound (1015-1024; FR-041-005, 188-191). ADR-0095 decision 4's Note (lines 90-94 of that ADR) and ADR-0097 decision 3b (120-142) agree.
- **D-6: resolved.** The measured figures replace "rare" at 1020-1022, 1362-1365 and 2013, and they match the report (0.31%, 2.0%, 726.7 ms, 3.6 × *b*). T041-37 has run-row criteria (2928-2929, 2949-2951).
- **D-7: resolved** (446-452, 490-491, 2007).
- **D-8: resolved.** The 2b citations are listed at 46-48, and a new note is at 32-41. I could not check the note's list without a diff; it does omit section 5 (see D-4).
- **D-9: resolved** (1265-1272). A Nit is left: the prototype's 26-statement breakdown (1273-1278) is still one sentence with nested parentheses, not a list, although `resolution.md` Z-6 says it is a list.
- **D-10: resolved** (Terms 706-719; Overview 24-30; "ensure" and "kept room" are gone).
- **D-11: resolved.** "What a report can wait for" is a numbered list (1005-1031), and FR-041-005 (186-191) and the Overview point to it.
- **D-12 (accepted): I still hold it, as an open Major.** The Overview now opens with five revision notes and a scope paragraph (32-89). One spike-history sentence is left at 1326-1327 ("A shared pool of 256 quota leases, this section's first form, collided"), counted as a Nit.

**Nits:**

| Nit | Status | Where | What is left |
| --- | --- | --- | --- |
| 1 | resolved | 5 | |
| 2 | partly | 94-98 | "both accepted" now follows a list of four ADRs |
| 3 | resolved | 62-63, 75-76, 1237-1238 | |
| 4 | resolved | 86-89 | |
| 5 | resolved | 2010, 2016 | |
| 6 | resolved | 586-591 | |
| 7 | moot | | the ensure is gone |
| 8 | resolved | 360 | |
| 9 | resolved | 98-100, 236-237, 1534-1536 | |
| 10 | moot | | |
| 11 | not resolved | 505, 520-614 (O041-036 at 609), 615-616 | 505 says "newest first", but the Answered list is still out of date order, and the double blank line is still there |
| 12 | resolved as far as legible | 745-775 | |
| 13 | resolved | 1311 | |
| 14 | partly | 1064 | "well under" remains; `resolution.md` does not claim this one |
| 15, 16, 17, 18, 19, 20, 21 | resolved | 1213-1215, 2011; 349-357; 2057; 1546, 2174; 184-194 | |

## New findings from the fixes

**N-1. The 1 × 16 floor admits about three times the accepted p99 cost, and the run record does not hold its source.**
- **Severity:** Major. **Disposition:** fix.
- **Affected scope:** 412-422, 2003; D041-126; `specs/041-step-logs/validation-report.md` 753-763.
- **Finding:**
  - Gate (2b)'s floor of +29.1 ms comes from one outlier valid round, but it is compared with a median of three rounds.
  - It admits a completion p99 of about 34.6 + 3.46 + 29.1 ≈ 67 ms. That is about +94% over off-1K, against the +31.9% to +36.9% that OD-2026-11-20-13 accepted.
  - So risk row 2003 ("catches a regression beyond that cost") is not true for (2b).
  - The worst-round values the floor is built on are not in the fourth run's record.
- **Recommendation:**
  - Record the eight valid rounds' gate values in `specs/041-step-logs/validation-report.md`.
  - State, beside the floor, the p99 in percent that it admits.
  - Either take the tolerance from the spread of three-round medians, or put the widened (2b) floor to the owner.
  - Correct risk row 2003.

**N-2. Gate (4) at 1 × 16 is judged on a ticket setting that no run measured.**
- **Severity:** Major. **Disposition:** fix.
- **Affected scope:** 406-408, 419-422, 1045-1050, 3169-3170, 3183-3184.
- **Finding:**
  - The fourth run had 10 tickets (1045), and two of its valid rounds needed 10 (1049).
  - T041-9 runs with T041-7's default of 7 (3169-3170), yet still requires refusals under 1%.
  - So "meets this floor in every one of its valid rounds" (421-422) does not cover gate (4).
  - T041-9 could stop the release on the very default that D-2's fix set, which repeats the D-1 failure mode.
- **Recommendation:** state in SC-041-007 which ticket setting gate (4) is judged at. Either use 10, as measured, and record the refusals at the default without judging them, or set a refusal criterion derived for 7.

**N-3. The wait slots bound one tenant, not the replica's pool.**
- **Severity:** Major. **Disposition:** fix.
- **Affected scope:** 1346-1354, 2014; T041-37's two-tenant regime, 2943-2948.
- **Finding:**
  - Each tenant has *P* = 7 wait slots per replica, and the default pool is 20.
  - Three tenants waiting on their own usage rows at once can therefore hold 21 connections, so "another tenant's reports and heartbeats keep the rest" holds for at most two tenants.
  - Top-ups (1388) hold their connection after the report and use the same slots, so they add to this.
  - T041-37 tests only two tenants.
- **Recommendation:** either add a replica-wide cap on waiting slots, kept below the pool less a reserve, or state the bound for three or more tenants as a residual in the risk row and add a regime with three or more waiting tenants.

**n-1. "Rounded against the build" says the opposite of what the figures do.**
- **Severity:** Nit.
- **Affected scope:** 415; D041-126.
- **Finding:** each tolerance was rounded up (1.53 → 1.6, 20.41 → 20.5, 3.57 → 3.6). That puts every floor on the build's side of the worst round: 80.4 < 80.46, 29.1 > 29.03, 83.5 < 83.52.
- **Recommendation:** say "rounded in the build's favour", or round the other way.

Counts: Blocker 0, Major 5, Nit 7