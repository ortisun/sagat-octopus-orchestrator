# Targeted architecture re-read of design 050 for approval, round 2 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 239577a1
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a re-read of the text changed since round 1 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found its eight round-1 findings still closed and no change contradicting an accepted decision without saying so.
Counts: Blocker 0, Major 2, Nit 3

1. **Major. Technical Plan 6, Constitution Check row II, Task 050-4.** Nothing says where the cursor is sealed: the use case cannot depend on `infra-secrets`, and `api-http` does not; `aes-gcm-siv` is in no task. Fix: a port `InboxCursorSealer` in the application, its adapter in `infra-secrets` (as `SecretCipher` is), wired in the control plane and the harness.
2. **Major. Technical Plan 6, Requirement 050-010, Architecture decision 0144, the runbook.** Both inbox routes now need `SAGAT_SECRETS_KEYS`, off by default; the requirement is unconditional, the `503` has no reason, and the Studio cannot tell it from overload. Fix: the condition in the requirement and the decision record, `KEY_RING_NOT_CONFIGURED` with its remediation, the Studio's unavailable state with a test, a runbook line, the harness with a ring.
3. **Nit. Task 050-4.** The derivation is not among its deliverables; labels are free strings. Fix: list it unless Task 013-8 landed it; a closed enumeration of labels.
4. **Nit. Architecture decision 0144, "Enforced by".** Decisions 4 and 5 have no listed checks. Fix: name the four tests.
5. **Nit. Metrics.** Only the approvals list has a rows histogram. Fix: one histogram with a closed `list` label.
