# egm-review re-read of design 013 for approval, round 1 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 9c893782
Reviewer: egm-review (re-read for approval, egm-design)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: a design-critic re-read of the text changed since the last full stages (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer confirmed the three plain reads, the five implementations, `usable_key`, `KeyRing`'s order, the opener policy, the `Slot` variants, the latest migration and the registry member's behaviour, and found the 029 contract amendment consistent.
Counts: Blocker 0, Major 1, Nit 7

1. **Major. Technical Plan 4 step 4, Technical Plan 13, Architecture decision 0050's addition.** The link and test confirmation takes the connection row after the membership, against the recorded lock order, and can deadlock with a redemption or a SCIM request of the same subject; it is not among the advisory-lock paths nor in `lock_order.rs`. Fix: connection row, re-check, advisory lock, membership; the path in `lock_order.rs`; a barrier test.
2. **Nit. Technical Plan 4.** `signing_inside_a_transaction_does_no_io` and the revoked-handle check have no task. Fix: Task 013-9.
3. **Nit. Technical Plan 3.** The tombstone has no home, and the refresh grace needs per-generation hashes, times and use. Fix: keep the code row with its identity cleared; name the grace's columns.
4. **Nit. Technical Plan 6.** `scimFailing`'s counter has no place, and a write per SCIM request on the connection row would queue sign-ins. Fix: a separate health row written outside the request's transaction.
5. **Nit. Technical Plan 11.** A counter with no connection label cannot express ten consecutive refusals per connection. Fix: a gauge set from `scimFailing`.
6. **Nit. Technical Plans 6 and 13.** A guard exception from `sagat_member_set_role` would roll back the whole SCIM change. Fix: a guarded outcome for recomputation callers.
7. **Nit. Technical Plan 4 "Lock order".** A SCIM group request's first lock is given as memberships, against Technical Plan 13. Fix: reword.
8. **Nit. Technical Plans 3 and 6.** The compare-and-set of a per-member row older than the current revision is undefined. Fix: recompute under the current rules, clear on the row's own revision; a newer job supersedes older per-member rows.
