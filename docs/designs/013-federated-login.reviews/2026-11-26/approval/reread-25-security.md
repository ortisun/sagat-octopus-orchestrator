# Targeted security re-read of design 013 for approval, round 25 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 41bb05ce0be898caf00138cb780df48191b9d8e1
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-24 fix (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found no raw read through the client's helpers, no reachable hidden submodule, the credentials policies refusing identity paths, and the re-exported ring items free of key material.
Counts: Blocker 0, Major 0, Nit 1

1. **Nit. Technical Plan 12, *Transport* and *The rule*; Task 013-8.** Where the `IdentityPath` component check runs is unstated, and an enum's variant fields are as visible as the enum, so any module can skip a checking constructor; the fixture refuses only a `String` variant, not other `str`-backed types. Fix: every operation checks every component when it builds the URL, before any request; the test builds variants directly with `..`, `a/b`, empty and 65-character components; variant fields only typed values (typed ids, `u32`, an enum of fixed names) or the fixture widened to any `str`-backed type.
