Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cfb22df54df5b084f748531dbdb9b3546c01f60d
Reviewer: egm-implementation-readiness (egm-review stage 9)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 1, Major 8, Nit 2

Scope: lines 663 to 765.

1. **Blocker.** Task T046-1 leaves design to its builder: the closed lock subsets, the bundle version 5 schema, the runner routes' payloads, the job token's claims, the error catalogue and the data model. Technical Plan 14 lists tables without columns; the build-job kind and the re-scan cursor have no schema. Fix: put the contracts and data model in files at the reviewed commit; make Task T046-1 a transcription.
2. **Major.** Slices are not crate-disjoint: Task T046-9 (upload endpoint in `api-http`, table in `infra-postgres`, rule in `infra-packages`, a CLI duplicated in T046-19), Task T046-14 (routes and a table), Task T046-21 (metrics in the gateway and runner from slice 1). Fix: split by crate.
3. **Major.** No task for job and service token issuance and key rotation, the policy-by-digest endpoint, the import-name mapping, or the egress path of the OSV pass.
4. **Major.** No task dependencies or phases; the capability gate lands after the publication flow. Fix: ordered phases that build and test; gate first.
5. **Major.** "Or the extension of Task T023-061", "shared with Task T023-047", "or added by this design" leave the table, migration and module to the builder.
6. **Major.** No versions pinned (uv, npm, base images, Rust crates for TOML, PEP 440, PEP 508, node semver, CycloneDX, tar); the probe has no acceptance and no fallback.
7. **Major.** The test run of a draft with a manifest (journey 1) has no task; its layer's signing, storage and caching are undefined.
8. **Major.** Background work undesigned: layer deletion after 30 days, waiver expiry, the re-scan's lease, page size and lock order.
9. **Major.** Tasks name areas, not files, tests or evidence commands; several acceptance criteria have no named test.
10. **Nit.** Wrong or missing `Refs:` (T046-21, T046-23, T046-24; Requirement 046-006 mapped only to T046-12; no non-functional requirement referenced).
11. **Nit.** Recorded upstream fixtures: capture, format and location unnamed.
