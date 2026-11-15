# Review: feature 022, release 0.3.0 slice — architecture

- **Doc:** `specs/022-flow-control-step-policies/` (spec, plan, tasks, data-model, contracts, research) with ADR-0056 and ADR-0088
- **Reviewed-commit:** `1a21cbdcf9afc80a21b29c90f768df256dd66c64`
- **Reviewer:** `architecture-reviewer` (independent read-only agent, run by the coordinator on 2026-11-13)
- **Recorded by:** the slice's author, from the coordinator's relay of the findings; the reviewer's full text was not handed to the author. Findings the relay did not describe are listed as such.
- **Answered in:** the revision commit that adds this file (D022-019..034, ADR-0089 proposed).

| Finding | Severity | How the revision answers it |
|---|---|---|
| B1 — node-level `concurrency` keeps permit-less steps `READY`, filling the claim's window (`workers.rs`, oldest `READY` per activity, `limit.min(64)`), starving other keys and breaking FR-038-002; contradicts ADR-0088's rejection of grant-at-claim | Blocker | FR-022-019 revised; D022-019; ADR-0089 decision 1 (proposed, supersedes ADR-0056 decision 3); test T022-099 / SC-022-024; fallback O1169 |
| B2 — mixed fleet: ADR-0050 decision 9 requires registry activation | Blocker | FR-022-053; D022-023; T022-094 with T018-080 as prerequisite; SC-022-027. Deviation: use before activation is refused `FLOW_CONTROL_ACTIVATION_PENDING` (registry rule 4), `BINARY_CAPABILITY_MISSING` is the refused process's answer |
| M1 — dual-gated runs served by two routines | Major | FR-022-044, FR-022-052; T26/T27 of the plan; T022-097, T022-098 |
| M2 — pass design and throughput (strict oldest-first across shards) | Major | key-driven pass with reservation rule (D022-021, ADR-0089 decisions 3–4); SC-022-029 floor; T022-089 |
| M3 — readiness writers bypass the gate | Major | FR-022-051; D022-020; T022-096 |
| M4 — hot-path checks late and selector too narrow | Major | T022-091 (admission line), T022-076 moved to Phase 2, T022-092 (selector); NFR-022-007 |
| M5 — one admission port not enforced; schedule firing already on the tree | Major | FR-022-059; D022-031; T022-093; T022-064/086 amended; G1181 corrected |
| M6 — namespace counters undefined in the transactions | Major | FR-022-058; D022-026; `flow_control_counts`; T022-105; T022-029/081 amended |
| M7 — invariant 10 not checkable as written | Major | D022-032; data-model invariant 10 revised; T022-079 amended |
| M8 — paused or review-marked steps keep slots | Major | FR-022-040, FR-022-046; D022-028; T29; T022-100 |
| M9 — lease recording should not lock the window | Major | T23 note; D022-021 (hint early, never late) |
| M10 — digest key version per process | Major | FR-022-053; `flow_control_key_versions`; T022-095 |
| M11 — invariants verified after their writers | Major | D022-033; T022-022, T022-072, T022-027, T022-079 amended |
| N1 — status lines still say pending | Nit | plan, research, ADR revision notes; T022-066 note (rows not yet on `develop`) |
| N2–N5 — not relayed to the author | Nit | not addressed; G1186 |
| N6 — principle rows of the Constitution Check | Nit | plan, "Constitution Check (the revision)" |
| N7 — ceilings to be gated by the measurement | Nit | FR-022-047; plan "Ceilings, revised" (20 per second until SC-022-029) |
| N8 — D022-017/018 order | Nit | moved in `research.md`, text unchanged |

Counts: B2 M11 N8
