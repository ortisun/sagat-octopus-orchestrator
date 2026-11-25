# ADR-0108: Queries of code runs are answered from declared state by the engine, and worker queries are ephemeral

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6 for Owner question 044-011; OD-2026-11-24-4 for the acceptance)
**Refines / supersedes:** extends feature 014's declared state and ADR-0031 (declared result disclosure) to code versions
**Enforced by:** `a_declared_query_is_answered_with_no_worker`; `a_worker_query_expires_without_a_journal_event`; `security_workflow_as_code::a_worker_query_of_another_tenant_is_not_found`; the conformance scenarios `code-declared-state` and `code-worker-queries` (design 044, Tasks 044-21, 044-24, 044-26)

## Context

A Temporal query is answered by a worker replaying the workflow, and fails when no worker runs. Feature 014 already
answers declared CEL queries over committed declared state, with no worker, also on ended runs while their data is
retained, under `run:query` and audited. Some questions only the code can answer (an in-memory structure the author did
not publish).

## Decision

1. A code version MAY declare state and CEL queries in the IR's `state` form; the function MUST set the state by a
   command journaled `state.updated`; the engine MUST answer the declared queries exactly as for a graph run.
2. A code version MAY declare worker queries. A worker query MUST be a row of `code_query_requests`, never a journal
   event, expiring at the caller's timeout (at most 30,000 ms) plus 5 s, announced by a hint, leased by a session of the
   run's digest (the sticky one first), answered in at most 256 KiB, audited without arguments or answer. A handler that
   issues a command MUST fail the query, not the run.

## Considered options

1. **Declared queries by default, worker queries beside them** — queries that work without workers and after the end,
   and the flexibility of code where needed.
2. **Worker queries only (Temporal)** — lost: no answer without a worker.
3. **Journal every query** — lost: unbounded history for reads.

## Consequences

- Good: an operator can read a code run's business state with every worker down.
- Bad: two query paths; a worker query adds a short-lived table and a task kind.

## Evidence

Design 044, Technical Plan 8.3; `crates/application/src/declared_operations.rs`.
