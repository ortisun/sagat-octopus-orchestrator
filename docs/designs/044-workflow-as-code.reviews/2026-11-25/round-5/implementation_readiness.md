# egm-review Stage 9 re-audit (implementation readiness), round 5 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 9b64ac22698c34d847c9658b8a08b5a5c68814d2
Reviewer: egm-implementation-readiness, implementation-readiness standard, DetailedRange 1916-2426, code claims checked
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The round-4 Blocker and
all six round-4 Majors are closed (publication and `signal-inbox-v1`, task order, the typed append, crash tests, the
scripted decider, signatures and packaging, move-job fields).

## Major

1. The call-site check relies on rustdoc JSON, which holds no call sites and needs nightly against the pinned stable
   1.100.0; name a workable mechanism (a text check in the manner of `managed_kind_sites.rs` with a checked-in allowlist,
   or a pinned SCIP index) or drop the check.

## Nit

2. Task 044-31 keeps the old signature and has no Files line.
3. Section 3.3 and rollout step 1 still require `signal-inbox-v1`; who switches the refusal to a hold.
4. A stale build-order example; gating the SDK runtimes on Task 044-9 being ticked.
5. Crash-point values, the advisory-lock key, a test name, Files lines for the hook and the decider.
6. TypeScript packaging: the bundler package directory, worker registration, the replayer taking a bundle, the
   `RetryPolicy` name clash, `acknowledgeCancellation`.
7. The esbuild review procedure (`scripts/check-vendor.py` covers only rskafka).
8. The real sync-script and proto paths of each SDK.
9. A test said to be extended does not exist.
10. The CLI and Studio tests are not gated and have no files.
11. The reconciler parity test is in no task.
12. `hold_requirement`'s signature and error are only on another branch.

Counts: Blocker 0, Major 1, Nit 11
