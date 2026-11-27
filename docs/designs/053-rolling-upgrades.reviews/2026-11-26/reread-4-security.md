# Targeted re-read 4 of design 053: security (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 219282911bf5ef04342519de646154b946451ba8
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation of the
fixes of `reread-3-security.md` (all three resolved), not a full review. The reviewer read files only and ran nothing.

No Blocker, no Major.

## Nit

1. "Nothing else" for `sagat_retirer` is claimed checked by the catalog test, which sees only definer functions and
   their grantees, not table privileges, other functions or role membership; a later `GRANT sagat_app TO sagat_retirer`
   would pass every test. Fix: the test asserts the role's whole privilege set in both directions.

Counts: Blocker 0, Major 0, Nit 1

Fixed at the next commit.
