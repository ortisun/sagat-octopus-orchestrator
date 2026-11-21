# What the reviews of the keys fix-up and G1198 changed (2026-11-20)

The three reviews of `f330ff66..295b158b` (`keys-fixes-security.md`, `keys-fixes-correctness.md`,
`keys-fixes-architecture.md`) and what was done about each Blocker, Major and minor. Decisions: D022-057 to D022-060 of
`research.md` (amended); gaps G022-013 to G022-015.

| Finding | Resolution |
|---|---|
| Architecture B1 (payload-classification pin) | `listConcurrencyKeys` excluded as `x-implemented: '022'`, like 017/018; the digest is not recomputed |
| Architecture B2 (`docs/reference/api.md`) | blessed after reading the diff (one row, 188 to 189 operations), rerun without the switch |
| Architecture B3 (Studio types) | `npm run api` (105 lines of `schema.d.ts`), `npm run check:api` passes; `select-test-targets.py` flags the Studio on a change of the canonical contract, with a test |
| Security Major 1 (instants link runs) | the listing's `oldestHolderSince` and `oldestWaitSince` are `null` with the label; asserted in `rest::a_key_label_is_shown_only_to_a_caller_who_may_reveal_personal_data`; the weaker channel recorded (G022-014) |
| Correctness M1 (cursor bound) | `MAX_CURSOR_PLAIN_BYTES = 384`, derived from the longest position (364); sealing refuses above it; unit tests of the longest derived group and of the bound |
| Architecture M1 (second reveal rule) | `reveal_grants::may_reveal_payload_unlocked`, called by the key reads; `reveal_rule_errors` keeps `payload_reveal_grants` read only in `reveal_grants.rs`, with a fixture |
| Architecture M2, correctness m2 (composition untested) | `admission::a_real_engine_with_the_ring_lists_the_gate_and_one_missing_the_active_version_does_not_start` (a real engine with `process_env` lists the capability; one missing the active version does not start, naming the variable and no key) |
| Architecture M3 (G1199, T022-104, D022-037) | G1199 amended (concurrency half met); D022-057 weighs listing before T022-104 and makes T022-104 a release and production-activation gate (G022-015, runbook); D022-037 annotated |
| minor: cursor version (security nit 1, correctness m4, architecture m2) | sealed under the version the database has active in the page's snapshot |
| minor: `Debug` prints the digest (security minor 1, architecture m9) | hand-written `Debug` of `RunPermit` and `KeyPosition`: the label only |
| correctness m3 (role half untested) | a demoted developer with a grant row left reads `null` |
| correctness m5 (key file with switches off) | the startup check runs only with a switch on |
| correctness m6 (`tasks.md`, `data-model.md`) | updated |
| correctness m7, architecture m12 (takeover, five starts) | recorded: G022-013, T022-110 |
| correctness m8 (invariant 12 for rate waiters) | added; "a RUN waiter's run is `WAITING`" not added: not verified that a queued start is `WAITING` in this build |
| correctness m9 (probes by name) | the schema the checks need is decided from `_sqlx_migrations`; a missing column is a failure |
| architecture m10 (compose file mode) | recipe: `chgrp 10001` and mode 0640, in the overlay and the runbook |
| architecture m13 (ADR-0093) | dated note |
| nits: zeroize capacity, switches parsed once, canonical `group` pattern, door rule renamed, count rule allows an alias | done |
| Not done | architecture m1 (narrow reveal port), m3 (no ring: documented in the contract's text), m5 (chart case in `contract_scalability_ha`), m6 (generic plan), m8 (`ConcurrencyKeyView.label` private), m11 (live conformance); correctness m1 (plan test reads "Rows Removed by Filter"), n1..n7 |
