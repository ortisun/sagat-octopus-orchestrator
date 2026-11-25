Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cfb22df54df5b084f748531dbdb9b3546c01f60d
Reviewer: architecture (repository agent `.claude/agents/architecture-reviewer.md`)
Agent-blob: architecture-reviewer
Counts: Blocker 1, Major 10, Nit 6

Condensed by the author from the reviewer's output; every finding and severity is kept.

1. **Blocker.** The block on the artifact row cannot change: `script_artifacts_immutable` refuses every update and delete; the row is per tenant while `blockNewAtLease` is per namespace. Fix: a table `script_artifact_blocks(tenant, namespace nullable, digest, reason)` joined by Task T023-061's one statement.
2. **Major.** The policy and report digests and the snapshot are in bundle 5 but not the key: a stale signed report or two artifacts for one key; the empty lock digest makes re-resolving impossible; resolver images in the key in one place and not another. Fix: move them to the per-build provenance; never cache resolve jobs; key install and build on the resolved lock, unpacker and image digests.
3. **Major.** ADR-0120 relaxes Accepted ADR-0013 (network, a mount, a mount allowing execution) and contradicts ADR-0057 decision 4 and option 3. Fix: "supersedes ADR-0013 in part", narrow ADR-0057 decision 4, state "not `noexec`".
4. **Major.** ADR-0023 caps a blob at 64 MiB and keys blobs under `tenants/<t>/namespaces/<n>/`; layers of 4 GiB, other key schemes, a retention policy, and an internet-facing gateway holding blob credentials that read every tenant's payloads. Fix: a separate bucket and credential, the gateway writing only its prefix, limits and retention in an ADR.
5. **Major.** The public store and private uploads count against no ADR-0027 quota and nothing is deleted. Fix: count uploads, a per-tenant fetch budget, collection by lock references.
6. **Major.** Evaluation rules in the domain need version orders and ranges placed in the adapter; the application cannot depend on an adapter and no port is named. Fix: version orders in the domain as pure code; parsers behind an application port.
7. **Major.** The proposed architecture rules check nothing. Fix: a source check that a network mode other than `none` appears only in the resolve function; a compose test; a container-spec test that the layer is the only, read-only mount; rows in `architecture-principles.md`.
8. **Major.** The internal network exists only on one host; Helm has no sandbox-runner template; concurrent resolve containers share a bridge. Fix: decide in ADR-0120 how each runner host reaches the gateway; a network per job with inter-container traffic off.
9. **Major.** The registry knows only control planes and engines (`20270113000000_instance_registry.up.sql`), so runners and the gateway cannot list the capability; an old runner can claim resolve and install jobs. Fix: runners declare the capability at claim and advertisement; old-claim fixture.
10. **Major.** A runner has one native target (ADR-0024 decision 5); per-target installs need a runner of that target; leases on a target without a layer are undefined. Fix: per-target install jobs; READY only when every supported target has a layer; typed refusal otherwise.
11. **Major.** "Whichever feature is built first builds it" for 023's lease check, policy endpoint and provenance (023 then planned `crates/infra-attestation`) implements another feature's work without an approved dependency. Fix: one owner with a delegate record; one owner for shared migrations.
12. **Nit.** The gateway is trusted for both walls; say so; have the unpacker report the install scripts it sees.
13. **Nit.** The job token is a credential in the resolve container; call it a scoped capability with its own key.
14. **Nit.** `RESOLUTION_FAILED` missing from the closed list; two codes for an unreachable gateway; no order for "first failing reason".
15. **Nit.** One advisory import is an unbounded transaction; no blocked-lease counter; label values unlisted.
16. **Nit.** New variable-size columns missing from `storage-classification.json`; layer deletion after the last reference cannot happen because artifacts are never deleted.
17. **Nit.** Principle II's "split a process only after measurement" not addressed for the gateway.
