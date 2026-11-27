Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: c00d7999f24de5eef7cbe2e2a95d1059bf27f03e
Reviewer: egm-comprehension (egm-review stage 6, round 2)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 7, Nit 7

**What this proposes**

From release 0.3.0, TypeScript and Python scripts can declare packages (a PEP 723 block for Python, a `package.json`-style manifest for TypeScript). At publication a resolve job in a sandbox container with no network turns the manifest into a lock; its HTTP requests are relayed by the runner over the supervisor channel to a new package gateway, the only component that reaches a registry. The control plane validates the lock, evaluates it against the namespace's policy and the OSV snapshot, and queues one install job per required target, each unpacking hash-checked files into a layer with a deterministic digest. Bundle version 5 names the lock and layer digests. At run time the runner rebuilds the layer if it lacks it, checks the digest and mounts it read-only. Blocks stop leases when malware or policy changes appear later. Private packages, offline bundles, an SBOM and provenance complete the feature.

**How the existing system works, per this document**

Feature 005 refuses every package for TypeScript and Python; scripts are built into signed bundles of at most 2 MiB in PostgreSQL, cached by a build key, run by runners that lease steps from the engine in containers with no network, a read-only root and nothing mounted, over a JSON-line channel.

**Findings**

1. **Major.** The test-run path: who installs, against which digest, and which gate accepts its lock (a test run is neither a dependency job nor a bundle version 5).
2. **Major.** The gateway has no database, yet filters by the job's policy and reserved names and serves private files; where it gets them and the job token's claims are not stated.
3. **Major.** The author-lock path: which transition fetches the metadata, under which token, and inserts install jobs.
4. **Major.** "A malware block is removed only by an administrator" conflicts with re-evaluation removing blocks; no permission or route for unblocking.
5. **Major.** Phase A's acceptance needs the hostile battery (Task 046-22, step E) and a lease test whose check is Task 046-17 (Phase B).
6. **Major.** No task owns vulnerability and licence evaluation, waiver routes, findings listing or the package-rules endpoint.
7. **Major.** Who counts the fetch budget and where publication fails.
8. **Nit.** The mount probe is Task 046-12's, not Task 046-16's.
9. **Nit.** Requirement 046-019 and Technical Plan 4 list different retryable statuses.
10. **Nit.** Adding or removing a waiver and deleting an upload do not trigger re-evaluation.
11. **Nit.** "Design 038's budget test" and "the hardening anchor" are undefined.
12. **Nit.** `apps/cli` belongs to neither builder; control-plane metrics have no owner.
13. **Nit.** "(no input or output)" is unclear.
14. **Nit.** The restart marker check does not say whether the digest is recomputed.
