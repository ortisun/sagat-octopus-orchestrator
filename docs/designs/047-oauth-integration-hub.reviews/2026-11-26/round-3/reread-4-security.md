# Targeted re-read of design 047, fifth pass: security (2026-11-26)

Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: 820f4c7714a575c8d2e45d76ea92699b349107cc
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A security read of the
changes that answered the EGM stages of 2026-11-26, after `security.md` (at `4bc5767b`, one Nit, fixed). The reviewer read
that diff, the design, the data model and `crates/api-grpc/src/limits.rs`, and ran nothing. No finding on the
`SAGAT_STUDIO_ORIGIN` setting (fixed at start, never a request header), the host call in five languages (each token joins
the redaction set before the answer), the reuse of feature 021's checks (the initiator check is still added) or the
operator surface refused until Architecture decision 0074.

## Findings

1. Major. Concurrent acquires can exceed the in-flight limits: the count sits in the credential `UPDATE`'s `WHERE` and
   the window row is locked after it, so a statement that waited still counts with its earlier snapshot (two acquires
   both count 3); `window_start` in the key gives each second its own row, so acquires in different seconds never wait.
   Fix: one window row per registration and per tenant, locked by its own `SELECT ... FOR UPDATE`, then a later counting
   statement.
2. Major. The new limit class `resource_delivery` is process-wide and acts before authentication; with no shedding rule
   or bound, a tenant filling its windows holds the admission places kept for heartbeats and completions, so other
   tenants' leases expire. Fix: shed at or below `discovery`, its own cap, a per-tenant cap, never wait holding a place.
3. Major (suspicion, provider-dependent). The `code` comes from the browser: crafted codes answered with
   `unauthorized_client` three times in 10 minutes mark a registration `BROKEN` for the tenant, or for every tenant of a
   shared one. Fix: count only answers to refresh and client-credentials calls.
4. Nit. The offers grant lets the runtime move `refresh_stops_at` on a withdrawn offer. Fix: refuse every runtime change
   to a `WITHDRAWN` row; a trigger sets `refresh_stops_at`.
5. Nit. When the sweeper's tombstone wins, a holder with a token from the provider has its write refused and does not
   revoke it, while the outcome may say `REVOKED`. Fix: that holder revokes; a tombstoned row records `UNKNOWN`.
6. Nit. `oauth_delivery_attempts` still counts re-asks per fixed hour (about 60 fit around the boundary), and two
   contracts still say "per hour". Fix: keep up to 30 ask times per attempt.

Counts: Blocker 0, Major 3, Nit 3

## The author's answers (all fixed; Decision 047-066)

1. One window row per registration and per tenant, `window_start` an ordinary column, locked in their own statements
before the counting update. 2. The class is shed at the `discovery` threshold or below, with a process cap and a
per-tenant cap, and gives its place back instead of waiting. 3. Client errors to a code exchange fail only that
connection. 4. A trigger sets `refresh_stops_at` and refuses every runtime change to a withdrawn offer. 5. The refused
holder revokes; a tombstoned row records `UNKNOWN`. 6. The last 30 re-ask times, a sliding window, in the data model and
both contracts.
