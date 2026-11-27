# Targeted re-read of design 047 for approval: security (2026-11-26)

Doc: docs/designs/047-oauth-integration-hub.md
Reviewed-commit: 6a57e37db990c842ad1066f8bba1cf62f3e33d28
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; nothing of substance left out. The last targeted security
re-read after review round 2, on the approval rule of Owner decision 2026-11-25-2 (fix Blockers and every security
finding, re-read the fixes). Its history in this folder: `reread-1-security.md` (Major 4, Nit 3), `reread-2-security.md`
(Major 1, Nit 1), `reread-3-security.md` (Nit 1), `reread-3b-security.md` (Nit 1), then, after the answers to the EGM
stages, `reread-4-security.md` (Major 3, Nit 3) and `reread-5-security.md` (Major 1, Nit 2); every finding fixed before the
next pass. The reviewer read the design, the diff of the last fixes and the data model, and ran nothing.

## The previous findings

1. Answered. A holder refused by a tombstone revokes only for a disconnecting or disconnected credential, otherwise it
   falls under the abandoned-revision rule (Requirement 047-013, matching Technical Plan 4). The reviewer also checked the
   race where a disconnect starts just after the holder reads `ACTIVE`: no live token is left behind (a per-token
   provider with a different value hash still gets the revocation; with a whole-grant provider the disconnect's
   revocation also ends the dropped token; with the same value hash the dropped token is the active one, which the
   disconnect revokes).
2. Answered. The process cap stays in the admission layer; the per-tenant cap is applied after the capability token is
   verified, keyed on the verified tenant.
3. Answered. The window rows exist beforehand or are inserted on conflict first, at READ COMMITTED; that insert waits for a
   concurrent uncommitted one and the next statement's lock then finds the committed row, so the first two acquires on a
   new registration can no longer both count.

## New findings

None. Unauthenticated callers can still fill the process cap before verification, the same pattern as the existing
`discovery`, `lease` and `transfer` classes, not something this design adds.

Counts: Blocker 0, Major 0, Nit 0
