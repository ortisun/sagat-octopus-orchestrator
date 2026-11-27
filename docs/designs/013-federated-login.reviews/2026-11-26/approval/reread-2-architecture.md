# Targeted architecture re-read of design 013 for approval, round 2 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 239577a1
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a re-read of the text changed since round 1 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found its eight round-1 findings still closed.
Counts: Blocker 0, Major 3, Nit 5

1. **Major. Requirement 013-026, Technical Plan 2, Task 013-9.** The refresh rules are an application use case, which cannot take `IdentityKeyRing` from an adapter; no port for the MAC is listed, nor for the attempt sealing. Fix: ports `RefreshTokenMac` and an attempt sealer in the application, adapters in `infra-secrets`, placement rows, the use case keeping the order.
2. **Major. Requirement 013-026, Technical Plan 12, Architecture decision 0140, Complexity Tracking.** The refresh token's format, ring and lifecycle, reached after rejecting alternatives and hard to reverse, are in no decision record or register. Fix: decision 11 of Architecture decision 0140 with options and consequences; "Enforced by"; a Complexity Tracking row.
3. **Major. Requirement 013-026, the rollout.** Start-up refusing an empty ring, with the rollout enabling federation before creating the key, takes every control plane down; a missing signing key only degrades. Fix: reorder the rollout, and degrade like signing.
4. **Nit. Requirement 013-025.** The `cli-refresh` ring is missing from the list of identity secrets. Fix: add it.
5. **Nit. Requirement 013-026, Technical Plan 12.** When the ring is re-read, what a failed read does, and how long a stale replica mints after a revoke are unstated. Fix: state them.
6. **Nit. Requirement 013-026.** The revoke's random-value rotation is one unbounded update across two stores. Fix: bounded batches; a failure after the ring write is safe; a re-run finishes.
7. **Nit. Architecture decision 0050's addition, the 029 contract.** The settings-row lock and the `lock_timeout` scope disagree with Technical Plans 4 and 13. Fix: align them.
