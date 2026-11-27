# egm-review for the approval of design 051 (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: c91af7ac2be73ff9eb4a2163d5d124a15f9bbac8
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary. The approval follows Owner decision 2026-11-25-2: one full round by every required
reviewer, then the fixes re-read, then the remaining findings settled in writing. The full round of the EGM lenses ran
on the free-runner draft at b604f497 (`zoom_out_report.md`, `critic_report.md`) and on the revised text
(`comprehension_report.md`, `implementation_readiness_report.md`), with the repository's architecture, security and
correctness reviewers (`*-full.md`). Every Blocker of every lens was fixed and every lens re-read the fixes
(`reread-1-*.md`). The security and correctness re-reads that followed changed the method most: durability is enforced
inside the database server by a first-party extension instead of being detected. egm-implementation-readiness
(`reread-2-implementation-readiness.md`) and egm-comprehension (below) confirmed at this commit, each with every finding
of its re-read resolved and one new Major, below. The Nits below are the minors left open at this commit.

## Major (approval confirmation, settled)

1. egm-implementation-readiness, `reread-2-implementation-readiness.md`, Major 1: the acknowledgement carries nothing
   back to the driver and the host cannot tell when the product is idle. Settled: the driver posts rung N+1's
   boundaries only after rung N's drain and wait for open runs, at least 30 s before the planned warm-up; the
   acknowledgement has a fixed grammar (rung index, calibration deviation, steal, and one verdict of continue, re-run,
   rung failed or turn failed); the host computes the verdict from its checks and the driver's ladder obeys it; tests
   of both ends in Tasks 051-2 and 051-6 (follow-up Task 051-14 (a)).
2. egm-comprehension, approval confirmation at this commit, Major 1: "the driver" names two processes and the `host`
   subcommand has no place in the sets. Settled: the Terms section defines the driver's container half (load, records,
   recorded ends) and its host half (`host`: the channel, the probe, the snapshots, the calibration, the faults, the
   upload copy); the host half runs pinned to the driver's thread, listed in the sets table; the requirements name the
   half that does each job (follow-up Task 051-14 (b)).

## Nit (settled)

3. egm-comprehension, approval confirmation, Nits 1 to 9: "*At every transaction commit.*" as the heading; fault (c),
   the host-only directory and the host's probe defined before use; "no two stages at once"; the pilot sentence's verb,
   the 6 pilot jobs and their dispatch; the report's common-rate columns for the chain of three and the fan-out only and
   the common-rate job measuring that rung alone; the Risks row counting worker-bound results; "every image pinned or
   built at the commit and matched by image ID"; how a statement is assigned to a table; the `xmin` parenthetical split
   in two.
4. egm-implementation-readiness, `reread-2-implementation-readiness.md`, Nits 1 to 7: the extension, its image and the
   `postgres/` service moved before Task 051-3; the workflow's requirement tied to the workflow file and Task 051-7's
   workflow steps moved to Task 051-8; the probe through `docker exec` on the socket, the harness role's privileges,
   the count function and where `CREATE EXTENSION` runs; the subcommands' arguments; this platform's compose files in
   Task 051-3; Task 051-11 as Requirement 051-017 says; one host-only directory per purpose, the snapshots out of the
   driver's reach.
5. The Nits of the repository reviewers' approval confirmations at this commit (`approval-correctness.md` Nits 1 to 5,
   `approval-architecture.md` Nits 1 to 5) and the security approval's suspicion (full image IDs).

Counts: Blocker 0, Major 2, Nit 3

Findings 1 to 5, and `approval-architecture.md`'s two Majors, are recorded as design 051's follow-up Task 051-14, added
after the approval, each built test first. None is a security finding: every finding of every security report of this
approval run is fixed. Owner decisions 2026-11-24-530 to 2026-11-24-533 record the settlements.
