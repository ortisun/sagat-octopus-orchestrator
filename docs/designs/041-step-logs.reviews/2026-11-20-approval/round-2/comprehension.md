**Re-read of design 041 at 4d6fe4cc: comprehension findings K-1 to K-19**

The review is complete. I read the persona, the first-round report, `resolution.md`, and the sections you named in `docs/designs/041-step-logs.md`: the Overview, the FRs, SC-041-001, SC-041-007, the Goals, the Open Questions, the owner decisions, the Terms, the diagram, sections 2 to 2b and the Verification Plan. For K-3 I also read tasks T041-35, T041-9 and the section 1 defaults. No document standard was supplied, so none applies. I read the working tree of the worktree. Its branch `feature/041-step-logs-server` points at 4d6fe4cc, but without a shell I could not confirm the tree has no uncommitted changes.

### What this proposes
SDKs send an activity's log lines to the platform. A long step streams earlier lines through `AppendStepLogs`. Every report carries the attempt's last lines and final mark in one call of `sagat_step_log_carry`. That call only tries its step-log locks and answers `contended` when one is held. Each charge goes to the writer's own quota lease, a row per server backend. Only when that quota lease is short does the carry run a fallback refill: it waits for the tenant's usage row for at most 200 ms per lock acquisition, and only while holding one of seven wait slots. A quota refusal or a busy row becomes a mark-only final mark inside the same call. Only a failed call re-runs the report. After a carrying report commits with `top_up` true, the store tops the quota lease up on the same connection, off the run lock. No transaction runs before a report.

### How the existing system works, per this document
Worker logs stay on the worker. Only script steps have a stored log. Reports lock the run row, and `work_budget.rs` holds leases and reports to exact counts. Retained bytes are charged through the tenant's single usage row, which is locked at commit.

### Per finding

**K-1: partly resolved.**
- Terms now define *task lease*, *quota lease*, *lease reclaim* and *quota reclaim* (706-710). The FRs, SCs, section 2 "What a report can wait for", 2a, 2b and the Verification Plan use the qualified forms.
- Left: a few bare uses where a pointer or "writer's own" makes the meaning clear:
  - section 2, T8: "charges the rows to a lease", "leases that open transactions hold" (940, 942)
  - L3b: "a reclaim takes the others" (974)
  - the charge paragraph: "its writer's own lease", "open transactions' leases" (1196, 1199)
  - risk row: "charging the writer's lease" (2008)
- Now a Nit.

**K-2: resolved.** The Overview defines quota lease, writer, fallback refill and top-up where they first appear (26-30). Terms add the refill vocabulary, *b*, the wait slot and the step-log lock (711-719).

**K-3: resolved.**
- SC-041-007 now states one criterion per size, with floors and how they were derived (397-431). T041-9 restates the same numbers (3182-3185).
- The defaults are set: tenant tickets 7, cap 10 of 20 (848-851, 1045-1051). T041-9 builds with T041-7's defaults (3169).
- T041-35's empty box is explained: rule 9 of `scripts/check-design-docs.py`, and the approval pull request ticks it (2305-2307).

**K-4: resolved.** SC-041-001 (371-374), the Verification Plan row (3257) and sections 2a (1211) and 2b (1466-1469) now agree. A quota refusal or a busy row is answered in the same call. Only a failed call re-runs.

**K-5: resolved.** The Goals (446-452) and the risk row (2007) now say: no transaction before or in a short step's report, and one statement per carry. They also state the top-up, about once per 300 ten-line carries, off the run lock. "Six for a long step" and "five for a short step" are gone.

**K-6: resolved.** The row (3258) now says the fallback waits at most *b* and then answers `service_busy`, or answers at once with no wait slot free.

**K-7: partly resolved. A Major remains.**
- The charge paragraph is scoped correctly: "only CA, and CC′ when the existing stream's row is held, answer it" (1206-1207).
- Left: the revised heading paragraph of the same section still lists the carry's own quota lease among the locks whose holder makes the carry `contended` (1153-1155):

  > Every step-log lock the function takes is tried, never waited for (its fences, its stream lock and row, its own quota lease `SKIP LOCKED`); a lock another transaction holds makes it write nothing (`contended`)

- That contradicts three other places:
  - section 2 item 3: a quota lease held "only by a quota reclaim" leads to the fallback refill (1015-1016)
  - section 2b: a held quota lease makes the charge call the refill (1331-1332)
  - line 1206, which says no other case answers `contended`
- An implementer writing the held-quota-lease test would get two expectations. Disposition: **fix**. One edit drops "its own quota lease" from that parenthesis, or says a held quota lease leads to the fallback.

**K-8: resolved.** The third measure is now a list (1265-1278). The 26 is labelled as the prototype's figure, and its parts add up to 26. The built figure is to be derived again by T041-36, and section 2b says the same (1499-1501).

**K-9: resolved.** The diagram (757-768) shows:
- the stream lock the ingest waits for
- the fallback's bounded wait with a wait slot
- the mark-only answers and the re-run on a failed call
- the top-up after the commit

**K-10: resolved.** "Three more ingests of T, each on a backend of its own" (360-361). No "K − 1" remains.

**K-11: partly resolved.** Terms now note that *R* and *S* have different meanings in section 2b (732-733). *L* (the limit against lock levels L0-L4) and *T* (a tenant against steps T1-T10, as in "tenant T is held after T8", 360) are still reused. Nit.

**K-12: resolved.** *Step-log lock* is defined as L1, L1b, L2, L3 and L3b, and L4 is excluded (718-719).

**K-13: resolved.** The Date is 2026-11-20 (5). The ADRs read "then Proposed, accepted on 2026-11-20" (63, 75-76). "Superseded by ADR-0095 decision 2" is unconditional (1238). The other stale phrases are gone.

**K-14: resolved.** The Open subsection says no question is open (501), and O041-026 is under Answered (517).

**K-15: resolved.** The gap list covers G041-001 to G041-027 with their statuses (3287-3306).

**K-16: resolved.** The risk row says "at most four transactions in all" (2011).

**K-17: not resolved.** It still does not say which section 10 is "the one on failure isolation": the first 10, or the second one after it becomes 12 (1710-1711). `resolution.md` does not list K-17. Nit, outside the changed text.

**K-18: resolved.** The sentence now reads "plus the batches for the records that do not fit its carry" (1294-1295).

**K-19: resolved.** The function is named `sagat_quota_step_log_insert_trigger` (1548-1549).

### New findings from the fixes
- **No new Blocker or Major.** The one remaining Major is K-7's leftover above (disposition: fix).
- **New Nit:** "delays the commit and never makes the report late" (190-191, 1024) uses "late" without saying what it means. A reader has to infer it from G041-021 ("refused as late", 3298): the fence has already passed, so a task lease that expires during the wait cannot refuse the report.

Counts: Blocker 0, Major 1, Nit 4