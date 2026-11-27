# Targeted re-read 2 of design 049: architecture (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 577fc97ddfe498a93306340626faba2093e08676
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since acd8c828, not a full review. The reviewer read files only and ran nothing, and worked out the changes
from the fix notes of each re-read, having no shell.

Re-read 1's Majors 1 to 6 and Nits 1 to 4 are resolved.

## Major

1. "Permanent" has no mechanism and no check: nothing marks a capability permanent and Task 053-5 has no refusal test.
   Fix: `Capability::is_permanent()` or a `platform_requirements.permanent` column, refused by both phases, with
   `a_permanent_capability_refuses_both_deactivation_phases`, listed in ADR-0132's "Enforced by".
2. Design 053 makes every `control-plane` command call the handshake, executable by `sagat_app` only, while `sagat_retirer`
   may execute only the retirement functions: `journal retire` fails one rule or the other. Fix: grant the handshake to
   `sagat_retirer` in both designs.

## Nit

1. ADR-0132's Consequences say one transaction per month; the decision and the Goals say three.
2. The owner-only functions are missing from the `EXECUTE` list, and no task builds `authorize_abandon_deletes`.
3. No test for a dated root racing `journal_retire_start`, for erasure's lock order, or for the process-month rule (no
   injectable process clock).
4. Constitution Check row II omits Task 049-5, and `ServingGate` keeps no requirement set today.
5. Nothing keeps the `test-probes` feature out of the applications' builds.
6. Tasks 049-3 and 049-4 are missing from the merge order.
7. Design 053 states 049's drain check where it then refuses the deactivation.

Counts: Blocker 0, Major 2, Nit 7

All were fixed at `6f499948`: `platform_requirements.permanent` and `Capability::is_permanent()`, refused by both phases
with the test in Task 053-5 and ADR-0132's "Enforced by" (Major 1); the handshake executable by `sagat_app` and
`sagat_retirer` in both designs (Major 2); the ADR's wording (Nit 1); the owner-only functions listed and
`authorize_abandon_deletes` in Task 049-14 (Nit 2); the three tests and the injectable clock (Nit 3); Task 049-5 in row
II and `is_active` reading the heartbeat's requirements (Nit 4); the CI refusal of the feature (Nit 5); Tasks 049-3 and
049-4 first (Nit 6); "none, permanent" in design 053 (Nit 7).
