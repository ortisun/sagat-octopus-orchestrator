# egm-review re-read of design 013 for approval, round 3 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 4d9b589e
Reviewer: egm-review (re-read for approval, egm-design)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: a design-critic re-read of the text changed since round 2 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found its six round-2 findings closed and the `KeyRing` claim accurate.
Counts: Blocker 0, Major 1, Nit 5

1. **Major. Technical Plan 2, `RefreshTokenMac`, against Technical Plan 4 and Redeem step 6.** One constructor, `handle(key_id)`, cannot carry the three paths: redemption mints with no key id, refresh verifies under a possibly `RETAINED` key but mints under the active key, and logout verifies against a stale ring while minting paths refuse it. Fix: two handles obtained before `BEGIN`: `verifier(key_id) -> Result<MacVerifier, UnknownKey | Unavailable>` (in memory, never refused for staleness, `Unavailable` only when an unknown id's re-read fails) and `minter() -> Result<MacMinter, Unavailable>` (the active key, refused when the ring is stale or empty; `mint` returns the key id it used); redemption takes the minter, refresh both, logout the verifier; name this in Technical Plan 2 and Task 013-9.
2. **Nit. Technical Plan 4, "Before the transaction".** The answer to an unknown id when this interval's re-read is already spent is unstated; `invalid_grant` there would break Requirement 013-026. Fix: the `503` form with `Retry-After` set to the rest of the interval, token unspent; a test.
3. **Nit. Technical Plan 12, the background task.** The per-request re-read of a stale ring is unbounded during an outage. Fix: join the same single-flight read, at most once per interval; otherwise `503` at once.
4. **Nit. Technical Plan 12, the attempt ring; Task 013-8.** The attempt ring also rotates in two steps and drops keys by age, which a `KeyRing` cannot hold. Fix: one stored-document format and loader for both identity rings, `PENDING` meaning opens or verifies but never seals or mints.
5. **Nit. Technical Plan 11, "Errors", against Technical Plan 4, "Rate limit and answers".** The token endpoint's `429` and `503` bodies differ between the two plans, and `temporarily_unavailable` is an extension value at the token endpoint. Fix: Technical Plan 11 says the token endpoint answers `429` and `503` in the RFC 6749 body with `error=temporarily_unavailable` as an extension value and the reason in `error_reason`; the client reads `error_reason` before `error`.
6. **Nit. Technical Plan 12, Task 013-8, Architecture decision 0140.** `derive_purpose_key(key_id, label, info)` has one caller passing an empty `info`, and its encoding would be ambiguous. Fix: `derive_purpose_key(key_id, label)` with the label as `info`; add `the_secrets_ring_cannot_reach_the_derivation` to decision 0140's "Enforced by".
