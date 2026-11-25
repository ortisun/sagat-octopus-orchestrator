Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cfb22df54df5b084f748531dbdb9b3546c01f60d
Reviewer: correctness (repository agent `.claude/agents/correctness-reviewer.md`)
Agent-blob: correctness-reviewer
Counts: Blocker 2, Major 11, Nit 3

Condensed by the author from the reviewer's output; every finding and severity is kept. Baseline: `crates/application/src/scripts.rs` (claim and cached finish) and `crates/infra-postgres/src/scripts.rs`.

1. **Blocker.** The build-key cache skips the policy check: a cached key finishes the version with an old artifact (`built_with`, `finish_build_cached`) scoped by tenant only, so a vulnerable lock republished comes back READY, a namespace that denies a package reuses another's artifact, and the bundle names another version's report. Fix: evaluate on the cached path; per-version report outside the artifact.
2. **Blocker.** A snapshot imported between resolve and signing is missed: the version is BUILDING and has no artifact row to block. Fix: in the finishing transaction read the current snapshot identities `FOR SHARE` and re-evaluate or refuse if they differ; the re-scan covers BUILDING versions; a test with an import held between resolve and finish.
3. **Major.** The resolve-to-install handoff has no transaction boundary; a crash leaves no job, a duplicate completion queues two. Fix: metadata call before the transaction; one fenced transaction stores lock and report and moves the job to install with a new token.
4. **Major.** An unchanged manifest never re-resolves (empty lock digest in the key); resolver image not in the key. Fix: never serve resolve from the cache.
5. **Major.** The re-scan can be lost after the import commits; no cursor. Fix: a re-scan row with a cursor inserted in the import transaction, advanced per page.
6. **Major.** One `runnable`/`blocked_reason` pair for two reasons (package and language); per tenant and digest while `blockNewAtLease` is per namespace. Fix: a set of block reasons keyed per namespace and artifact.
7. **Major.** Policy changes and waiver expiry never reach the lease check. Fix: durable recompute jobs on both.
8. **Major.** Layer retention and upload races (collection before reference, concurrent uploads, double quota, runner-computed digest, lock order). Fix: the control plane hashes the stream, conditional put, row lock order, blob deleted after the row.
9. **Major.** Volume cache population: labels cannot be added later; two runs populate concurrently; eviction between lookup and start makes the engine create an empty volume. Fix: a marker file written last, single flight from lookup to container start, marker checked before start, a barrier test.
10. **Major.** A down gateway (refused connection, timeout) is classified non-retryable. Fix: classify transport errors as their own retryable kind.
11. **Major.** Routing by native target is not designed (who builds arm64; leases to a runner without a layer; npm `os`/`cpu`/`libc` selection). Fix: a target on install jobs and the step claim; an optional native dependency fixture.
12. **Major.** Test runs are claimed but not designed (policy, malware refusal, layer lifetime, quota).
13. **Major.** Private uploads: no PEP 503 normalisation; an identical retry refused; blob and row ordering; a name made private later does not re-check locks.
14. **Nit.** Duplicate completion returns `AlreadyDone` (`Ok`) in the baseline, not "refused as stale".
15. **Nit.** Reproducibility needs two hosts and must include the bytecode files.
16. **Nit.** The job token in the resolve container contradicts "no secret" and outlives a lapsed lease; bind it to the lease.
