Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: e1d438b0ff4e17994feb29517a4d0cd2582b1349
Reviewer: egm-design (egm-review stage 7, round 2)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 8, Nit 6

Round 1: answered Blocker 1 and Majors 2 to 6, 8 to 10; not answered 7 (finding 4); in part 11 (finding 6) and Nits 12, 14, 15.

1. **Major.** Delivery checks only the publisher's `resource:use`; add the initiator's, `resource:effect` and the slots' secret access; decide whether `resource:effect` is always required for scripts and activities.
2. **Major.** Scripts and activities get a raw token with every granted scope; hub narrowing binds only the integration runner; activities run outside guarded egress; a broker-side proxy injecting the token is not weighed. Fix: a risk row and the alternative.
3. **Major.** A token without `expires_in` stays deliverable; nothing reports an executor's `401` to the broker; GitHub OAuth-app tokens never expire and have no RFC 7009 endpoint. Fix: expiring GitHub App user tokens or GitHub's grant-revocation call; a `401` feedback path; the residual stated.
4. **Major.** The per-registration limit lets one tenant starve the others; enforcement across control planes unstated; 200 ms polling by many waiters. Fix: a fair share per tenant; a database-backed limiter; LISTEN/NOTIFY or backoff; a capacity table.
5. **Major.** `invalid_client` sends every credential to reauthorization and rotation means a new registration. Fix: rotate secret revisions in place; pause the registration and keep its credentials.
6. **Major.** Two new step codes and top-level REST codes where the error catalogue is closed and feature 021 uses `details.reason`. Fix: reasons under existing codes; list every code; update `sdks/error-catalog.json`.
7. **Major.** Whether a security revocation also refuses delivery to runs already admitted. Recommendation: refuse delivery for the security class; a test.
8. **Major.** Nothing stops an operator from offering a client-credentials registration. Fix: offers only for the authorization-code grant.
9. **Nit.** ADR-0122 and ADR-0123 lag the design.
10. **Nit.** Both preconditions (pull request #275 merging, the Principle V wording) should be stated.
11. **Nit.** `CLOCK_MONOTONIC` stops during a virtual machine's suspension; use `CLOCK_BOOTTIME` or record it.
12. **Nit.** Thirty re-asks fail an activity that outlives about 30 token lifetimes; derive the bound from the lease and the expiry.
13. **Nit.** Say that a run fails once retries are exhausted; name the test-run refusal code.
14. **Nit.** The revocation feed has no staleness alarm or sequence.
