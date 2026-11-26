# egm-zoom-out on design 051, full round of 2026-11-26

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: b604f497564d3d7de3277c99cf38caf203e59f60
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing.

## Major

1. The paid and three-machine tiers are fully specified in a free-only release: stale by the time they are paid for,
   not "unchanged" (their own costs re-run pilots), untested paths in `run.sh` and the workflow, and CUJ 3 describes a
   journey 0.3.0 cannot run. Fix: a short "Later tiers (unscheduled)" section; owner-gated follow-ups; drop the tier and
   topology variables, the runner input, Requirement 051-011 and the paid clauses; CUJ 3 on the free fault (c).
2. The reuse contract names the wrong consumers: design 048's drill extends `dr.rs` (its Task 048-14), design 052
   builds its own open-loop mode; the real consumers are designs 049 and 053, which expect a per-table statement latency
   and a step-completion-to-next-lease time this design never specifies. Fix: name 049 and 053; drop User story 051-03 and
   the "one harness" goal; correct the citations (048-3, 048-14); a first deliverable (driver core, this platform's
   adapter for the chain of three, its history loader) before any competitor work; add the two measures or state that 049
   and 053 measure them.
3. The free tier's 2-vCPU partition runs a two-of-each deployment (Temporal's services each in their own process) in
   every throughput job, and the 1-vCPU driver partition carries two acknowledged calls per activity. Fix: single-node
   documented deployments for throughput and the timer burst, the two-of-each only in crash jobs; execution records
   appended to a local file in the worker's partition before the activity body, collected after the rung.
4. The history is loaded in every free-tier job but restored on the paid tier: two paths, a time risk unmitigated, a
   contradiction in the Risks row; a 100,000-chain directory fits easily, and artifacts are free for public
   repositories. Fix: load once per campaign, store the data directory as a pinned artifact, restore before each turn.
5. The 0.3.0 claim is not tied to the 0.3.0 commit. Fix: a release note cites only a campaign run at its release
   candidate commit; a re-run before the release note.
6. The free tier's premise, a public repository, is neither stated nor checked. Fix: state it; the workflow refuses a
   private repository; the campaign's total minutes.

## Nit

1. The coefficient of variation "resting on three repetitions" (five on the free tier).
2. How a campaign of 61 long jobs is dispatched; a matrix with `max-parallel` below 20 so that CI is not blocked.
3. Temporal's script variants as an unisolated activity: report "not run natively" and do not build them.
4. Task 051-11's check of documents is once; make it a step of each release note's publication.
5. Tasks 051-9 and 051-10 overlap; merge them.
6. Requirement 051-017 comes before 051-016.

Counts: Blocker 0, Major 6, Nit 6
