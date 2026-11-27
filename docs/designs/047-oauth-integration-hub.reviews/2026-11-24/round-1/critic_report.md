Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: f6fef1d6c9277d7f88b5b159177b0cbd7af5decd
Reviewer: egm-design (egm-review stage 7; the structural and style reports of the same stage are beside this one)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 1, Major 10, Nit 4

1. **Blocker.** When activation updates no row the design retires a rotated refresh token already stored in OpenBao, possibly the only valid one; only the gap from the provider's reply to the first write is unavoidable. Fix: the next holder adopts a pending unactivated revision (feature 017's intent); extend the lease while writes are slow.
2. **Major.** Activation checks neither tenant nor state and disconnect does not move the fence; a refresh finishing during `DISCONNECTING` leaves a live grant. Fix: disconnect moves the fence; activation requires `ACTIVE` and the tenant; revocation targets the latest refresh revision.
3. **Major.** Feature 021 expects a manage permission on the credential that 047 never defines; anyone with `resource:manage` reconnects a resource to their own account; connecting delegates the account to all `resource:use` holders unstated. Fix: `oauth:credential:manage`; audit a change of account; list the delegation as a risk.
4. **Major.** `iss` checked only when present can be stripped. Fix: `issRequired` from `authorization_response_iss_parameter_supported`.
5. **Major.** Mandated integrations without fixed hosts: Salesforce `instance_url`, Microsoft per-tenant token endpoints, Atlassian `cloudid`. Fix: controlled host parameters checked against a pattern, or narrower scope with gaps.
6. **Major.** A preset declares its own class; feature 021 says hub types inherit the operation's class; a mistake could mark a POST safe. Fix: narrow toward effect only; derive from the method; CI refuses weaker.
7. **Major.** No throughput model for token endpoints; the pass on one replica without limits; no per-registration rate limit or `429` handling; one tenant can exhaust a shared client; the hot row. Fix: per-registration limits, `Retry-After`, sharding, batched `last used`, a capacity estimate.
8. **Major.** Principle V "Pass with conditions" waits on someone else while Open Questions says none. Fix: an open question, or the 5.0.0 text quoted.
9. **Major.** Alternatives dismissed by pointing to an ADR; the in-process broker not weighed against a separate service.
10. **Major.** Revocation arrives only with a release's embedded index. Fix: a signed revocation feed or an operator block by digest; a stated time to revoke.
11. **Major.** Feature 021's revised names (`oauth` scheme) and codes (`CAPABILITY_UNAVAILABLE`, `OAUTH_BROKER_UNAVAILABLE`).
12. **Nit.** Deployment registration paths; registration lifecycle (rotation, offer withdrawal, deletion with live credentials).
13. **Nit.** Egress: the namespace policy intersected with the deployment's.
14. **Nit.** "Everything is additive" beside a forward-only rollback failing admitted runs.
15. **Nit.** Test-run refusal code; credentials that never expire or have no refresh token.
