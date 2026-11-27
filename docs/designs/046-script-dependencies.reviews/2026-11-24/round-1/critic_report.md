Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cfb22df54df5b084f748531dbdb9b3546c01f60d
Reviewer: egm-design (egm-review stage 7; the structural and style reports of the same stage are beside this one)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 9, Nit 4

1. **Major.** The build key's lock digest is "empty when the platform resolves" and the policy digest is left out, so the same source and manifest hit the cache and the author can never re-resolve; resolution depends on policy and clock. Fix: never cache resolve jobs by key; compute the key after the lock; an explicit re-resolve request.
2. **Major.** "Only the gateway listens" is not enforced: on an internal network concurrent resolve containers of different tenants reach each other; the host bridge address; embedded DNS forwarding; the gateway's internal API on the same listener. Fix: inter-container traffic off, a separate internal listener, a minimum engine version, hostile-battery cases.
3. **Major.** An internal network exists on one host only; a runner on another host cannot reach the gateway; the Helm text speaks of resolve pods that do not exist. Fix: a per-host path or a stated co-location rule.
4. **Major.** Engine assumptions: volume labels are set only at creation; `nosuid`/`nodev` on named local volumes is not plainly offered; two leases can populate the same volume or remove one being filled. Fix: a completion marker or runner index; one populate per tenant and digest; probe the mount options.
5. **Major.** The feature 005 profile does not fit install jobs: scratch tmpfs of 512 MiB counting against memory, per-file limit, `nofile` 256; bytecode compilation needs the tree; `uv` may download whole wheels for metadata. Fix: a resource profile per job kind; core-metadata from the gateway; compile in the populate container.
6. **Major.** Closure completeness can only be checked against the lock's own edges; the gateway's metadata returns no dependencies; npm per-target selection (`os`, `cpu`, `libc`) is not specified. Fix: return declared upstream dependencies; specify and test per-target npm selection.
7. **Major.** Dependency-confusion protection exists only after an upload and only in that namespace; PEP 503 normalisation not mentioned; no reservation. Fix: tenant name reservation, normalised matching, variant tests.
8. **Major.** Cold start fans out through the control plane (N runners × up to 4 GiB); population versus step timeout and lease undefined; no download cap; runner file cache unbounded. Fix: direct blob reads or prefetch, population under the heartbeat with a cap, a bounded cache.
9. **Major.** The OSV download "through the gateway's upstream connection" has no endpoint or egress rule.
10. **Nit.** 256 KiB lock bound versus 2,000 packages are inconsistent.
11. **Nit.** "Only mount" ignores `/tmp`; "no secret" ignores the job token.
12. **Nit.** The control plane should recompute the layer digest on upload.
13. **Nit.** Alternatives omit ADR-0120 option 5; "ten most downloaded" is not pinned.
