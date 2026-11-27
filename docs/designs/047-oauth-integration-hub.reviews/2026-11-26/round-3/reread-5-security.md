# Targeted re-read of design 047, sixth pass: security (2026-11-26)

Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: 31914a472d9eb6ec174a55c3668c81b118fabcf6
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A confirmation of the fixes of
`reread-4-security.md`; the reviewer read the design, the diff of the fixes, the data model and
`crates/api-grpc/src/limits.rs`, and ran nothing.

## The previous findings

1. Answered (one window row per registration and per tenant, locked in their own statements; it needs READ COMMITTED and
existing rows: finding 3). 2. Partly: shedding, the process cap and giving the place back are fixed; the per-tenant cap
has a gap (finding 2). 3. Answered. 4. Answered. 5. Answered, but the fix causes finding 1. 6. Answered.

## New findings

1. Major. The rule "a holder whose vault write is refused because a tombstone won revokes the token it received" has no
   condition, while the adopt step also tombstones rows of live credentials: a holder paused past its lease, an adopter's
   tombstone, the holder's refused write, then a revocation that kills a live grant on a provider that returns the same
   value or revokes the whole grant. Fix: revoke only for disconnecting or disconnected credentials, otherwise discard
   under the abandoned-revision rule; a test.
2. Nit (suspicion). The admission layer chooses the class from the request path before authentication, so a per-tenant
   cap there would read an unverified tenant claim and let an attacker fill a victim's cap. Fix: the process cap in the
   layer, the per-tenant cap after the capability token is verified, as `TaskStreams::admit` (`limits.rs:89`).
3. Nit. A `SELECT ... FOR UPDATE` on a window row that does not exist yet locks nothing, so the first concurrent acquires
   on a new registration or opt-in both count. Fix: create the rows with the registration and at opt-in (or insert on
   conflict first), and state READ COMMITTED.

Counts: Blocker 0, Major 1, Nit 2

## The author's answers (all fixed; Decision 047-067)

1. Requirement 047-013 revokes only for disconnecting or disconnected credentials and otherwise discards under the
abandoned-revision rule; `a_holder_refused_by_an_adopters_tombstone_does_not_revoke_a_live_grant`. 2. The per-tenant cap
after verification, keyed on the verified tenant. 3. The rows created beforehand (or inserted on conflict first) and
READ COMMITTED stated in the data model.
