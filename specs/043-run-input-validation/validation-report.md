# Validation report: run input validation (design 043)

Dated runs of the checks that are regression checks rather than proofs (D043-060, D043-103): the oracle property test of
the platform's validator against `jsonschema` 0.58.6, and the measurements of the work counter (G043-017, G043-030).
A new run is a new dated section; an old one is never edited.

## 2026-11-15, phase 1 after the review (`feature/043-validator`)

Machine: Apple M3 Max, 16 cores, macOS 26.3, rustc 1.100.0.

### Oracle property test

`oracle::the_platform_validator_agrees_with_jsonschema_over_the_subset` (`crates/infra-schema/tests/run_input.rs`):
768 generated cases, each a schema of the subset with two definitions and one to seven inputs, checked under `$schema`
absent, draft-07 and 2020-12. The generators include the u64/f64 boundary pairs (`18446744073709551615` and
`1.8446744073709552e19`, `9223372036854775809` and `9.223372036854776e18`, `±2^63`) and nodes that hold both `properties`
and `additionalProperties` (D043-103).

| Run | Verdicts compared | Skipped (`workLimit`, `maxDepth`, backstop) | Skip ratio | Disagreements |
|---|---|---|---|---|
| unoptimised test build, `--nocapture` | 9,087 | 0 | 0.000% | 0 |

The test fails above a 5% skip ratio and on the first disagreement. The seed is proptest's own per run, so another
run compares a different sample.

### Work per unit

From `measure_k_and_the_cost_of_a_full_budget` (release, `--ignored`): the dearest keyword unit is `const` at 7.5 ns,
`multipleOf` on a huge float 8.7 ns, `required` 5.8 ns and `properties` 6.2 ns; a full budget of 1,000,000 units costs
at most 7.5 ms. A pattern at the state cap (466 states, the most a pattern reaches before its 16 KiB program) costs
0.0032 ns per (byte × state) in the raw automaton with the prefilter off, and 0.0144 at 225 states of classes. The
details and the k per keyword are in G043-017 and G043-030 of `research.md`.
