Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: 4c35bed4ed55c5aa10b08e2dd1749ffbb9be3dd1
Reviewer: security (repository agent `.claude/agents/security-reviewer.md`, round 3)
Agent-blob: security-reviewer
Counts: Blocker 0, Major 4, Nit 5

Condensed by the author; every finding and severity kept. Round 2: answered 1 to 7, 9, 10 (contests on 4 and 5 judged reasonable); 8 in part (finding 8 below).

1. **Major.** Nothing ties a locked file to the name and version it claims: an author lock can label a malicious wheel or tarball `requests 2.32.3` or `lodash`; every policy rule and every later block matches on the label. Fix: the metadata route looks up name, version and file in the registry's own document and returns its hash, and the control plane refuses a mismatch (private files against `private_packages`); the layer tool checks `.dist-info` and `package.json`; a relabelled-file test.
2. **Major.** Checked-hash bytecode checks only the source hash in its header, so a planted bytecode file with the right header runs; a compromised unpacker can make `__pycache__` unwritable so compilation fails and the planted file stays; the restart check skips `__pycache__`. Fix: bytecode inside the digest (proved reproducible) with forced compilation and a failed install on any write failure, or no compile step and `-B`.
3. **Major.** Only index requests for reserved names are rerouted: metadata and file requests carry the name only inside the file name and go to the gateway; npm's scoped `@acme%2fbilling` collides with the rule against encoded separators. Fix: each forwarded route with how its name is extracted; one canonical decoding; private metadata and file routes; refuse what does not parse; tests that the gateway receives none.
4. **Major (suspicion).** npm's abbreviated documents have no per-version publication time, so the cut cannot be applied from them. Fix: read `time` from the full document with a streaming, size-capped parse; drop versions without a time and PEP 691 files without `upload-time`.
5. **Nit.** The fetch budget charges only locked files not yet stored; resolve traffic is free.
6. **Nit.** Blocks keyed by `(id)` and a lease predicate without the tenant; identical digests in two tenants; the runnable route trusts a runner-supplied namespace. Fix: tenant in keys and predicates; namespace from the leased attempt; `404` for unknown digests.
7. **Nit.** One process holds both the runtime role and the import role.
8. **Nit.** Uploads: a decompression bound when reading the archive's name; no quarantine on uploads.
9. **Nit.** While an import is held, refuse publications and materializations matching its malicious records.
