# Review: feature 022, release 0.3.0 slice — correctness

- **Doc:** `specs/022-flow-control-step-policies/` (spec, plan, tasks, data-model, contracts, research) with ADR-0056 and ADR-0088
- **Reviewed-commit:** `1a21cbdcf9afc80a21b29c90f768df256dd66c64`
- **Reviewer:** `correctness-reviewer` (independent read-only agent, run by the coordinator on 2026-11-13)
- **Recorded by:** the slice's author, from the coordinator's relay of the findings; the reviewer's full text was not handed to the author.
- **Answered in:** the revision commit that adds this file.

| Finding | Severity | How the revision answers it |
|---|---|---|
| B1 — readiness writers bypass the gate (operator retry in `state_effects.rs`, `settle` in signal/approval/update/timer/child, the retry pass in `maintenance.rs`, unpause) | Blocker | fail-safe reducer mapping and one adapter at `persist` (FR-022-051, D022-020, ADR-0089 decisions 1–2); invariant 11; one test per source (T022-096, SC-022-025) |
| B2 — dual-gated runs can half-grant or leave a waiter row on timeout | Blocker | one grant and one timeout routine (FR-022-052, D022-021/022, T26/T27); T022-097, T022-098, SC-022-026 |
| M1 — pass throughput | Major | key-driven pass, reservation rule; SC-022-029 |
| M2 — starvation of a dual-gated head | Major | reservation rule (FR-022-052) |
| M3 — step hoarding of permits by granted, unleased steps | Major | permit held from grant; holders include granted-unleased (FR-022-019, FR-022-044) |
| M4 — invariant 10 intervals | Major | per-grant half-open `(t_e − period_e, t_e]` (D022-032) |
| M5 — clock read and purge margin | Major | `clock_timestamp()` after the window lock (NFR-022-008, D022-027); purge margin; `libfaketime` test; G1184 |
| M6 — paused and review-marked steps | Major | return on leaving `READY` other than by lease (FR-022-046, D022-028, T022-100) |
| M7 — grant attempt identity | Major | grant names `attempt() + 1`; lease recording accepts that attempt only (FR-022-043, D022-029) |
| M8 — timeout precedence for dual gates | Major | rate first; one `wait_timed_out` plus one `dequeued` (FR-022-052) |
| M9 — child starts on a full buffer | Major | typed retryable child-node failure, no run (FR-022-055, T022-102) |
| M10 — per-key ceiling for step waiters | Major | run waiters only; step waiters bounded by `active_runs` × gated nodes (FR-022-042, D022-024) |
| M11 — SC-022-017 does not exercise the pass | Major | second key saturated; keys and tenants beyond one pass's budget (SC-022-017 revised) |
| N1 — status consistency | Nit | as architecture N1 |
| N2 — not relayed to the author | Nit | not addressed; G1186 |
| N3 — a grant after the deadline | Nit | grants skip past-deadline waiters (FR-022-042) |
| N4 — how a dual-gated run counts | Nit | once in `queued_runs` (FR-022-058) |
| N5 — task-ready hint for a buffered step | Nit | suppressed (FR-022-051) |
| N6 — continue-as-new inserts | Nit | asserted never to see a gated definition (FR-022-059, T022-093) |
| N7 — `live_grants` after a purge | Nit | decremented by rows actually deleted (data-model revision, T022-103) |

Counts: B2 M11 N7
