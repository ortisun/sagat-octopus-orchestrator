# Targeted security re-read of design 013 for approval, round 20 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: fc1cab5b742fdc16e79a29c7cc3403362feb4edb
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-19 fixes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found that the existing OpenBao client's handling of response bodies was the one point it could not verify; the author checked it: `crates/infra-secrets/src/openbao.rs` propagates no response body.
Counts: Blocker 0, Major 2, Nit 1

1. **Major. Technical Plan 12; Task 013-8.** `rotate` and `revoke` must rewrite the document carrying every key's material, but the allowlist gives the guarded module no document type or command operation, so the code lands in `apps/control-plane` with a second document struct holding `material` as text, which the audited "rotation cancelled" or conflict failure can format into audit and logs. Fix: the commands' read, rewrite and check-and-set inside the ring module, as allowlisted operations returning only key ids, states and versions; the test covers the commands' audit rows and log lines.
2. **Major. Technical Plan 12.** "Decodes `material` straight into a `RingKey`" points to a custom `Deserialize`, whose usual error puts the refused string, which for a truncated or hex key is the key, into an error the background reader logs every interval. Fix: any decode failure of `material` maps to a fixed error with no input, attributed to the key id; the test covers a material that is not base64 and one of the wrong length.
3. **Nit. Technical Plan 2; Technical Plan 12.** The allowlist forbids items the design needs (the types `IdentityKeyRing`, `KeyId`, `Tag`, `Verdict`, `Sealed`, the error; `Drop`, `Zeroize`), so a builder would loosen it ad hoc. Fix: list the allowed types and trait implementations, and state the traits always refused on the key newtypes.
