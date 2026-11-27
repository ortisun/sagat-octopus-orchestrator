# Targeted architecture re-read of design 013 for approval, round 4 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 2009034f5fc1e1140754aeb392bc738c7a0e4a09
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a re-read of the changes since round 3 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept.
Counts: Blocker 0, Major 1, Nit 6

1. **Major. Technical Plan 12, "The rule"; Technical Plan 2; Architecture decision 0140.** The `identity_ring` allowlist omits items the design needs: the sealing supertrait trips `private_bounds` and its usual `pub trait Sealed` is refused; the adapters must build a `KeyId` and a `Tag`, read a `Tag`'s bytes and the parts of `Sealed`; `KeyId` and `KeyState` need comparison and hash traits and `RingError` needs `Display` and `Error`. Fix: allow those items with negative fixtures that keep material out; state that only `RingKey`, `PurposeKey` and `RingDocument` bear material and stay fully private.
2. **Nit. Technical Plan 2 against Technical Plan 12.** The application's `MacVerifier`/`MacMinter` cannot hold an `infra-secrets` type. Fix: application trait objects implemented in `infra-secrets` holding an `Arc<IdentityKeyRing<CliRefresh>>`.
3. **Nit. Technical Plan 12, "The rule".** The list of `openbao.rs`'s public items omits the derived `Clone` of `OpenBaoConfig` and `OpenBaoSettings`. Fix: add it.
4. **Nit. Technical Plan 12, "Operations" and "Transport".** Nothing stops the secrets-mount client being passed to the ring or identity-mount operations. Fix: a newtype `IdentityOpenBao` built only from the identity settings, required by them, with a negative fixture.
5. **Nit. Technical Plan 11, "Alerts"; Technical Plan 4, Authorize.** `identity_signing_available` covers only the access-token key, so a stale attempt or `cli-refresh` ring may not alert. Fix: a closed `key` label, ring transport through the client's `measured` wrapper, the alert test extended.
6. **Nit. "Lock order" against `revoke`.** The batched random-value rewrite is not placed in the slots, so two runs can deadlock. Fix: lock each batch's session rows in (tenant, id) order in slot 4a, in `lock_order.rs`.
7. **Nit. Technical Plan 2.** "the `cli-refresh` string literals" is garbled. Fix: "the label literals".
