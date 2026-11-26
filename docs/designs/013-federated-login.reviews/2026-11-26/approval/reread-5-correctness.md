# Targeted correctness re-read of design 013 for approval, round 5 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: e0da753e272e0174382c72882cfdfeb9d97e9ccf
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a confirmation of round 4 and a re-read of the changes since (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. Round-4 findings 1 to 5 are closed; the uncounted filter point is closed for `integration_federated_login` only (finding 1).
Counts: Blocker 0, Major 0, Nit 3

1. **Nit. Targets; Tasks 013-9 to 013-13.** Only `integration_federated_login` names modules, so the `signin` filter on the contract and fault targets and the `verify`, `scim`, `admin` and `maintenance` filters can select no test, and `cargo test` passes with zero run. Fix: every verification filter is a module in every target it is applied to, and each verification reports a non-zero count of tests run.
2. **Nit. Technical Plan 12, the stored document, `rotate` and `drop_retained`.** The commit point is not in the document's field list, so a `rotate` between a crashed `revoke` and its `--resume` may drop it and force a fresh revocation. Fix: the point in the document's fields, carried forward unchanged by every command operation; `a_revoke_resumes_after_an_intervening_rotation`.
3. **Nit. "Lock order"; Requirement 013-016.** A suspension or member removal revoking one subject's sessions locks them in index order, so it can deadlock with a `revoke` batch. Fix: those paths use the same `ORDER BY tenant_id, id FOR UPDATE` subquery, in `lock_order.rs`.
