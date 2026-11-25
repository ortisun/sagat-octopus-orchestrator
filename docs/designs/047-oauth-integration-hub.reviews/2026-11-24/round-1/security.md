Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: f6fef1d6c9277d7f88b5b159177b0cbd7af5decd
Reviewer: security (repository agent `.claude/agents/security-reviewer.md`)
Agent-blob: security-reviewer
Counts: Blocker 1, Major 8, Nit 5

Condensed by the author from the reviewer's output; every finding and severity is kept.

1. **Blocker.** Mix-up and code injection through one shared callback: one redirect URI for every provider and tenant and `iss` checked only when present (most providers never send it); a tenant-authored provider whose authorization endpoint forwards a victim to a real provider with a shared client id makes the platform send the victim's code and the verifier to the attacker's token endpoint, and the attacker completes it with their own `state`. Fix: a redirect URI per registration, refused when the path does not match the registration stored with `state` (RFC 9700 section 4.4.2); `iss` required when the definition says it is supported.
2. **Major.** A disconnect during a refresh leaves a live unrevoked refresh token: activation checks neither tenant nor state and disconnect does not move the fence. Fix: disconnect moves the fence; activation checks tenant and `ACTIVE`; a refused revision is revoked before it is retired.
3. **Major.** A slow OpenBao write, not only a crash, loses a rotated token: the writes have no deadline inside the lease. Fix: a deadline or a fenced lease extension; a barrier test holding the write.
4. **Major.** Reconnecting swaps the account behind pinned workflows: anyone with `resource:manage` reconnects an `ACTIVE` credential to their own account; authority is not rechecked at callback. Fix: a change of account subject requires a new resource revision; audit a digest of old and new subjects; recheck authority at callback and activation.
5. **Major.** Operations are not classified: no permission for hub import, reading credentials, disconnecting, offers, the operator block; importing a provider bypasses `oauth:client:manage`; imports create active versions where Requirement 028-041 allows drafts; items from operator indexes never pass the contract suite. Fix: a permission table; the write-time checks of authored items on imported ones.
6. **Major.** Path parameters of request presets can be injected (`../`, `%2F`, `?`, `#`). Fix: encode each variable as one segment, refuse dot segments, check the normalised path against the template; a refusal test.
7. **Major.** A revoked hub item stays admitted by default. Fix: a reason class; a security revocation blocks new admissions by default; unblocking is audited.
8. **Major.** The Constitution Check is written against text that has not merged: `docs/constitution.md` is 4.7.0; the Principle V reading is left to the coordinating session. Fix: do not approve until 5.0.0 is merged with the Principle V reading written in.
9. **Major.** "Readable only by the broker" is not enforced: feature 017's references are one mount under one control-plane token (`crates/infra-secrets/src/openbao.rs`), and ordinary delivery runs in the same process; registration secrets have no namespace. Fix: a separate mount or prefix with its own policy and a reference type the ordinary resolver cannot build; a test that secret delivery cannot address an OAuth path.
10. **Nit.** The callback leaks the code (URL, referrer, history, telemetry); the return path is unchecked.
11. **Nit.** `private_key_jwt`: `aud` must be the issuer only; one key per registration; prefer signing in OpenBao transit.
12. **Nit.** Withdrawing an offer or deleting a registration is unspecified.
13. **Nit.** A registration not offered must answer like a missing one; the account label (personal data) must stay out of audit, spans and logs.
14. **Nit.** Tokens carry every granted scope; a refresh may ask for a narrower `scope` per declaration (RFC 6749 section 6).
