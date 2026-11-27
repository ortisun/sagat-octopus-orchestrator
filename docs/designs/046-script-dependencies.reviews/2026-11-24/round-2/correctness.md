Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: c00d7999f24de5eef7cbe2e2a95d1059bf27f03e
Reviewer: correctness (repository agent `.claude/agents/correctness-reviewer.md`, round 2)
Agent-blob: correctness-reviewer
Counts: Blocker 0, Major 10, Nit 3

Condensed by the author; every finding and severity kept. Round 1: answered 3 to 7, 9 to 11, 13, 14; not answered 1 (finding 1), 8 (finding 7), 12 (finding 9); in part 2, 15, 16.

1. **Major.** A cached build (`claim_build` then `finish_build_cached`) only sets READY: no snapshot recheck, no SBOM row, another version's provenance. Fix: run the finishing steps on both paths; a test with an import before a cached finish.
2. **Major.** The finish rechecks only the snapshot; a policy change, waiver expiry or reservation during BUILDING is missed. Fix: the finish holds the policy row and a tenant reservation revision `FOR SHARE` and re-evaluates on a difference.
3. **Major.** Re-evaluation pages read inputs without locks; an older job can delete a block a newer one wrote, or an import's malware block. Fix: pages hold the policy and snapshot rows `FOR SHARE` or one pass per namespace in order; never delete import or operator blocks.
4. **Major.** "The last install completes" is a lost wake-up under READ COMMITTED; siblings after a refusal undefined; `UNIQUE` with a NULL target. Fix: lock the version row after the job row; a refusal cancels siblings; `NULLS NOT DISTINCT`; a two-target barrier test.
5. **Major.** Error classification: a transient fetch failure of an npm optional dependency yields a lock without a binary and a non-retryable refusal; the gateway's filter turns source-only, quarantined and denied into `RESOLUTION_FAILED` on the resolve path; refusals of resolved locks are asynchronous; the author-lock path's outage; the fetch budget cannot outlast the day with three attempts. Fix: any forwarded transport failure fails the job as `PACKAGE_GATEWAY_UNAVAILABLE`; codes per path.
6. **Major.** The gateway cannot apply deny globs, reservations or the quarantine from a digest; gateway clock skew. Fix: an absolute quarantine cut from the control plane's clock in the token; the filter fetched by digest.
7. **Major.** Collection's re-read then delete is not atomic with new references. Fix: mark, then delete after a grace period longer than the longest publication.
8. **Major (plausible).** Directories 0555 owned by 0, then compiling as user 65534 without capabilities cannot create `__pycache__`; `compileall` failure handling. Fix: unpack, compile with per-file errors recorded, fix modes, then the marker after the runner compared the digest.
9. **Major.** A test run gets a gateway token only after posting a lock, but resolving needs one first.
10. **Major.** Whether evaluation reads the advisories in the transaction that stores `snapshot_ids`; findings replaced at finish; the report's home in bundle-v5.md versus the design.
11. **Nit.** The cross-host digest test compares two architectures; assert the re-queue, not "no version change"; no test for re-evaluation after a non-malware advisory.
12. **Nit.** The runner has no stable instance identifier; volumes orphaned on restart.
13. **Nit.** The job token is bound to the job, not the lease attempt.
