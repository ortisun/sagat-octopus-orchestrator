# egm-review for the approval of design 048 (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 8bcdf393ed0dd38c3b07b816f7538969a99343b5
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary. The approval follows Owner decision 2026-11-25-2: one full round by every required reviewer,
then the fixes re-read, then the remaining findings settled in writing. The full round of the EGM lenses ran on
1e283510 (`zoom_out_report.md`, `comprehension_report.md`, `critic_report.md`, `implementation_readiness_report.md`
beside this file), with the repository's architecture, security and correctness reviewers (`*-full.md`). Every Blocker
of every lens was fixed, and every Major of the full round was fixed (zoom-out's eight by removing the stale-tolerant
read, the data-directory copy, the detached connections and the beacon table, by reusing `dr_acked_workload` and
`dr_verify`, by `WriteTx`, by the outcome rule and by a free kind smoke; the critic's eight by the identity function, the
tickets, the outcome rule, the bounded reads, the amendments of feature 032 recorded in ADR-0130, CUJ 2, the lease sizing
and the reference's status). The fixes were re-read by egm-comprehension (`reread-1-comprehension.md`) and
egm-implementation-readiness (`reread-1-` to `reread-4-implementation-readiness.md`), and both lenses confirmed at this
commit: egm-comprehension with every Major of its two reports resolved and no Blocker or Major; egm-implementation-
readiness with its Blocker and every Major resolved and one new Major, below. That Major is settled in writing by a
decision recorded in follow-up Task 048-18; the Nits below are the minors left open at this commit, settled the same way.

## Major (approval confirmation, settled)

1. egm-implementation-readiness, approval confirmation at this commit, Major 1: Task 048-13 does not decide how the
   Compose scrape network and the two rule files fit the observability overlay (`compose.observability.yaml` overlays
   `compose.yaml`, not `compose.ha.yaml`; a shared `prometheus.yml` would make the Patroni and CloudNativePG scrape-down
   alerts fire for ever in the single-node stack; how Kubernetes loads its rule file is unsaid). Settled: Task 048-18
   decides the layout (an HA observability overlay of its own, the default stack's rules untouched, a `PrometheusRule`
   on Kubernetes), and Task 048-13 is built to it.

## Nit (settled)

2. egm-implementation-readiness, approval confirmation, Nits 1 to 3 (the Patroni allowlist check has no task; Requirement
   048-001's policy omits the monitoring peer; the `up` guard is not written).
3. egm-comprehension, approval confirmation, Nits 1 to 5 (the negative control's row, block and judgement; "operator"
   for two things; `database-runner`'s `denyCidrs` undefined; where `client_connection_check_interval` is set; fault
   (3) and the qualifying rule).
4. The Nits of the repository reviewers' approval confirmations at this commit (`approval-correctness.md` Nit 1,
   `approval-architecture.md` Nits 1 to 3), all on the scrape network and the monitoring peer.
5. The full round's Nits not fixed by the re-reads (zoom-out Nit 4: a three-member etcd with no etcd fault, kept as the
   shape of production's consensus store; the others fixed or moot).

Counts: Blocker 0, Major 1, Nit 4

Findings 1 to 4 are recorded as design 048's follow-up Task 048-18, added after the approval, each built test first;
finding 5 stays as the design chose. None is a security finding: every finding of every security report of this
approval run is fixed. Owner decisions 2026-11-24-519 and 2026-11-24-520 record the settlements.
