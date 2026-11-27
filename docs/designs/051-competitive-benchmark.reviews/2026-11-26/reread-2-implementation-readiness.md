# Targeted re-read 2 of design 051: egm-implementation-readiness (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: c91af7ac2be73ff9eb4a2163d5d124a15f9bbac8
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The approval
confirmation of this lens, not a full review; the reviewer read the design and the earlier reports and ran nothing.
All eight findings of `reread-1-implementation-readiness.md` are resolved (the injector on the host, the common-rate
hand-off, the image build, the subcommands' arguments then listed, `--only competitive`, the absence check's trigger,
the `reconcile` marker, fault (c)'s replayed WAL). Tasks 051-0, 051-1, 051-8, 051-9 and 051-10 can be built as
written; it agrees with `approval-correctness.md`'s Nits 1 and 2 without repeating them.

No Blocker.

## Major

1. The acknowledgement channel carries nothing back to the driver, and the host cannot tell when the product is idle:
   the driver posts only window boundaries and milestones, not the end of the drain and the wait for open runs; and the
   calibration's deviation, steal and the host's verdicts (a refusal, an unlogged relation, a missing key) reach the
   ladder by no stated means. Fix: a "drained" post (for example rung N+1's boundaries posted only after the wait) with
   a minimum lead before the warm-up; an acknowledgement in a fixed grammar (deviation, steal, and continue, re-run,
   rung failed or turn failed); which side decides the re-run; tests of both ends in Tasks 051-2 and 051-6.

## Nit

1. The extension, its image and the `postgres/` server service land in Task 051-6 after the tasks that need them
   (the rule and the `ci.yml` step in Task 051-2, the compose files in Tasks 051-4 and 051-5); move them before Task
   051-3.
2. "Required once `campaign.json` exists" breaks Task 051-6's commit, since the workflow comes in Task 051-8, and Task
   051-7 adds steps to that workflow; move `campaign.json` to Task 051-8 or tie the trigger to the workflow file, and
   Task 051-7's workflow steps to Task 051-8.
3. How the host's probe reaches each server (`docker exec` over the socket, or TCP), the harness role's privileges, the
   extension's function returning the count and where `CREATE EXTENSION` runs are not stated.
4. The subcommands' arguments miss the values now passed: `host` has none listed, `turn` takes no job kind or rate,
   `crash` no rate, seed or verdict.
5. No task names this platform's compose files (single node, the crash file with its load balancer, the
   sandbox-runner service with its exception); add them to Task 051-3.
6. Task 051-11 says less than Requirement 051-017 (the `version` job of `release.yml`, a fixture for each refusal and
   each exemption).
7. Whether the host-only directory of the acknowledgement and snapshots is the one of fault times and uploads is
   unclear; if it is, the driver container can read the snapshots' `query` text.

Counts: Blocker 0, Major 1, Nit 7
