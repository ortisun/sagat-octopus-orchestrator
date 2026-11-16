# Review: feature 022, 0.3.0 slice, second build — architecture

- **Change:** `feature/022-throughput-control`, commits `fe8b2d57..162ec3aa` (T022-002, T022-013, T022-069, T022-095)
- **Reviewer:** `architecture-reviewer` (independent read-only agent, given the changed files, the constitution, the ADRs
  and the spec), 2026-11-16
- **Recorded by:** the author, from the reviewer's report. 1 Blocker, 4 Major, 7 minor, 4 nits.

| Finding | Severity | Answer |
|---|---|---|
| B1 — the per-key rotation goes against ADR-0056 decision 7, accepted, with no new ADR | Blocker | ADR-0093 written (Proposed), indexed in `docs/adr/README.md`; it needs the owner (O1170). T022-095 stays unticked with an `Owner-gate` follow-up; the pull request stays draft. Not open as a defect of the code: the owner decides |
| M1 — the retire trigger adds a deployment-wide lock and a quadratic cost | Major | Fixed: lazy retirement, no trigger (D022-043); see correctness F2 |
| M2 — a rotation may never finish; the rollback path was removed; activation before every process holds the key fails every decision | Major | Owner question O1170 (what ends a rotation); G1193 (activation before key distribution) recorded in ADR-0093's consequences. A replaced version is never active again: stated as decision 4 of ADR-0093 for the owner to confirm |
| M3 — `file_keys` breaks the lock order when a retiring row vanishes | Major | Fixed: retryable refusal; see correctness F1 |
| M4 — the node-declarations limit is read before the transaction (ADR-0027) | Major | Decided and recorded (D022-043): a publication-time policy read like the `workflows` quota's limit; lowering never revokes. The store's transaction does not judge it again |
| m1 — "an older reader never reads the new names" is false (`stored_limits` refuses unknown names) | minor | The migration comment is corrected; G1192 records it (true of every earlier resource) |
| m2 — rules without a check that fails | minor | Added: `scripts/check-architecture.py` refuses a key-row insert outside `flow_control.rs` (test red when the rule allows every file); `contract_flow_control_step_policies::quotas` pins the OpenAPI enums and the newest `CHECK` lists to `QuotaResource::ALL` (red with a name removed); the quota maxima use the domain's constants. Not added: a general check that delegating stores override defaulted methods (the one method of this change has no default, so the compiler enforces it) |
| m3 — concurrency claims of D022-042 without held-lock tests | minor | The vanished-row race is tested with a held lock; the last-two-rows race is moot (no trigger); the activation waiting for a decider is the `EXCLUSIVE`/`FOR SHARE` conflict of the foundation, not new here |
| m4 — documents out of step; the plan has separate live-key ceilings for semaphores and windows | minor | `flow_control_live_keys` renamed `concurrency_live_keys` (semaphores only; windows with T022-077). OD-2026-11-16-2's status now says what was done. Approved texts: G1195 |
| m5 — waiters per key not kept below `queued_runs` | minor | Fixed; see correctness F5 |
| m6 — raw transaction helpers are public | minor | `#[doc(hidden)]` with a comment; the store keeps the only callers once T022-096 lands |
| m7 — rotation progress leaves no trace | minor | G1196 (metric and runbook with T022-061, T022-082) |
| nits — a doc comment naming `FLOW_CONTROL_LIMIT`, an unused `DEFAULT_NODE_DECLARATIONS`, the index column order, the namespace foreign key | nit | Fixed: the comment, the constant (now the default of the quota resource), the index (group first), the foreign keys (security S3) |
