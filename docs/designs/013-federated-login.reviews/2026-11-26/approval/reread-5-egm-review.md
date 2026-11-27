# egm-review re-read of design 013 for approval, round 5 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: e0da753e272e0174382c72882cfdfeb9d97e9ccf
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: a re-read for approval by the EGM design critic (`egm-design`) of the text changed since round 4 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The round-4 Major is closed (`IdentityOpenBao::client` at `pub(in crate::openbao)`, the transport at `pub(super)`) and Nits 2 to 5 are closed; Nit 1 is partly closed (finding 1). This round's four reports are the approval set: security `reread-47-security.md`, architecture `reread-10-architecture.md`, correctness `reread-5-correctness.md` and this one, all at this commit; their nits are settled in `docs/owner-decisions.md` (2026-11-26) with follow-up tasks.
Counts: Blocker 0, Major 0, Nit 4

1. **Nit. Task 013-8 red tests.** Three tests (`an_empty_cli_refresh_ring_refuses_only_cli_refresh`, `a_cli_refresh_key_rotation_keeps_a_session_refreshing_across_it`, `a_session_signed_in_after_a_revoke_keeps_refreshing`) need routes Task 013-9 builds. Fix: move them to Task 013-9, or run them in 013-8 at the ring level with inserted sessions and add route-level versions in 013-9.
2. **Nit. Technical Plan 12, *Transport*, against Task 013-8.** `RingPath` and `RingDocument` placed in `transport` would need unlisted accessors for the parent. Fix: define them privately in `identity_ring`, which `transport` can see.
3. **Nit. Technical Plan 2, the rules.** No task builds the rule on `VerifiedClaims` and `Membership::principal` or its fixture. Fix: give it to Task 013-10.
4. **Nit. Technical Plan 6, maintenance; "Lock order".** Where clearing a context floor takes its locks is unstated; clearing it in the last marking batch would lock sessions then the connection, an inversion with a Studio redemption. Fix: clear the floor in its own transaction after the last batch, settings row then connection row, in `lock_order.rs`.
