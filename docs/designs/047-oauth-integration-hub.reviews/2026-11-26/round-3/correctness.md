# Targeted re-read of design 047 for approval: correctness (2026-11-26)

Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: 6a57e37db990c842ad1066f8bba1cf62f3e33d28
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A targeted re-read of the
changes since review round 2 (approval rule of Owner decision 2026-11-25-2), not a full review; the reviewer read the
diff since `e1d438b0`, the design, the companions and the round-2 report and resolution, and ran nothing.

## Round-2 findings

Blocker 1: partly answered (the holder stores first and the sweeper uses tombstones; the revocation pass still has an
orphan case: finding 2). Answered: 2 to 6, 8 to 11, 13, 14; 7 answered but introducing finding 1; 12 answered for
`IS NOT DISTINCT FROM`, not for the expiry base (finding 5).

## New or remaining findings

1. Major. Writes that change no state (the stale mark, `forceRefresh`, `last_used_at`, `last_error_code`) also increase
   the fence, so an in-flight refresh fails to activate and its lease extension and clear are refused: a holder answered
   at 24 s, a stale mark at 5 s, the extension refused, the lease lapsing at 30 s, a waiter tombstoning the empty
   reference, the holder's write refused and the token discarded; at a rotating provider the grant is lost. Fix: only
   state transitions and the refresh steps increase the fence; the stale mark compares `access_revision = $reported`.
2. Major (plausible). The revocation pass destroys every revision, including a `PENDING` row whose reference was never
   written; destroying a key with no versions writes no metadata, so a later `cas=0` write succeeds, and a suspended
   holder then leaves a live token nothing tracks (the sweeper skips `DESTROYED` rows). Fix: tombstone before destroying an
   unmarked reference, or leave the row to the sweeper; a test against a suspended writer.
3. Major (plausible). A holder that loses to a disconnect revokes and retires, with nothing said if its revocation fails:
   the row is retired, the sweeper never revokes retired rows, the token stays live and the outcome may read `REVOKED`.
   Fix: on a failed revocation leave the row `PENDING` and marked for the sweeper.
4. Nit. A bound of 8 revisions enforced at insert would fail a store after the provider rotated the token. Fix: check at
   acquire, never refuse the store.
5. Nit. Expiry is dated from the store statement (overstating it after a delay); Decision 047-023 dates it from acquire.
6. Nit. The acquire reads `count_this_second` but never increments it or rolls `window_start`.

Counts: Blocker 0, Major 3, Nit 3

## The author's answers

All six are fixed in the data model, which the build reads (Decision 047-069), with four new tests; the design's own
sentences (the fence rule of Technical Plan 4, Requirement 047-013's destroy, step 5's retirement, the expiry base) are
restated by follow-up Task 047-26 before Tasks 047-7 and 047-8 are built. The Majors are recorded as settled by Owner
decisions 2026-11-24-331 to 2026-11-24-333 (delegate): fixed where the build reads them, the design text following,
because the text at the reviewed commit is the one the security re-reads cleared.
