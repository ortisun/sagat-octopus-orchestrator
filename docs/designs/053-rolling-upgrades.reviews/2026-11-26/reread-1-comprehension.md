# Targeted re-read 1 of design 053: egm-comprehension (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: c0536d9f160d5ac33806ca97b9a16e3473950661
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`comprehension_report.md`, not a full review; the reviewer read the design and ADR-0131 only.

Full round: Majors 1 to 12 and 14 resolved (4 to 6 on design 053's side; designs 048 and 049 carry theirs in Tasks
048-19 and 049-20); Major 13 partly; Nits 1 to 4, 7, 8, 10 to 12 resolved, 5 mostly, 6, 9 and 13 partly (the ADR's title
still names contracts, kept: renaming the record would break its links, and its decision 1 says contracts wait).

## Major

1. `hot-lock` refuses the several locks `capability-list-widening` takes, and its one-table header and the retry's
   "table of the last statement" do not fit three tables.
2. The partitioned-index procedure's attaches, retried per partition over 5 rounds, are outside the stated 171 s bound,
   and the deadline's reach over the procedure is unsaid.
3. The scenario "processed by N-1 after its removal" cannot be built for a capability that cannot be removed; "drains
   and undrains the others" reads two ways, and whether removed capabilities are activated again is unsaid.

## Nit

1. "No statement timeout" for a migration without a transaction against the deadline's statement timeout.
2. Whether the release-branch run before 0.3.0 blocks the tag.
3. The ADR's test name with "four functions", unexplained now.
4. `missing_for`, `requirement_recorded` and `census_step` unexplained.
5. "The binary's own registered list" for a command that never registers.
6. `instance_evict` has no named outcomes.
7. Owner decision 2026-11-24-521 looks like a typo (it is not: the row exists in `docs/owner-decisions.md`).

Counts: Blocker 0, Major 3, Nit 7

Every finding but Nit 7 was fixed at the next commit; ADR-0131 decision 4's "503" became `UNAVAILABLE` there too.
