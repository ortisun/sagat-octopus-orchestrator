# Contract: evaluating a policy

`evaluate(policy_version, samples, state, context, now) → Decision` is a pure function (`crates/domain/src/scaling/evaluate.rs`). The same function answers the evaluator loop and the dry evaluation of the management interface, so a recorded decision can be reproduced from its recorded inputs (FR-036-026, FR-036-027). Nothing in it reads a clock, a database or a random source.

## Inputs

| Input | Meaning |
|---|---|
| `c` | current replicas of the unit: the live replicas when known (the engine: live `scheduler_members`), else the last desired value, else the policy's `min`. The source is recorded. `c ≥ 1`. |
| `samples` | one `Sample` per signal the policy binds (value, state, age, coverage) |
| `state` | `recent` raw values with their times, `last_up_at`, `last_down_at` |
| `context` | `role_ceiling` (shards, partitions, or none), `partition_bound` flag, `budget_replicas`, overrides, `frozen`, `mode` |
| `now` | taken once per round from the database clock |

## Steps

Each step may change the value `v`. A step that changes it appends a `Limit {name, before, after}` to the record, in this order.

1. **Usable signals.** A signal is usable when its state is `AVAILABLE` and its age is within its freshness. If none is usable: `v = c`, outcome `HELD`, reason `NO_SIGNAL`, and the steps below are skipped (FR-036-043).
2. **Demand of each usable signal.** `DEMAND`: `d = ⌈value ÷ target⌉`. `UTILISATION`: `r = value ÷ target`; if `|r − 1| ≤ tolerance` then `d = c`, else `d = ⌈c × r⌉`.
3. **Raw.** `raw = max d` (the combine rule `MAX`). If any contributing signal has coverage below 1, the result is a lower bound.
4. **Stabilization.** If `raw > c`: `v = min(raw, every recent raw within the scale-out window)`; if that is `≤ c` the limit is `STABILIZATION_OUT` and `v = c`. If `raw < c`: `v = max(raw, every recent raw within the scale-in window)`; if that is `≥ c` the limit is `STABILIZATION_IN` and `v = c`.
5. **Coverage.** If the raw was a lower bound and `v < c`: `v = c`, limit `COVERAGE` (never scale in on a lower bound, D1720).
6. **Cooldown.** If `v ≠ c` and `now − last change < cooldown`: `v = c`, limit `COOLDOWN`.
7. **Step.** If `v > c`: if `now − last_up_at < scale_out.step.every_seconds` then `v = c`, else `v = min(v, c + scale_out.step.max_change)`, limit `STEP_UP`. If `v < c`: the same with `last_down_at`, `scale_in` and `max(v, c − max_change)`, limit `STEP_DOWN`.
8. **Bounds.** `lo = max(min, OVERRIDE MIN)`, `hi = min(max, OVERRIDE MAX)`; `v = clamp(v, lo, hi)`; limit `BOUNDS`, `OVERRIDE_MIN` or `OVERRIDE_MAX` according to what bit.
9. **Hold or freeze.** An active `HOLD` override sets `v` to its value (`OVERRIDE_HOLD`). An active `FREEZE` override or the global switch sets `v = c` (`FROZEN`).
10. **Safety ceilings, last.** `v = min(v, role_ceiling)` (`PARTITION_BOUND` when the ceiling is the partition count, else `ROLE_CEILING`); `v = min(v, budget_replicas)` (`DB_BUDGET`); `v = min(v, cost_ceiling)` (`COST_CEILING`). No override passes them.
11. **Outcome.** If `v = c` the outcome is `HELD`. Otherwise `OBSERVED` (mode observe), `RECOMMENDED` (recommend, advisory) or `APPLIED` (enforce, authoritative). The desired value of the policy's target is written whenever the mode is recommend or enforce, even when `v = c`, so that the provider always has a value.

**Unit desired capacity.** For each `(unit, kind, parameter, authority)` the desired value is the largest final value of the policies of that unit in the matching mode. A policy in observe contributes nothing.

**Database budget.** `share(unit) = ⌊(server_max − reserve) × weight(unit) ÷ Σ weight⌋`; `budget_replicas = ⌊share ÷ per_replica_budget⌋`. Validation refuses a policy whose `min` exceeds `budget_replicas`, and a `min` below the unit's availability floor (2 for `engine` and `control-plane` unless the configuration says otherwise). If configuration later makes the ceilings lower than the floor, the ceilings win and the condition `FLOOR_UNREACHABLE` is raised.

## Golden vectors

The same vectors are the property tests' fixed cases and the dry evaluation's contract tests. Unless stated: `DEMAND`, mode `ENFORCE`, `min 2`, `max 8`, `cost_ceiling 8`, `scale_out {step 4 every 0 s, stabilization 0}`, `scale_in {step 1 every 120 s, stabilization 300 s}`, `cooldown 0`.

| # | Case | Inputs | Result |
|---|---|---|---|
| V1 | lag scale-out | target 1,000; `c = 2`; backlog 5,200, coverage 1 | raw 6; no limit; **final 6**, `APPLIED`, reason `OK` |
| V2 | partition-bound | as V1 but backlog 12,000; `role_ceiling = 4` (partitions) | raw 12; `STEP_UP 12→6`; `PARTITION_BOUND 6→4`; **final 4**, reason `AT_CEILING` |
| V3 | scale-in stabilizing | `c = 6`; backlog 1,200 (raw 2); recent raws in the last 300 s: 6, 6 | `STABILIZATION_IN 2→6`; **final 6**, `HELD`, reason `STABILIZING` |
| V4 | scale-in step | `c = 6`; backlog 1,200; recent raws all 2 for 300 s; `last_down_at = now − 600 s` | `STEP_DOWN 2→5`; **final 5**, `APPLIED` |
| V5 | stale signal | `c = 4`; the only signal `STALE` | **final 4**, `HELD`, reason `NO_SIGNAL`, no limits |
| V6 | database budget | `max 16`, `cost_ceiling 16`; `server_max 200`, `reserve 20`, weights engine 2 / control-plane 1, 20 connections per engine replica (share 120, `budget_replicas 6`); `c = 4`; backlog 9,000 | raw 9; `DB_BUDGET 9→6`; **final 6**, reason `AT_CEILING` |
| V7a | utilisation inside the band | `UTILISATION`, target 0.6, tolerance 0.10; `c = 4`; value 0.62 (`r = 1.033`) | raw 4; **final 4**, `HELD` |
| V7b | utilisation outside | as V7a; value 0.9 (`r = 1.5`) | raw `⌈4 × 1.5⌉ = 6`; **final 6**, `APPLIED` |
| V8 | partial coverage | `c = 3`; backlog 100 (raw 1), coverage 0.75; scale-in window empty | `COVERAGE 1→3`; **final 3**, `HELD`, reason `PARTIAL_COVERAGE` |
| V9 | global freeze | `c = 4`; backlog 7,000 (raw 7); `frozen` | `FROZEN 7→4`; **final 4**, `HELD`, reason `FROZEN` |
| V10 | override MAX | `c = 2`; backlog 7,000; override `MAX 3` | `STEP_UP 7→6`; `OVERRIDE_MAX 6→3`; **final 3**, `APPLIED` |
| V11 | cooldown | `cooldown 60 s`; last change 30 s ago; `c = 2`; backlog 6,000 | `COOLDOWN 6→2`; **final 2**, `HELD`, reason `COOLDOWN` |
| V12 | frozen above a lowered ceiling | `c = 8`; backlog 8,000 (raw 8); `frozen`; `role_ceiling = 6` (configuration lowered after the policy was set) | the freeze keeps `v = c = 8` (no limit recorded, it changed nothing); `ROLE_CEILING 8→6`; **final 6**, `APPLIED`, reason `AT_CEILING` |

Worked check of V2: `⌈12,000 ÷ 1,000⌉ = 12`; the scale-out window is 0 and `recent` is empty, so `v = 12`; the step allows `c + 4 = 6`; the bounds `[2, 8]` leave 6; the partition ceiling 4 gives 4. Of V6: `⌊(200 − 20) × 2 ÷ 3⌋ = 120`, `⌊120 ÷ 20⌋ = 6`.

## Properties (checked by `proptest`, `SC-036-007`)

For every generated input:

1. `final ≤ min(role_ceiling, budget_replicas, cost_ceiling)` whenever those exist.
2. `final ≥ policy.min` unless a ceiling is lower than it (then `FLOOR_UNREACHABLE`).
3. A stale or unavailable signal never changes `final` (compare with the same input without that signal).
4. `|final − c|` never exceeds the step except by bounds or ceilings that apply to `c` itself.
5. A frozen policy never changes `final` away from `min(c, ceilings)`.
6. Evaluating the recorded inputs again yields the same decision (determinism, FR-036-027).
7. Scaling in on a lower-bound signal never happens.

## Partition targets (`assignment.rs`)

`partition_targets(P, M)` returns, for ranks `0..M`, `⌊P/M⌋ + 1` for the first `P mod M` ranks and `⌊P/M⌋` for the rest.

| P | M | Targets |
|---|---|---|
| 12 | 3 | 4, 4, 4 |
| 12 | 5 | 3, 3, 2, 2, 2 |
| 4 | 6 | 1, 1, 1, 1, 0, 0 (partition-bound) |
| 1 | 1 | 1 |

`plan(held, free, rank, targets)`: a holder with more than its target gives back its **highest** partitions beyond it; a holder with fewer takes the **lowest** free ones up to it. Example: 12 partitions, replica A holds all 12 and a second replica B joins (`M = 2`, targets 6, 6): A gives back 6 to 11; B takes 6 to 11 at its next round.
