# Targeted architecture re-read of design 013 for approval, round 10 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: e0da753e272e0174382c72882cfdfeb9d97e9ccf
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a confirmation of round 9 and a re-read of the changes since (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The round-9 finding is closed; the changed text adds no layering, compatibility or visibility defect, and the visibility claims match `crates/infra-secrets/src/openbao.rs` and `lib.rs`.
Counts: Blocker 0, Major 0, Nit 4

1. **Nit. Technical Plan 12, `revoke`; the allowlist; Task 013-8.** No operation reads the revocation's commit point for `--resume`, the stored document's field list omits it, and Task 013-8 lists `revoke` without `--resume`. Fix: a read-only `revocation_point(&IdentityOpenBao) -> Result<Option<KeyId>, RingError>` through `read_versioned`, allowlisted; the point in the document's fields; `--resume` in Task 013-8.
2. **Nit. Task 013-8 against Technical Plan 12.** "The submodule `transport` with `RingPath` and `RingDocument`" would put the types where `identity_ring` cannot build or read them without an unlisted accessor. Fix: `transport` holds `read_versioned` and `write_cas` over `RingPath` and `RingDocument` defined privately in `identity_ring`.
3. **Nit. Task 013-8 red tests; the lock-order path.** Three refresh-dependent tests sit in Task 013-8, before the refresh grant exists, and no task adds `revoke`'s batch path to `lock_order.rs`. Fix: those tests insert sessions directly and use the ring's `mint` and `verify`, or move to Task 013-9; Task 013-8 adds the `revoke` batch path.
4. **Nit. Targets; Task 013-9 verification.** The `signin` filter also runs on the contract and fault targets, whose tests are selected only from a `signin` module there. Fix: each task's filter names a module in every target its check runs.
