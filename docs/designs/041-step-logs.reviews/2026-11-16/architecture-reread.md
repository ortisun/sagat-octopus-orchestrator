# Architecture re-read of design 041 on the owner's approval path

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 46c42bbbfc2f2c1cc918970086990fb590020ce4
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a fresh `architecture-reviewer`, asked for a short targeted re-read of the changes of resolution.md section 12 (OD-2026-11-16-7), given its own `architecture-approval-3.md`; no shell. Recorded condensed.

Third-run Majors: 1 (one tenant fills `Class::Logs`) fixed; 2 (ADR-0085 against the design; no check of ADR-0084 decision 11) fixed.

## Blocker

None.

## Major

1. The script marker's own transaction has no lock timeout and no failure path: an error either reaches the runner for a report already kept (it retries as a repeat and the step's completion is delayed) or is uncounted; a wait on the tenant row delays the script step's completion with no bound. `begin_safely` with 200 ms, an outcome `failed`, never change the report's answer, a test that forces it.
2. ADR-0084 decision 9 ("and the page is empty") contradicts the new `complete` rule (D041-084), and no Note records it.

## Nit

1. NFR-041-SEC still says "the two accessors".
2. The ADR-0084 Note does not retire decision 5's "locked membership read" or the Consequences' queued secrets-lock taker.
3. The marker's port method holds the domain decision between its steps.
4. The completion bound leaves out markers.
5. ADR-0085's Enforced-by still cites the withdrawn T041-28.

Counts: Blocker 0, Major 2, Nit 5
