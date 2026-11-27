# egm-review re-read of design 050 for approval, round 1 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 9c893782
Reviewer: egm-review (re-read for approval, egm-design)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: a design-critic re-read of the text changed since the last full stages (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer confirmed the drain columns, `ApprovalListItem`, the quota route, the reset switch, the drivers route, `LANGUAGE_NOT_BUILT`, `refetchInterval`, `retryAfterMs`, `endpoints.journal`, the journey arithmetic, the cursor rule and the per-run flag.
Counts: Blocker 0, Major 4, Nit 6

1. **Major. Technical Plan 4, Task 050-6, Requirement 050-007.** The body loop of `structured_definition_with_policy` (`convert.rs:1107-1133`) raises the commonest body refusals (kinds not allowed in a body, destinations, a body without one `return`, `end` in a body, duplicates, cycles, unreachable nodes) with a bare id; the six named sites miss it. Fix: a seventh site; a test with an entrypoint id equal to a root id; the canvas's return problem is the server's.
2. **Major. Task 050-19.** `SAGAT_STUDIO_GREP` only adds `--grep`, so the existing job still runs the new journeys and exceeds its cap. Fix: tag the new journeys, exclude them with `--grep-invert`, verify the list.
3. **Major. Technical Plan 6 "Ended runs", Task 050-4.** The guard reads the request's status, which `decide` changes in memory; moved after `decide`, it never fires for an eligible caller, who then writes a receipt against a terminal run. Fix: the status captured before `decide`; a test for the eligible caller.
4. **Major. Technical Plan 6, Requirement 050-010, Architecture decision 0144 decision 5, Risks, Metrics.** The 2,000 bound counts rows that pass the predicates, so a page can read every pending row and return nothing, unseen by the histogram. Fix: restate the bound and measure examined rows, or seal the cursor so the bound can count every examined row.
5. **Nit. Technical Plan 6.** A second copy of the policy resolution exists in `crates/infra-postgres/src/versioning.rs::effective_policy`. Fix: route it through the same pure function.
6. **Nit. Task 050-14.** `frontend/e2e/scripts/perf-report.mjs` rewrites the report from one input. Fix: a second input and its sections.
7. **Nit. Technical Plan 5.** A sentence reads as if `retryAfterMs` were new. Fix: rephrase.
8. **Nit. Technical Plan 6, Task 050-4.** The probe's run-end paths are not named. Fix: list them.
9. **Nit. Technical Plan 4.** `refused_collection_operator_node` works on an unordered definition. Fix: the codec returns its index map.
10. **Nit. Technical Plan 3.** The name and the codec read different switches. Fix: derive the name from `reset_reapply_enabled()`.
