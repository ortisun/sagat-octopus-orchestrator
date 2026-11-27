# Targeted re-read 1 of design 048: architecture (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 2d63c0bd0a3a4b614f445b2462918e30a96f57ba
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the
fixes of the full round (`architecture-full.md`, at 1e283510), not a full review. The reviewer read files only and ran
nothing.

Full round: 3 to 12, 15, 17, 19 resolved; 13, 14, 20 moot; 1 partly (still blocking: no profile for feature 032's
passive processes); 2 partly (feature 032's specification, contracts and task bodies unamended); 16 and 18 partly (Nits
2 and 3).

## Blocker

1. (Full round 1, partly.) Feature 032's passive processes must read their region's standby; add a passive observer
   profile and reword Requirement 048-020 and ADR-0130 decision 8.

## Major

1. Feature 032 sends `DATABASE_UNCONFIRMED` and `REGION_*` to clients, against this design's constant answer before
   authentication; ADR-0130 must record the `REGION_*` exception and 032's reason become operator-only.
2. `BEGIN READ WRITE` overrides ADR-0067 decision 4's read-only role guard; move the guard into the confirmation.
3. A dropped `WriteTx` returns an open transaction to the pool; close its connection on drop.
4. The read-write profile of `migrate` and `seed-dev` is exempt from `synchronous_commit=on` and the checks.
5. (Full round 2, partly.) Task 048-1 does not amend 032's specification, contracts and task bodies.

## Nit

1. ADR-0067's decision 6 has no lettered parts (the letters are FR-032-009's); its "Enforced by" and a sentence still
   state the superseded rule.
2. The lint on `set_config` fails on 14 existing uses; match the protected names after design 053's cut-off.
3. Design 053's Task 053-4 also needs Task 048-10 first.
4. `transaction_timeout` cannot be a startup option on PostgreSQL 16.
5. The spawned task, not the caller, classifies an unknown outcome.

Counts: Blocker 1, Major 5, Nit 5

All eleven were fixed at `7b5f574c`.
