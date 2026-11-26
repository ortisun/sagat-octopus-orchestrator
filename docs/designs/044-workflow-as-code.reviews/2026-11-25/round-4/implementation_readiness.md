# egm-review Stage 9 re-audit (implementation readiness and codebase details), round 4 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: ba5a3fdd02abfc57e03b76efc4a9fb2e237b5c45
Reviewer: egm-implementation-readiness, implementation-readiness standard, DetailedRange 1744-2197, code claims checked
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Of round 3's
implementation-readiness findings, 1, 3 to 8, 10 to 12, 14, 16 and 17 are closed and 2, 9, 13, 15 partly closed; of the
codebase-details findings, both Blockers and Majors 3, 5 to 14, 16 to 22 are closed and 4 and 15 partly closed.

## Blocker

1. Publication waits for `signal-inbox-v1`, which does not exist on this branch, and the inbox comes only with Task
   044-21, after the tasks whose tests must publish; make its builder a prerequisite of Task 044-3 or add the capability
   in Task 044-6; copy the inbox's schema here if the fallback stays.

## Major

2. Red tests need work from later tasks (publication before the migration; the waking test before signals and children;
   review causes before updates and the stop drain; the command-line export before its route; audit and metrics before
   Task 044-33; conformance before Tasks 044-21, 044-22 and 044-24).
3. `append_events(events, origin)` cannot be called from four files that write through a bare transaction, and journal
   rows carry more columns; specify a free function, its row type and its statements; graph counts stay exact.
4. The crash tests have no `test-support` feature, no binary, no barrier across processes, no script, and the gate runs
   without `--ignored`.
5. The server lane has no workflow worker for its tests; add a scripted decider in `tests/support/`.
6. Technical Plan 1.12 lists names, not signatures; TypeScript `engines` covers the whole package; `@types/node` lacks
   `DONT_CONTEXTIFY`; esbuild's dependency class and the `bin` entry.
7. A move job needs `batch_jobs.reason` (8 to 2,000 characters), the selector, `max_items` and the rate.

## Nit

8. The domain cannot compute the golden hash; the port's signature.
9. What produces the vectors; the canonicalizer before Task 044-5.
10. Two tests named for extension do not exist.
11. Other loaders of definitions (chain progress, loop progress, primitives, reset).
12. A source-text check cannot pick out `WorkflowDefinition::node` among 506 `.node(` calls.
13. The SDK copies of `worker.proto` and the generated catalog; catalog fields.
14. `sdks/AGENTS.md` says publishing is not an SDK operation.
15. The core move test touches sticky fields of a deferrable task; which columns the migration adds.
16. Missing tests (`INCONCLUSIVE_REDACTED`, some bounds, `SIGNAL_INBOX_FULL`); the replayer test belongs to the SDKs.
17. File lists for several tasks.
18. Gates: the architecture check's tests and four crates' unit tests; targets of the Studio and command-line tests.
19. The esbuild supply-chain review in Task 044-10.
20. Where the new task messages attach in the protocol.
21. `APPROVAL` already exists with `approval_request_id`.

Counts: Blocker 1, Major 6, Nit 14
