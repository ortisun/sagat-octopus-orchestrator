# Targeted re-read 1 of design 051: egm-zoom-out (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 7c7bdec4ba5ccb0d9142b526c8666cb1a5ca295a
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of
`zoom_out_report.md` (at b604f497), not a full review. The additions other reviewers asked for are each the smallest form
of their need.

Full round: all six Majors and six Nits resolved.

No Blocker, no Major.

## Nit

1. Common-rate jobs for all eight variants (40 jobs), though only the chain of three and the fan-out are ranked at the
   common rate. Cut: those two only (10 jobs), the others at half each maximum.
2. Steal or a calibration loop by the pilot's verdict is a branch; always run both.
3. Tasks 051-12 and 051-13 cite Requirement 051-003, which describes the free runner.

Counts: Blocker 0, Major 0, Nit 3

Nits 1 and 2 were applied at the next commit (a campaign at 18,745 runner minutes and about 39.8 hours); Nit 3 stays,
since a task's references name requirements and 051-003 is the machine requirement the later tiers change.
