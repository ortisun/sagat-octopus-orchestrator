# ADR-0108: Queries of code runs are answered from declared state by the engine, worker queries are ephemeral, and every result a code run discloses is declared

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6 for Owner questions 044-011 and 044-014; OD-2026-11-24-4 for the acceptance)
**Refines / supersedes:** extends feature 014's declared state and ADR-0031 (declared result disclosure) to code versions, and makes the history export of a code run a payload read path of feature 016 (ADR-0028), the new decision ADR-0031 asks for before any generic payload reader
**Enforced by:** `a_declared_query_is_answered_with_no_worker`; `a_worker_query_expires_without_a_journal_event`; `a_worker_query_answer_outside_its_schema_is_refused`; `an_update_result_outside_its_schema_is_refused`; `an_update_receipt_of_another_principal_is_not_found`; `security_workflow_as_code::a_history_export_redacts_classified_fields_without_a_reveal_grant`; `security_workflow_as_code::a_secret_field_is_never_exported`; `security_workflow_as_code::a_worker_query_of_another_tenant_is_not_found`; the conformance scenarios `code-declared-state` and `code-worker-queries` (design 044, Tasks 044-21, 044-24, 044-26, 044-29)

## Context

A Temporal query is answered by a worker replaying the workflow, and fails when no worker runs. Feature 014 already
answers declared expression queries over committed declared state, with no worker, also on ended runs while their data
is retained, under `run:query` and audited. Some answers exist only in the function's memory. ADR-0031 requires a
disclosure declaration and closed, validated result schemas for every result the platform returns, and rules out a raw
payload reader without a new decision. A code run's replay in a team's tests needs its history.

## Decision

1. A code version MAY declare state and expression queries in the IR's `state` form; the function MUST set the state by a
   command journaled `state.updated`; the engine MUST answer the declared queries exactly as for a graph run.
2. A code version MAY declare worker queries and updates, each with `disclosure` and a closed `resultSchema`. Their
   results MUST be validated by the platform's validator (design 043) before they are stored or returned, and returned
   with `Cache-Control: no-store`. An update receipt MUST be readable only by the principal that admitted it or by one
   with `run:operate`.
3. A worker query MUST be a row of `code_query_requests`, never a journal event, expiring at the caller's timeout (at most
   30,000 ms) plus 5 s, announced by a hint, leased by a session of the run's current digest (the sticky one first),
   answered in at most 256 KiB, audited without arguments or answer, under `run:read` and `run:query`, at most 64 in flight
   per namespace. A handler that issues a command MUST fail the query, not the run.
4. `GET /runs/{runId}/history` MUST be granted by `run:read` and the new `history:export` (held by the admin and developer
   roles), MUST apply feature 016's redaction and reveal rules to every payload, MUST never export a field classified
   `secret`, and MUST be audited.

## Considered options

1. **Declared queries by default, worker queries beside them, every result declared, the export under feature 016** —
   queries that work without workers and after the end, the flexibility of code where needed, and no new way around the
   disclosure rules.
2. **Worker queries only (Temporal)** — lost: no answer without a worker.
3. **Declared queries only** — lost: a long loop would publish its progress into the journal at every iteration.
4. **Journal every query** — lost: unbounded history for reads.
5. **Export payloads as stored** — lost: a classified field would reach anyone with two permissions, which ADR-0031
   forbids.

## Consequences

- Good: an operator can read a code run's business state with every worker down; a team can replay real histories in its
  tests without seeing what it may not.
- Bad: two query paths; a worker query adds a short-lived table and a task kind; a replay over a redacted history can end
  `INCONCLUSIVE_REDACTED`.

## Evidence

Design 044, Technical Plan 7.4 and 8; `crates/application/src/declared_operations.rs`; Decisions 044-050 and 044-067; the
security findings of round 1.
