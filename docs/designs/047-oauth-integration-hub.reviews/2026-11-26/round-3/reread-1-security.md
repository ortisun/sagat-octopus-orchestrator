# Targeted re-read of design 047 after review round 2: security (2026-11-26)

Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: 6f77483fa659fb2124837e3fa7eceaf3fa755f48
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A targeted re-read of the
changes since round 2 (approval rule of Owner decision 2026-11-25-2), not a full review. The reviewer read the diff, the
design, `data-model.md`, `timings.md` and feature 021's `resource-types.md` and `spec.md`, and ran nothing.

## Round-2 security findings

1. Answered (a check on `oauth_credentials`; authorization code only on deployment registrations). 2. Answered (feature
021's resolution checks; `resource:effect` to declare). 3. Answered in the text; it must still be merged into the
constitution, as the design states. 4. Answered (Requirement 047-018; FR-021-003 treats a host change as rebinding).
5. Partly (finding 2). 6. Partly (findings 3 and 6). 7. Answered (028's indexes and a 48-hour staleness alert).
8. Answered for providers; credentials and requests remain (finding 5). 9 to 16: answered (13: finding 7).

## Findings

1. Major (new). The sweeper revokes retired refresh revisions at the provider (RFC 7009); a provider that returns the
   same refresh value on refresh (HubSpot, Salesforce without rotation), or revokes the whole grant when any token is
   revoked (Google), loses the live grant, and a script can trigger this with `forceRefresh`. Fix: retired revisions
   destroyed in the vault only; the provider called on disconnect, or for an abandoned token whose value hash differs,
   only when the definition declares per-token revocation.
2. Major (round-2 finding 5, partly open). The store goes on after the lease lapses, but the revocation pass runs once no
   lease is held: a disconnect during the call, the pass revokes and records `REVOKED`, then the holder inserts a
   `PENDING` row, writes the vault and crashes; a live refresh token stays, no sweeper covers it, and the outcome says
   `REVOKED`. Fix: the sweeper revokes `PENDING` rows of disconnecting or disconnected credentials and stale ones; a late
   row downgrades the outcome and is audited; a crash test.
3. Major. `oauth_request_windows.in_flight` is never decremented: a holder that crashes or fails leaks its slot, and after
   4 leaks every refresh on the registration stops (on a deployment registration, every tenant). Fix: count in-flight
   requests as unexpired leases, or per-slot rows that expire.
4. Major. `oauth_deployment_offers` has no state predicate for opt-in and a table-wide UPDATE: a tenant administrator can
   opt into a `WITHDRAWN` offer, get the shared client back, and the withdrawal disconnect never runs. Fix: state
   predicates, a column grant, a refusal shaped like a missing offer, a test.
5. Nit. `(registration_kind, registration_id)` points at two tables, so no composite foreign key carries `tenant_id`. Fix:
   two nullable foreign-keyed columns, exactly one set; restate the client-credentials check on them.
6. Nit. The per-tenant share is half the registration's limit, so two busy tenants starve the rest; exchanges, userinfo
   and revocation calls bypass the windows. Fix: about 1/N, and every broker call through the windows.
7. Nit. `sagat_app` can call `hub_record_revocation` at any time. Fix: it only inserts blocks, never lifts, pinned
   `search_path`, a test.

Counts: Blocker 0, Major 4, Nit 3

## The author's answers (all fixed; Decision 047-061)

1. Retired and abandoned revisions destroyed in the vault only; the provider called on a disconnect, and for an abandoned
revision only with a different value hash and `revocation: "per-token"` (a new field of the provider definition, default
`grant`); `a_retired_revision_is_never_revoked_at_the_provider`. 2. Requirement 047-013 now has the sweeper revoke late
`PENDING` rows, downgrade and audit; `a_holder_crashing_after_a_late_store_leaves_no_live_token`. 3. In flight is the
unexpired leases and `oauth_broker_calls` rows, counted under the window row's lock; `a_crashed_holder_frees_its_window_slot`.
4. State predicates, a column grant, `404` for a withdrawn offer, a trigger keeping `WITHDRAWN` final;
`a_withdrawn_offer_cannot_be_opted_into`. 5. Two foreign-keyed columns, exactly one set; the check restated;
`a_credential_names_exactly_one_registration`. 6. 1 in flight and an even share per second for a tenant; every broker call
through the windows. 7. Inserts `INDEX` blocks only, pinned `search_path`, `EXECUTE` revoked from `PUBLIC`;
`hub_record_revocation_never_lifts_a_block`.
