# Implementation ledger for release 0.3.0

Owner instruction, 2026-10-26: “Continue com a implementacao do restante agora para a release v0.3”.
Baseline: `develop@70f0943540859a4b581b8a848e7ed3fa53edbf8a`; branch
`feature/014-v0.3-completion`. Preserve sibling worktrees and their uncommitted changes.

The release target is 0.3.0. The eight remaining canonical tasks are T014-014–017,
T014-040, T014-042–044. Their original acceptance criteria remain in force; this
ledger decomposes execution and does not close them. Feature 016 and 017 are
released dependencies, not new product scope. GitHub Releases remains the delivery channel.

## Ordered increments

1. [ ] Finish the existing collection output/result compatibility frontier: genuine old
   Applied report, exact replay, stopped drain and original-forward compensation; current
   authority, terminality and uncertain-commit controls. Review existing unverified work
   before reuse. Verify through public gRPC and real PostgreSQL in owner/runtime profiles.
2. [ ] Complete bounded scoped planning and persistence for broader acyclic bodies. Start
   with one control shape at a time, with failing domain and public runtime tests. Preserve
   local identity, evaluation snapshot/CAS, reserved derived instances, cancellation and
   compensation. Keep publication closed for every shape whose runtime proof is absent.
3. [ ] Complete the accepted capacity-group protocol in bounded transaction families:
   own-member authority, current forward versus reserved drain eligibility, durable
   discovery/parking/transfer, creation/continuation/cancellation and retention. Preserve
   chain-before-own-run entry; never mutate a second existing run. Test the actual lock graph.
4. [ ] Integrate quota projection, mixed-writer rollout and one nested public shape. Prove
   capacity-one progress, policy reduction including zero, descendant drain, detached-child
   finality, waking an ancestor while a child waits, stale transfer epochs and restart.
5. [ ] Extend the common five-SDK fixtures for the accepted shapes; inspect golden changes
   and rerun without blessing. Run old-client/old-worker/active-run upgrade evidence.
6. [ ] Run the complete fault/security matrix and measure the specified 10,000-item
   concurrency 1/16/256, 10,000 suspended approvals and nested 100,000-instance workloads.
   Record environment, failures, queue delay, transactions, memory and journal growth.
7. [ ] Close only evidenced canonical tasks; update contracts, telemetry, runbooks,
   traceability and demos. Run all required integrated gates, then prepare 0.3.0 release
   metadata under Gitflow. A release target does not waive a failed gate.

## Verification discipline

Behavior changes start with a test observed failing for the relevant reason. Races use
held locks/barriers. Tests start their own repository-declared disposable services. Use a
private target directory outside the repository and at most two heavy builds. Each accepted
increment records its command, result and limitations here or in the validation report.
Required source/transaction decisions receive bounded adversarial review; the owner's prior
choice of the current review and smaller decisions remains applicable.

## Initial inventory

- Fresh worktree is based on the merged 0.2.0 backmerge, not the stale local develop checkout.
- `feature/014-bounded-execution` contains uncommitted changes; preserve them.
- `feature/016-project-completion` contains an unverified Applied compatibility test diff;
  review and verify in this worktree before accepting any of it.
- Existing scalar/path stores and dormant capacity/source-certification stores are available.
  Dormant storage is not evidence of enabled nested execution.

Refs: T014-014, T014-015, T014-016, T014-017, T014-040, T014-042, T014-043, T014-044.
