Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: c00d7999f24de5eef7cbe2e2a95d1059bf27f03e
Reviewer: egm-codebase-details (egm-review stage 9, round 2)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 3, Major 10, Nit 2

Round 1: 3 to 8 and 11 to 15 answered; 1 put the lease check in the wrong file; 2 partly; 9's bounds break again.

1. **Blocker.** `crates/infra-postgres/src/workers.rs` has no script code; the admission hook is `RunTx::script_execution_admitted_instance` (`crates/application/src/execution.rs`, `crates/infra-postgres/src/execution.rs`), called from `crates/application/src/worker.rs`, which skips a refused step; the permanent pre-lease failure path is `reject_input`; `Failure::Denied` has no reason field.
2. **Blocker.** The capability constraints were renamed by `20270118000000_run_input_validation.up.sql`: three `*_capability_known` constraints including the history one, and the set includes `run-input-validation-v1`.
3. **Blocker.** An old runner refuses an unknown bundle with `Failure::Unavailable` (`SCRIPT_SANDBOX_UNAVAILABLE`), not `SCRIPT_UNTRUSTED_ARTIFACT`.
4. **Major.** The build key includes the toolchain the claiming runner supplies, so it cannot be computed at insert; a cache hit is finished by `finish_build_cached`.
5. **Major.** `publish` deduplicates a draft equal to the latest version; `content_digest` has no manifest; the build job is created at publish.
6. **Major.** The runner assembles the bundle; `Expected` and `Bundles::check` compare only language, policy and schemas; `BuildAssignment` not mentioned.
7. **Major.** The channel discriminator is `op` and a `fetch` message exists; a 192 KiB chunk exceeds the 240 KiB line; `MAX_HOST_CALLS` 100 and `MAX_CHANNEL_LINES` 400.
8. **Major.** Sandbox containers run as 65534 with no capabilities; they cannot set owner 0 or write after 0555 (`crates/infra-sandbox/src/lake.rs` documents this).
9. **Major.** `lake.rs` already builds a sandbox container with one volume mount and `nofile` 1024; reuse it; scope the checks to it.
10. **Major.** The `script_jobs` quota counts `script_build_jobs` and `script_test_runs` by namespace (`crates/infra-postgres/src/quotas.rs`); dependency jobs have no namespace and other states.
11. **Major.** `script_build_provenance.envelope` is 65,536 bytes and keyed by build key; 2,000 entries do not fit; a reused build shows another's statement.
12. **Major.** The gateway cannot use domain rules for version ranges; it needs policy, reservations and token status from routes that do not exist.
13. **Major.** Missing files in the placement table (`scripts.rs`, `execution.rs` in both crates, `quotas.rs`, `identity.rs`, `domain/src/scripts.rs`, `dto_scripts.rs`); no general runner target setting.
14. **Nit.** Healthchecks use a `<binary> healthcheck` subcommand.
15. **Nit.** The runner gate is reached through `lockfile::approved`; `accept` has no job-kind parameter; `BlobSettings::from_lookup` hardcodes `SAGAT_BLOB_*`.
