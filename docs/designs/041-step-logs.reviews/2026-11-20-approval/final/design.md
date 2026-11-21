**Final confirmation read of design 041 at 54341713: stages 5 to 8 (design, zoom-out, comprehension, structure and style)**

Of the third round's items, Majors 1, 2 and 3 and Nits 1 to 6 are resolved. Z-3 is closed by the owner. D-12 is still kept, and I recommend "accept". The fixes add two new Majors and no Blocker, plus seven Nits.

I read the design's newest revision note and Overview, SC-041-007, the Admission paragraph, L4, section 2b, the Risks, T041-9 and T041-37. I also read `docs/runbooks/step-logs.md`, ADR-0097's header, `resolution.md` "Third round", D041-136 and D041-137, OD-2026-11-21-1 to -3, the eight-round table in `specs/041-step-logs/validation-report.md`, `docs/adr/README.md` and `docs/runbooks/scaling.md`. Every file could be read. No separate standard was named, so my third-round report was the review context.

## The items from the third round

- **Major 1 (the 8 × 128 judging rule): resolved.** SC-041-007 (lines 430–435), T041-9 (lines 3443–3448) and D041-136 set gate (2b) at 8 × 128 to at most +112.9 ms, under OD-2026-11-21-1. The new false-stop formula C(m, r) / C(8 + m, r) is right, and I checked the rates for m = 7 to 12. New Major 1 below covers a related problem the fix introduced.
- **Major 2 (L4 and the Overview): resolved.** L4 (lines 1021–1026) and the Overview (lines 27–29) now match D041-134. Nit 1 lists two places that are still stale.
- **Major 3 (*Q* behind a pooler): resolved.** *Q* is now passed per transaction with `set_config(…, true)` inside statements the store already sends (section 2b, lines 1404–1411; T041-5; data-model §3b and §8a). T041-37 checks through the pooler that a transaction sees *Q* = 5 and the next one sees 1. The sizing rule that came with this fix is new Major 2.
- **Nit 1 (rate per gate and per release): resolved.** Lines 445–457 give both. Nit 7 covers a residual.
- **Nit 2 (too few valid rounds): resolved.** Lines 426–429 give the rule.
- **Nit 3 (what the owner report leads to): resolved.** The report does not stop the release, and the figures are restated. New Major 1 covers when the report fires.
- **Nit 4 (revision note): resolved.** Lines 33–41 now cover the second and third rounds. Nit 2 covers what the note's list leaves out.
- **Nit 5: resolved.** Lines 1414–1418 now say "never held by step-log waits or Logs calls".
- **Nit 6: resolved.** Line 3434 now says "the tenant's ten tickets".
- **Z-3: closed by the owner** ("Keep it", OD-2026-11-21-2). It is no longer mine to recommend on and is not counted.
- **D-12: still kept. I recommend "accept".** There are now five revision notes at lines 33–100, as the amendment rule asks. This is counted as an open Major below.

## Open findings

**1. Major (fix). The owner report compares the median of all valid rounds against the largest of the three rounds used, so the accepted build would trigger it about half the time.**
- *Where:* SC-041-007 lines 461–464; T041-9 lines 3449–3452; runbook section 1; D041-136.
- *Problem:* the statistic is now the median over all valid rounds. The threshold is still the largest of the three rounds used (+36.9% and +9.5%). This is the same mismatch as the third round's Major 1, now in the report rule.
- *8 × 128:* the eight valid rounds' (2b) median is +2.06 ms. In p99 terms that is about 10% + 2.06 / 481.5 ≈ +10.4%, above +9.5%. So the build the owner accepted already exceeds the threshold on its own record.
- *1 × 16:* the median is (8.62 + 9.68) / 2 = +9.15 ms, about +36.4%. The threshold (+9.68 ms, round 2) is the 5th-smallest of the eight valid rounds, so new runs exceed it often.
- *Effect:* the owner is told the accepted cost was exceeded when it was not, and the runbook and release-note figures are restated from noise.
- *Fix:* derive the threshold from the eight valid rounds, the same way the floors are derived (for example from their median p99 percent plus the derived tolerance), and state its false-report rate. Alternatively, report on the statistic the threshold came from.

**2. Major (fix). The pooler sizing rule leaves little room for multiplexing, contradicts the scaling runbook, and changes what the G041-027 regime measures.**
- *Where:* section 2b lines 1422–1425; Risks lines 2125 and 2127; ADR-0097 decision 4; runbook section 3; T041-37 lines 3129–3130 and 3170–3172; `docs/runbooks/scaling.md` line 36 (unchanged).
- *(a) The rule defeats the pooler's purpose.* It needs about 0.75 N + 1 server backends per replica, so the pooler can save at most about 25%. `scaling.md` advises a pooler exactly when replicas × N would pass `max_connections`. For example, 16 replicas of 20 need 256 backends. A deployment can only follow both documents while the pooler saves little.
- *(b) The pooler reserve is far smaller than the direct reserve, with no reason given.* The rule reserves one backend per replica, against 5 of 20 per replica with direct connections. That one backend is all that is guaranteed for heartbeats, task leases and reports that do not wait. Other processes behind the same pooler are not counted at all.
- *(c) The G041-027 regime now measures an easier case.* T041-37 moved from 40 server backends (4:1) to 128. The option-1c criterion behind the accepted gap G041-027 (OD-2026-11-20-15) is no longer measured at heavy multiplexing. Yet the Risks row and the runbook still say `scaling.md` "stands".
- *(d) Nothing enforces the new rule.* ADR-0097's new MUST names no check, which README rule 3 requires.
- *Fix:* either bound the Logs cap and *Q* behind a transaction pooler by the replica's share of the server pool, or state the multiplexing ceiling in `scaling.md` and the G041-027 row and put it to the owner. Either way, keep a heavily multiplexed pooler row in T041-37 for option 1c and for heartbeat queueing, and give a reason for R.

**3. Major (accept). D-12, the revision notes in the Overview.** This is unchanged and is what the amendment rule requires.

**4. Nit (fix). Text left stale by D041-137.**
- Design line 1590 still lists `"held"` as a top-up outcome, against line 1492, data-model §7 and ADR-0097 decision 8.
- data-model.md line 705 still says the fallback tries the usage row "`NOWAIT`, then a wait slot", where it should say `SKIP LOCKED` and two slots.

**5. Nit (fix). The revision note's "Changed" list (lines 40–41) is incomplete.** It omits section 2's Admission paragraph (lines 877–895, the pool defaults) and tasks T041-6 and T041-7, all of which changed. Re-reviewers scope their reads by this list.

**6. Nit (fix). Owner decisions missing from the design's records.** The header's Owner-acceptance line (line 6) does not list OD-2026-11-21-1 to -3. T041-9's Owner-gate (line 3454) does not list OD-2026-11-21-1, the decision that set its 8 × 128 bar.

**7. Nit (fix). "B's `maxwait`" measures every client, not tenant B (line 3185).** `SHOW POOLS` reports `maxwait` per (database, user) pool, and the benchmark uses one role. Either name it as the pool's figure or measure B's acquire wait in the driver.

**8. Nit (fix). The Admission paragraph's isolation claim holds only at a pool of 20 (line 895).** "A second tenant keeps 3 of the cap" is false at the binary's default of 10, where the cap and the tickets are both 5. The design does not state what data-model §6 and the runbook now state about pools below 16. Either state it in the design and the Risks, or default the tickets to min(7, cap − 1).

**9. Nit (fix). ADR-0097's header cites the wrong rules.** It says the delegate amendments take effect "under rules 2 and 4". Rule 2 says the opposite: an accepted ADR is not edited in substance, and a change becomes a new superseding ADR. The Status line still reads plain "Accepted" while some body text waits for the approval. Cite the actual authority (OD-2026-11-20-17) and mark the amendments as pending until then.

**10. Nit (fix). The owner's record does not show the release-level false-stop rate.** OD-2026-11-21-1 cites only the 3.85% per gate. The rate for the release as a whole can reach 34.1% at eight valid rounds (union bound), and the two 8 × 128 rate gates alone contribute about 15%. Put the release-level figure in front of the owner at approval, in the approval block or the resolution.

Counts: Blocker 0, Major 3, Nit 7