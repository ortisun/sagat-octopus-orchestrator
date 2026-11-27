# Targeted re-read 1 of design 051: egm-implementation-readiness (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 16f18080faeb686708bbbe572a116c1f035ba169
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`implementation_readiness_report.md`, not a full review.

Full round: the Blocker, Majors 1 to 3 and 5 to 10 and all fourteen Nits resolved; Major 4 partly (Nit 1 below).

## Major

1. The fault injector cannot run in the driver's container (no engine socket, no host process namespace), and a daemon
   restart between turns stops the driver. Fix: the injector on the host, writing fault times into the record
   directory; `run.sh` starts the driver container once per turn in the order `driver plan` prints.
2. Nothing hands the common rate, or the halved crash rate, from one stage to the next. Fix: each later job's first step
   downloads the earlier stages' summaries and runs `driver report --common-rate`; crash jobs read the confirmation's
   verdict.

## Nit

1. The image build's time and archive in the history job's minutes and the disk budget; each pilot builds its images.
2. The subcommands' arguments.
3. `--only competitive` is a new flag; say so.
4. "Afterwards its absence fails" names no trigger.
5. A `reconcile` marker in the execution records.
6. Fault (c)'s replayed WAL has no method.

Counts: Blocker 0, Major 2, Nit 6

All eight were fixed at the next commit (the history job at 230 minutes, a campaign at 20,627 runner minutes and about
42.3 hours).
