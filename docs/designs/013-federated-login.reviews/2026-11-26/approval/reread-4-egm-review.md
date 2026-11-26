# egm-review re-read of design 013 for approval, round 4 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: a8820014ea23e0d9a2b5305437956597d29e197a
Reviewer: egm-review (re-read for approval, egm-design)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: a design-critic re-read of the text changed since round 3 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found its round-3 findings closed, the verifier and minter handles used alike in Technical Plans 2, 4 and 12 and Task 013-9, Architecture decision 0140's "Enforced by" matching, and the chart and configuration claims accurate.
Counts: Blocker 0, Major 1, Nit 6

1. **Major. Technical Plan 12, "Transport" and "The rule"; Task 013-8.** The module layout does not compile: `identity_mount`'s operations take an `IdentityOpenBao` whose field is private to the sibling `identity_ring`, the allowlist gives it no accessor and a non-private field is a negative fixture; and `read_versioned`/`write_cas` in the `transport` submodule must be at least `pub(super)`, which the allowlist does not say. Fix: one allowlisted `pub(in crate::openbao)` accessor `IdentityOpenBao::client`, the fixture becoming any other field or accessor; `read_versioned` and `write_cas` `pub(super)` and allowlisted at that level.
2. **Nit. Task 013-8 against Task 013-9.** Task 013-8 runs the shared table against the cookie codec and prints both adapters and `RefreshMac`, which Task 013-9 builds; and the fixture parenthesis also holds real tests. Fix: move those test parts to Task 013-9 and close the fixture list before the tests.
3. **Nit. Technical Plan 4, "Before the transaction"; Task 013-9.** "Redemption takes the minter" is not limited to `cli`, so a stale `cli-refresh` ring would refuse Studio sign-in. Fix: "a `cli` redemption takes the minter".
4. **Nit. Technical Plan 2; Task 013-1.** "`openbao.rs` adds no non-private item" contradicts Technical Plan 12's two items, and "three new rules" should be four. Fix: both wordings.
5. **Nit. "Lock order"; Technical Plan 6, maintenance.** The maintenance loop's session batches have no stated order and can deadlock with `revoke`'s. Fix: the same (tenant, id) order, in `lock_order.rs`.
6. **Nit. Technical Plan 12, "Transport".** The token-policy tests "against the pinned OpenBao container" would run against `start_openbao`'s plain-HTTP development server, where a development client skips lookup-self. Fix: name the `-dev-tls` pattern of `crates/infra-secrets/tests/openbao.rs` and the test target.
