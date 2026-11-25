# Round 1 of 2026-11-24: what answers each finding

The complete run (the five stages of `egm-review` with direct dispatch, and the three repository reviewers) read
`f6fef1d6c9277d7f88b5b159177b0cbd7af5decd`. It ended with 8 Blockers and 80 Majors, so the design was rewritten and a
second round reads the new text. TEXT: the design, a contract, the data model or an architecture decision now says it.
CONTEST: the evidence says otherwise, with the reason. OPEN: not this design's to settle. Decisions 047-021 to 047-038 of
`specs/047-oauth-integration-hub/research.md` record the choices, under Owner decision 2026-11-24-4.

## The changes that answer most findings

| Change | Findings it answers |
|---|---|
| The fence on every writer, acquire comparing the access revision, activation checking tenant and state, pending revisions adopted, time checks, lease extension, expiry from the acquire (Technical Plan 4; Decisions 047-021 to 047-023) | correctness Blocker 1 and Majors 2 to 5, design Blocker 1 and Major 2, security 2 and 3, comprehension 3 |
| One redirect URI per registration, `iss` required where sent, the callback as one statement (Technical Plan 3; Decision 047-025) | security Blocker 1, design 4, correctness 7 |
| Reconnect keeps the account; `oauth:credential:manage` and the permission table (`contracts/rest-api.md`; Decisions 047-026, 047-027) | security 4 and 5, design 3, architecture 3, 9 and 10 |
| The OAuth vault: its own mount, policy, reference type and system actor (Technical Plans 1 and 2; Decision 047-028) | security 9, codebase Blocker 1 and Major 7, correctness 6, architecture 6 |
| Egress per the credential's namespace (Technical Plan 6; Decision 047-029) | codebase Blocker 2, architecture 11, design Nit 13 |
| Delivery to scripts through `ResolveDeclaredResources`, a growing redaction set, database-counted re-asks, minor gating (Technical Plan 5, `contracts/resource-delivery.md`; Decision 047-033) | codebase Blocker 3 and Majors 4 and 13, architecture 8, zoom-out 7, correctness Nit 12 |
| Imports as drafts with publication (Requirement 047-020; Decision 047-030) | architecture Blocker 2, security 5 |
| Feature 021's names, scheme, codes and catalogue; `RESOURCE_REAUTHORIZATION_REQUIRED` added (Technical Plans 5 and 9, `contracts/errors.md`; Decision 047-034) | zoom-out 2 and 5, architecture 3 and 5, design 11, codebase 5 |
| Hub type rules: all kinds, bound hosts, path encoding, effect class (Technical Plan 7, `contracts/hub-items.md`; Decision 047-031) | design 5 and 6, security 6, architecture Nit 12 |
| Revocation classes, the feed and the operator block (Technical Plan 7; Decision 047-032) | security 7, design 10 |
| Feature 028 owns the index code (Technical Plan 7; Decision 047-036) | zoom-out 10, architecture 7, codebase 6 |
| Two thresholds; the access-token pre-refresh dropped (Technical Plan 4; Decision 047-024) | zoom-out 3, comprehension 4, style 5, implementation readiness 4 and 5 |
| Contracts and data model written; tasks with files and tests; a builder and phase table (`specs/047-oauth-integration-hub/`) | implementation readiness 1 to 3 and 5 to 13, comprehension 7, codebase 13 |

## The other findings

| Report | Finding | Answer |
|---|---|---|
| architecture | Blocker 1 (Principle V) | OPEN: Technical Plan 12 proposes the third declared set; the coordinating session is asked to add it (pull request #275 or after); the design stays In Review until it merges (Decision 047-038). The 5.0.0 "Admitted scope" is quoted in the Constitution Check |
| security | 8 (constitution not merged) | OPEN, as above |
| comprehension | 1 (the Principle V condition undefined) | TEXT: Technical Plan 12 and Open Questions |
| zoom-out | 1 (two features in one; ten integrations) | TEXT in part: four integrations required, six as content tasks (Decision 047-035). CONTEST in part: the broker and the hub stay in one design because constitution 5.0.0 names design 047 for the hub and the hub's provider items are what the broker consumes; the phases keep them apart in delivery |
| zoom-out | 4 (offers) | CONTEST: kept and gated on Architecture decision 0074 (Decision 047-037) |
| zoom-out | 6 (constitution version) | TEXT: the Check names 5.0.0 and pull request #275; OPEN for Principle V as above |
| zoom-out | 8 (`INTEGRATION` atomic) | TEXT: one transaction so that a provider and its types never half-import |
| zoom-out | 9 (implementation detail) | TEXT: the SQL left the design for invariants; constants are in the requirements because they are acceptance bounds |
| design | 7 (throughput) | TEXT: per-registration limits, `Retry-After`, the keep-alive pass alone, `last_used_at` coalesced |
| design | 8 (Principle V) | OPEN, as above |
| design | 9 (alternatives inline) | TEXT: reasons in the document; the separate broker service weighed |
| design | Nits 12, 14, 15 | TEXT |
| correctness | 8 (recovery drivers) | TEXT: the revocation pass, the deletion transition, the state rechecked after a token request |
| correctness | 9 (import transactions and keys) | TEXT: one transaction per integration, `IDEMPOTENCY_KEY_REUSED`, `HUB_DIGEST_CONFLICT` |
| correctness | Nits 10, 11 | TEXT |
| security | Nits 10 to 14 | TEXT (callback hygiene, `aud`, withdrawal, identical `404`, the account label kept out of audit); scope narrowing per declaration stays Gap 047-001 |
| codebase | 8 to 12, Nits 14 to 16 | TEXT |
| comprehension | 2, 5, 6, Nits 8 to 12 | TEXT |
| structural | 1 to 3, Nits 6 to 9 | TEXT |
| structural | Nits 4 and 5 ("CUJ", "n/a") | CONTEST: the constitution's words-in-full rule refuses the abbreviations; the checker reads only the row prefixes |
| style | 1 to 6, Nits 7 to 13 | TEXT |
| implementation readiness | 7 (021 shipping first) | TEXT: feature 021 has only the `oauth` scheme, so no runner-visible client secret exists to migrate |
