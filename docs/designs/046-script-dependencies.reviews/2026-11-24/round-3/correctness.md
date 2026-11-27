Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: 4c35bed4ed55c5aa10b08e2dd1749ffbb9be3dd1
Reviewer: correctness (repository agent `.claude/agents/correctness-reviewer.md`, round 3)
Agent-blob: correctness-reviewer
Counts: Blocker 0, Major 12, Nit 3

Condensed by the author; every finding and severity kept. Round 2: answered 1, 6, 8, 9, 11 to 13; in part 2 (finding 8), 3 (finding 5), 4 (findings 1 and 2), 5 (finding 12), 7 (finding 11); 10 answered in text, buildability in finding 10.

1. **Major.** A refused or exhausted install fails the version and its siblings but not the build job inserted at resolve completion, which stays queued and counted in the quota. Fix: fail the build job in the same transaction; the build claim and the quota count only jobs of `BUILDING` versions.
2. **Major.** A refusing completion locks its job, the version, then a sibling job; the sibling's completion holds its job and waits for the version: a deadlock. The two-installs test tests what the design no longer does. Fix: never write sibling rows, or lock the version first; a barrier test.
3. **Major.** `script_execution_admitted_instance` returns `bool` (false leaves the step ready, counted as a retention refusal). Fix: a three-way admission and `reject_input` on denial.
4. **Major.** A blocked step with no runner holding its volume is never advertised, so never leased, so never failed. Fix: advertise on `PACKAGE_BLOCKED`, or the control plane fails ready steps of a blocked artifact; a test.
5. **Major.** A re-evaluation page can delete a fresh reservation block (it does not hold the reservation revision); the order of tenant-wide and namespace rows is undefined.
6. **Major.** The older-or-shrinking refusal is checked while staging, outside the lock; an older import switching last replaces a newer one. Fix: re-check after taking the snapshot row in the switch.
7. **Major.** The hold delays malware without limit while finishes and leases go on. Fix: while held, finishes refuse versions matched by its malicious records and an alert fires; state the exception.
8. **Major.** A waiver that expires between resolve completion and the finish, before the hourly pass, is not seen. Fix: store the earliest waiver expiry in the report and re-evaluate past it.
9. **Major.** Test runs skip the start check (the runnable route takes an artifact digest). Fix: a runnable check by test run and lock digest.
10. **Major (plausible).** `finish_build` and `finish_build_cached` are single adapter calls with precomputed inputs; the evaluation cannot run between their `FOR SHARE` and the commit. Fix: the store compares the revisions the application evaluated against and returns stale inputs for a retry.
11. **Major.** A collected file cannot be fetched again by hash (no upstream name). Fix: install fetches through name-carrying routes with an upstream fallback and the hash check.
12. **Major.** Install-time gateway failures have no retryable code. Fix: their codes and a fault test.
13. **Nit.** The pending-activities route is namespace-scoped.
14. **Nit.** A 4 GiB layer at 20 s per 100 MiB is 819.2 s, more than the 600 s install job and the 10-minute materialization.
15. **Nit.** A lapsed lease is reclaimed at once, not after `not_before`; assert the switch's order, not its timing; re-evaluation rows are keyed per tenant.
