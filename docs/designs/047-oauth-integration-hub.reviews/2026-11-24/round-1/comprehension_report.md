Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: f6fef1d6c9277d7f88b5b159177b0cbd7af5decd
Reviewer: egm-comprehension (egm-review stage 6)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 7, Nit 5

**What this proposes**

From release 0.3.0, a feature-021 resource can hold an OAuth 2.0 credential, from a person connecting an account (authorization code with PKCE) or from a machine client (client credentials). A token broker in the control plane is the only component that talks to the provider's token endpoint; it keeps refresh tokens, client secrets and keys in OpenBao and refreshes under a fenced database lease. Integration runners, script sandboxes and SDK workers receive only the declared resources' non-secret settings and a current access token, through the all-or-nothing declared-secret delivery. A curated hub under `hub/` holds integrations as signed data (providers, resource types narrowing `http_api`/`graphql` with fixed hosts and presets, templates), validated in CI, imported as pinned versions or drafts, revocable by a later index.

**How the existing system works, per this document**

Feature 021 provides resource types, resources with secret slots in OpenBao and a closed set of operations run by an isolated integration runner, with client credentials only and the client secret given to the runner. Feature 028 designs a signed template index imported as drafts. Feature 017 is the secret authority with recoverable provisioning. Principle V allows an executor only the secrets declared by its immutable task definition. Scripts and activities that need an account today carry hand-copied long-lived tokens.

**Findings**

1. **Major.** Constitution Check row V rests on "Principle V's first declared set" and "the coordinating session confirms the 5.0.0 wording", neither defined. Quote the text; say what happens without confirmation.
2. **Major.** Five credential states without transitions; reconnecting a `NEEDS_REAUTHORIZATION` credential and a crash during it are undefined. Add a transition table.
3. **Major.** The disconnect-during-refresh test has no expected outcome; activation checks only the fence.
4. **Major.** Four freshness thresholds (60 s, 30 s, 5 minutes, 120 s) with no stated relation.
5. **Major.** Journey 1 records `oauth-broker` on a namespace; Requirement 047-023 puts it in the instance registry; who records it is unclear.
6. **Major.** Other features' concepts used without links or definitions (closed operation kinds, auth slot, two-phase capability resolution, recoverable provisioning, retired revisions).
7. **Major.** No task creates the hub tables; Task T047-15 names no crate; the crate-disjoint claim cannot be checked.
8. **Nit.** Whose `resource:use` is checked at publication, admission and delivery.
9. **Nit.** Where the account label comes from.
10. **Nit.** "Revalidated after provider input or output" is ambiguous.
11. **Nit.** Undefined terms and unlinked architecture decisions.
12. **Nit.** Task T047-20 should reference Non-functional requirement 047-002.
