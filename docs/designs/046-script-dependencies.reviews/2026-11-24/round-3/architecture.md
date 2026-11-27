Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: 4c35bed4ed55c5aa10b08e2dd1749ffbb9be3dd1
Reviewer: architecture (repository agent `.claude/agents/architecture-reviewer.md`, round 3)
Agent-blob: architecture-reviewer
Counts: Blocker 0, Major 6, Nit 5

Condensed by the author; every finding and severity kept. Round 2: answered 1 to 4, 6 and Nits 8 to 12; in part 5 (finding 6 below) and 7 (findings 2 and 3); 13 answered in the design but not in ADR-0120.

1. **Major.** `script_execution_admitted_instance` is the retention-quota admission (a `bool`: false means stay ready); the scoped child-root lease path calls `script_execution_admitted` directly (`worker/scoped_claims.rs`, `infra-postgres/src/execution/path.rs`), so blocked children would be leased; feature 023's plan 8a says `lease_one`. Fix: a typed admission (`Admit`, `Wait`, `Deny(reason)`) on both lease paths; a child-step test; plan 8a aligned.
2. **Major.** The backfill pass contradicts the model: install completion requires `BUILDING`, layer digests are guarded once `READY`, records are signed only at finish; no task covers it. Fix: a backfill install that signs a record for a `READY` version without changing its state; digests per target only in the records; a test.
3. **Major.** A step whose layer no live runner can build waits silently before its lease (no record for the target, installs failing everywhere, the gateway down); `LAYER_UNAVAILABLE` has nothing to attach to. Fix: runners report materialization failures; a waiting gauge; after N failures or no record for any registered target, fail the step through `reject_input`; a test.
4. **Major.** ADR-0120 and ADR-0121 lag the design (the gateway "applies policy" in option 1; "refines ADR-0023" against decision 4; the private routes missing; the checks' crate lists; test names no task creates; the gateway's fetch of archives).
5. **Major.** Nothing enforces the gateway's isolation: no check fails if `apps/package-gateway` links the PostgreSQL adapter, the application or the sandbox crate. Fix: a dependency allow-list like `LAKE_RUNNER_PACKAGES`, with a negative test.
6. **Major.** Where the advisory switch's range matching runs (it needs `PackageRules`), and finishes committed between staging and the switch. Fix: precompute matched versions at staging; the switch also matches newer rows by exact versions; extend the held-lock test.
7. **Nit.** The gateway cannot apply the wheel filter by target without a target claim.
8. **Nit.** Ports for the SBOM and provenance builders and the advisory parser are unnamed.
9. **Nit.** The `sagat_advisory_import` role's grants and credential.
10. **Nit.** The activation guard of Requirement 046-020 has no check.
11. **Nit.** "No other component MUST reach" should be "Other components MUST NOT reach".
