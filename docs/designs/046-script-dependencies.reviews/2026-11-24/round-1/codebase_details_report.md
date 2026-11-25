Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: cfb22df54df5b084f748531dbdb9b3546c01f60d
Reviewer: egm-codebase-details (egm-review stage 9)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 3, Major 11, Nit 1

1. **Blocker.** `runnable` on the artifact row: `script_artifacts_immutable` refuses every update (`crates/infra-postgres/migrations/20261014000000_scripts.up.sql`); no lease-time script check exists in `crates/`. Fix: a separate block table keyed by tenant and digest read by the lease statement; name `crates/application/src/worker.rs` (`lease_one`) and `crates/infra-postgres/src/workers.rs`.
2. **Blocker.** "Kinds of the existing build job": `script_build_jobs` is `UNIQUE (tenant_id, version_id)` with no kind column; the build key is computed at claim from the draft's lockfile (`crates/application/src/scripts.rs`), before any lock is resolved. Fix: the schema (phase column, unique key), when the key is computed, and how `build_key` (`crates/application/src/sandbox.rs`, `sagat/build/v1`) keeps framework-only keys identical.
3. **Blocker.** Port 18090 is dev-idp's (`infra/docker/compose.yaml`); the compose `name:` prefixes networks. Fix: a free port; `name:` on the network.
4. **Major.** The existing lockfile code is ignored: `Lockfile` documented as `requirements.txt` with hashes for Python (`crates/domain/src/scripts.rs`), lockfile columns on three tables, `crates/infra-sandbox/src/lockfile.rs::accept`. State the format change and how existing locks are treated.
5. **Major.** `crates/infra-schema/src/bundle.rs::decoded_files` accepts versions 1 to 3 only; `infra-schema` is missing from the placement table; version 4 exists only as feature 023's contract.
6. **Major.** `BlobKey::of` yields `tenants/{t}/namespaces/{n}/blobs/{id}` (`crates/application/src/blobs.rs`), `BlobService` caps blobs at 64 MiB and is scoped to step output or run input; the gateway has no database for the catalog.
7. **Major.** `EgressClient` buffers whole bodies (1 MiB, 15 s, no redirects; `crates/infra-egress/src/lib.rs`); PyPI files are on `files.pythonhosted.org`. Specify a streaming API and the file host.
8. **Major.** `/script-runner/*` routes have a 4 MiB body limit (`crates/api-http/src/lib.rs`, `RUNNER_MAX_BODY_BYTES`); routes register in `lib.rs`; script routes are namespace-scoped and drafts are edited by PATCH, so `/v1/scripts/{id}/draft` does not exist.
9. **Major.** Bounds: the result frame is 240 KiB (`MAX_RESULT_BYTES`), so a JSON-escaped 256 KiB lock does not fit; scratch at most 512 MiB with `fsize` equal; both `network_disabled` and `network_mode: none` are set; `PYTHONDONTWRITEBYTECODE=1` already set; `Images` and the runner's image variables cover only the Alpine images.
10. **Major.** A runner knows a target only for Go; one install job cannot compile Python bytecode in the other architecture's image without emulation. Decide claim per target or cross-architecture execution.
11. **Major.** `script_artifacts_sbom_size` is at most 262,144 bytes; a 2,000-package SBOM exceeds it; the runner builds the SBOM today.
12. **Major.** Warm containers keyed by digest (ADR-0052 Proposed) and feature 023's policy endpoint are described as existing; mark them as dependencies.
13. **Major.** Missing data changes: the TypeScript `manifest` column on three tables, the policy table, `script_versions_guard`'s protected columns, a content-quota trigger for layers.
14. **Major.** No file is marked create or modify; `BuildRequest` and `RunRequest` ports lack manifest and layer fields; the job-token key versus the artifact key is undecided.
15. **Nit.** The domain-to-adapter and `serde_json` rules already exist in `scripts/check-architecture.py`; "upstream registry client" is not checkable; the domain allowlist rules out version-range crates.
