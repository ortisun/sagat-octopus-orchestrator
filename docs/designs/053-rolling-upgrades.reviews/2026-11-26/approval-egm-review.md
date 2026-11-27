# egm-review for the approval of design 053 (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 3cf5ee9737c4611e9126403c7f5dbfecc0b4f861
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary. The approval follows Owner decision 2026-11-25-2: one full round by every required reviewer,
then the fixes re-read, then the remaining findings settled in writing. The full round of the EGM lenses ran on the first
draft at 212e4262 (`zoom_out_report.md`, `critic_report.md`), whose findings reshaped the design (no version gate, no
permanent flag, contracts deferred), and on the revised text at 7771688e (`comprehension_report.md`,
`implementation_readiness_report.md`) with the repository's architecture, security and correctness reviewers
(`*-full.md`). Every Blocker of every lens was fixed. The fixes were re-read by every lens (`reread-1-*.md`, and
`reread-2-implementation-readiness.md`, which is egm-implementation-readiness's confirmation at this commit with no
Blocker or Major); egm-comprehension confirmed at this commit with every Major of its reports resolved and no Blocker or
Major. The Major below is the one a lens's re-read raised that the design answers by keeping its choice; the Nits below
are the minors left open at this commit.

## Major (settled)

1. egm-zoom-out, `reread-1-zoom-out.md`, Major 3: the eviction record adds no safety beyond the runbook's stop-first
   step. Settled by keeping it: it answers security re-read 1's Major (a process whose stop step failed or was skipped
   keeps registering under its identifier and holds phase 2 again), for one owner-only table and one read at
   registration.

## Nit (settled)

2. egm-zoom-out, `reread-1-zoom-out.md`, Nit 1: Requirement 053-004 carries the retry arithmetic and the SQLSTATE list;
   kept there, where the correctness and comprehension reviews asked for the bounds.
3. egm-comprehension, `reread-1-comprehension.md`, Nit 13 (partly resolved): ADR-0131's title still names contracts;
   kept, since renaming the record would break its links and its decision 1 says contracts wait.
4. egm-implementation-readiness, approval confirmation at this commit, Nits 1 and 2 (Task 053-3's wording of
   `connect_command`; the fixture capability's token, widening and descriptor).
5. egm-comprehension, approval confirmation at this commit, Nits 1 to 4 (who processes a non-removable capability's
   state after phase 1; how `develop` gets the online rules' start; "per attempt" in ADR-0131's Consequences; the table
   a migration without a blocking lock names when it times out).
6. The Nits of the repository reviewers' approval confirmations at this commit (`approval-correctness.md` Nits 1 to 3,
   `approval-architecture.md` Nits 1 to 3).

Counts: Blocker 0, Major 1, Nit 5

Findings 2 to 6, and `approval-correctness.md`'s Major, are recorded as design 053's follow-up Task 053-7, added after
the approval, each built test first; ADR-0131's and ADR-0132's texts named by them are corrected in the approval commit;
finding 1 stays as the design chose. None is a security finding: every finding of every security report of this
approval run is fixed. Owner decisions 2026-11-24-522 to 2026-11-24-524 record the settlements.
