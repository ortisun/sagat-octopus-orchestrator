Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cfb22df54df5b084f748531dbdb9b3546c01f60d
Reviewer: security (repository agent `.claude/agents/security-reviewer.md`)
Agent-blob: security-reviewer
Counts: Blocker 0, Major 11, Nit 8

Condensed by the author from the reviewer's output; every finding and severity is kept. Finding 4 was checked by the reviewer in code.

1. **Major.** Malware blocking has two holes: the block waits for a paged re-scan, during which the version can still be leased; the same pass clears blocks, so an older, truncated or partial archive unblocks known malware. Fix: set malicious-package blocks in the import transaction from an index of locked packages; refuse an older or shrinking snapshot unless an operator forces it; clear a malware block only when the record is withdrawn or an administrator acts.
2. **Major.** A policy change "takes effect for lease checks" but only snapshot imports re-evaluate `runnable`; a new deny rule, a disabled ecosystem, `blockNewAtLease` or an expired waiver never stops a running version. Fix: re-evaluate on policy change and waiver expiry.
3. **Major.** The build key omits policy and snapshot while the signed bundle carries them; a cache hit carries a stale verdict into a new version. Fix: cache the layer by lock digest; evaluate and sign on every publication.
4. **Major (checked in code).** `script_artifacts_immutable` refuses every update of `script_artifacts` (`crates/infra-postgres/migrations/20261014000000_scripts.up.sql`), so `runnable` cannot live there. Fix: a separate table keyed by tenant and digest; restate ADR-0121 decision 5.
5. **Major.** `sagat-packages` is a shared bridge: concurrent resolve containers of different tenants reach each other; the gateway joins the platform network too; no test tries container-to-container traffic or the internal routes. Fix: per-job isolation, a separate listener, an outbound side limited to upstreams, tests.
6. **Major.** Server-side request forgery: PyPI files live on `files.pythonhosted.org`, not a default upstream, so either the gateway follows upstream URLs or cannot fetch; names enter upstream paths without validation; the runner might take a host from the lock. Fix: https only, a configured list of file hosts, no cross-host redirects, validated names, runner fetch URLs built from its configured base.
7. **Major.** Dependency confusion: names per namespace but storage per tenant; no PEP 503 normalisation; protection only after the first upload. Fix: one scope, normalised comparison in gateway and validator, reservation without upload.
8. **Major.** Tenant boundaries of the new routes are not stated (layers by digest, private files, gateway answers about another tenant's name); deterministic digests let one tenant test another's lock; the shared public store and document cache leak by timing. Fix: lookups by tenant and digest with identical 404s; split the cache or accept the residual risk in writing.
9. **Major.** No permission table: who uploads, changes policy and waivers, reads findings and SBOM; an upload hides a public name for every script that is resolved again. Fix: a role table, upload restricted, the shadowed name audited.
10. **Major.** The layer cache can trust a broken volume: labels only at creation; the hash checked while the volume is written; a crash leaves a named volume later mounted as valid. Fix: download to a file, verify, populate, write a digest marker last, require it on every mount.
11. **Major.** Install-time holes: `compileall` run with the working directory in `site-packages` imports a planted `compileall.py` or `sitecustomize`; two packages may write the same path. Fix: `python -I` outside the layer; refuse a path written by two packages.
12. **Nit.** The engine's mount API cannot set `nosuid`/`nodev` on a named local volume; "the only mount" overlooks `/tmp`.
13. **Nit.** The layer and SBOM are runner-reported (Gap 118 of feature 005 now covers large layers); mark them so in the provenance; a second-runner rebuild would close it.
14. **Nit.** Malware recognised only by the `MAL-` prefix; use the identifier, aliases and type; refuse waivers on such records.
15. **Nit.** The job token in the resolve container and in 8 KiB of kept error output; never in a URL, redact it, reject it once the job ends.
16. **Nit.** Exported store bundles and OSV archives are not authenticated, yet bundle metadata drives the quarantine.
17. **Nit.** Missing parsing limits: nesting depth, globs not regular expressions, decompression limit for upload metadata, wheel file name versus `METADATA` name.
18. **Nit.** Yanked is filtered only by the gateway, not refused or reported by the control plane.
19. **Nit.** A blocked artifact should evict a warm container holding its layer.
