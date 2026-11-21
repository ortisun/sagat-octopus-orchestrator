# Architecture re-review of the follow-up (`295b158b..40854f4d`)

Reviewer: `architecture-reviewer` (read-only), 2026-11-20. Ran nothing. (Condensed.)

**No Blocker and no Major.** All three earlier Blockers and three Majors are resolved in code and records (the pinned
digest exclusion without recomputation, `api.md:127`, the Studio types and their selection with a test, the one reveal
rule with its fixture, the engine composition test, G1199/D022-037/D022-057/G022-015 and runbook §1.2). No duplicated
work in `feature/022-sdk-builders`.

minor: m1 the control-plane start check, the switches-off case and a missing retiring version are untested; m2 the
switches-off record overclaimed (*resolved: the file is not read with every switch off*); m3 no composition test of
sealing under the active version; m4 the `group` grammar differs in three places (*canonical and fragment now `{2,62}`*);
m5 the contract's "such a caller" reads backwards (*resolved*); m6 the reveal rule has no row in
`architecture-principles.md`, and G022-015 is prose only (no `unreleased.md` entry, no release check); m7 a change of only
the canonical contract does not select `contract_developer_experience`, `contract_payload_classification`,
`contract_iceberg_writes` on a pull request (the owner's selection decision). nit: the longest-position test's instant,
the fixture's exact error count, the selector's docstring, the active version read without the helper.
