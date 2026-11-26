# ADR-0108: Signals, updates and queries of code runs reuse the platform's mechanisms, and every result a code run discloses is declared

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), under the owner's delegation (OD-2026-11-20-6 for Owner questions 044-011 and 044-014; OD-2026-11-24-4 for the acceptance and for Owner question 044-015); no person has reviewed it
**Refines / supersedes:** supersedes ADR-0031 decisions 8 and 9 (an update schedules no work; its handler is a declared expression) for code versions, and decisions 5 (the evaluator holds no credentials, network client or live lease) and 7 (a query observes one committed snapshot) for worker queries, whose snapshot is the function's state after a committed completion; extends feature 014's declared state and ADR-0031's declared result disclosure to code versions; makes the history export of a code run a payload read path of feature 016 (ADR-0028), the new decision ADR-0031 asks for before any reader of raw payloads; refines ADR-0111 (branch `docs/temporal-parity-readiness`, Proposed) for code versions: every declared signal is buffered and the activation is its consumer; refines ADR-0033's reading that a payload with no schema at all is structure only for everyone (Decisions 265 and 269 of feature 016), for code runs, as Decision 302 did for events; extends the closed carrier set of ADR-0036 decision 6 with the carriers of a code version's envelope
**Enforced by:** `a_signal_before_the_function_waits_is_delivered_at_the_next_activation`; `a_delivered_signal_is_consumed_by_the_completion`; `a_repeated_signal_key_is_answered_before_the_bounds`; `a_declared_query_is_answered_with_no_worker`; `an_update_result_outside_its_schema_fails_the_update_only`; `an_update_receipt_of_another_principal_is_not_found`; `a_lost_update_response_replays_after_authorization_is_checked_again`; `a_worker_query_is_claimed_by_polling_with_the_broker_down`; `a_worker_query_answer_outside_its_schema_is_refused`; `integration_workflow_as_code::security::an_activity_input_is_redacted_on_export_by_its_entrys_input_schema`; `...::a_side_effect_value_is_structure_only_without_the_reveal_grant_and_revealed_with_it`; `...::a_secret_field_reaches_the_workflow_lease_as_a_placeholder`; `an_open_update_fails_run_ended_when_the_run_ends`; `an_open_update_fails_run_continued_at_continue_as_new`; `integration_workflow_as_code::security::a_secret_field_is_never_exported`; the conformance scenarios `code-signals`, `code-updates`, `code-declared-state` and `code-worker-queries` (design 044, Tasks 044-21, 044-24, 044-26, 044-29)
**Required before acceptance:** ADR-0111 accepted with its refinement for code versions (Gap 044-017), and the measurements of design 044 Task 044-25

## Context

A code function reads signals when it chooses, so a signal must be kept until then; ADR-0050 decision 4 and ADR-0030
decision 17 forbid buffering, and ADR-0111 (Proposed) admits a bounded per-run inbox for versions that declare it. ADR-0031
requires a disclosure declaration and closed, validated result schemas, says an update schedules no work, and rules out a
raw payload reader without a new decision. A Temporal query needs a live worker; feature 014 answers declared expression
queries from committed state with no worker. Some answers exist only in the function's memory.

## Decision

1. A code version MUST declare its signals in ADR-0111's shape, refined here with a per-signal `maxBytes` and a closed
   `schema`, each buffered, and the publication of a version that
   declares signals MUST require `signal-inbox-v1`; the next activation MUST deliver the buffered signals in journal order; a signal MUST be consumed
   only when the function's handler or wait takes it, reported by the completion and marked in its transaction; a
   delivered signal not yet taken MUST stay pending, moved or discarded as ADR-0111 decides. After authentication,
   authorization and tenant resolution, the idempotency key MUST be answered before any bound.
2. A code version's update MUST follow `ADMITTED`, then `ACCEPTED` or `REJECTED`, then `COMPLETED` or `FAILED`, each a
   row of an append-only transition table (graph receipts stay immutable), with a closed `argsSchema`; a `202` MUST be
   answered only to a request that opts in with `acceptPending`: admitted
   durably, validated by the function when its admission event is applied, handled by the function (which may schedule
   work), and completed with a result validated against its closed `resultSchema` by the platform's validator, or failed
   (`RESULT_SCHEMA_VIOLATION` when the result does not fit), the run left alone. An `updateKey` MUST be scoped by the run and the admitting
   principal and by the address the client used (a run or a chain, so a chain-addressed key holds across
   continue-as-new, and a chain's signals keep their chain scope); a receipt and a repeated admission MUST be answered only within that scope (a holder of `run:operate`
   reads the list of every principal's update under the key), after authorization is checked again, and an unknown key
   MUST answer the same `404` as a missing one. The stop drain and the handoff of continue-as-new MUST finish every
   update that has not completed (`REJECTED` if admitted, `FAILED` if accepted, reason `RUN_ENDED` or `RUN_CONTINUED`),
   and an update to a run that is stopping or ended MUST be refused `409 RUN_NOT_MUTABLE`.
3. A code version MAY declare state and expression queries in the IR's `state` form; the engine MUST answer them exactly
   as for a graph run.
4. A worker query MUST be a durable row, holding its arguments (at most 64 KiB) and answer inline and charged to the
   tenant's content quota while it lives, claimed through `PollTasks` by a session of the run's current digest, answered
   from the function's state at its last committed completion with that completion's journal sequence, never a journal
   event, expiring at the caller's timeout (at most 30,000 ms) plus 5 s, its answer validated against its closed
   `resultSchema` and returned with `Cache-Control: no-store`, at most 64 in flight per namespace.
5. Feature 016's carriers MUST gain, under a new version of its classification rules, the schemas of a code version's
   envelope (activity and script `inputSchema` and `outputSchema`, signal and event schemas, a children entry's
   `outputSchema`); every other payload of a code run (a side-effect value, the declared state, update arguments and
   results, a continue-as-new input without a schema) MUST be classified `pii` as a whole, feature 016's refusal of
   annotations in `state.schema`, `argsSchema` and `resultSchema` holding for code versions, so a holder of the reveal
   grant sees it and every other reader receives structure only. This refines ADR-0033's reading that a payload with no
   schema at all is structure only for everyone, as Decision 302 of feature 016 did for events, because an operator
   repairing a run and a replayer need those values; and it extends ADR-0036 decision 6's closed carrier set. A field
   classified `secret` MUST reach a workflow lease as the export's typed placeholder, never its value: a code run cannot
   route a secret through the function, and an activity receives a secret only through its own entry's `secrets`. Each
   stored payload MUST be classified by the carrier of the version that accepted it, a step's outcome by the version
   that issued the step, so a move or a handoff never declassifies it; a children entry that drops a `secret`
   annotation of its listed child version MUST be refused at publication. A
   blob whose carrier schema has a `secret` field MUST NOT be delivered to a workflow, query or replay-check token, and
   the export MUST emit every blob as a reference only. The classification governs raw reads; the answers of declared
   queries and updates follow ADR-0031's declared disclosure.
6. `GET /runs/{runId}/history` MUST be granted by `run:read` and `history:export`, held by the admin role only by
   default, in a versioned format with a baseline; it MUST classify every payload by its schema (an activity input by its
   envelope entry's `inputSchema`, a business literal by its declaring schema or as `pii`), apply
   feature 016's redaction and reveal rules, never export a field classified `secret`, and be audited.

## Considered options

1. **Reuse ADR-0111's inbox, an asynchronous update lifecycle with declared results, declared and worker queries, an
   export under feature 016** — one buffered-signal mechanism for both models; queries without workers; no new way around
   the disclosure rules.
2. **A signal store of design 044's own** — lost: two buffered-signal mechanisms with two meanings for a refusal.
3. **Synchronous updates only** — lost: no handler could wait on an activity.
4. **Worker queries only (Temporal)** — lost: no answer without a worker.
5. **Export payloads as stored, or grant the export to developers by default** — lost: a classified value would reach
   readers ADR-0031 does not allow, and a wider default is a security choice the tenant should make.

## Consequences

- Good: an operator can read a code run's business state with every worker down; a team can replay real histories in its
  tests without seeing what it may not.
- Bad: two query paths; a short-lived table and a task kind for worker queries; a replay over a redacted history can end
  `INCONCLUSIVE_REDACTED`; the signal path waits for ADR-0111.
- Compatibility with runs in flight: graph signals, updates and queries keep their behaviour and their immutable
  receipts; code versions exist only after this decision; older update clients never receive a `202` they did not ask
  for. Operational cost: update waits re-read at most 64 times in 60 s and are bounded per API process.

## Evidence

Design 044, Technical Plan 7.4 and 8; `crates/application/src/declared_operations.rs`; Decisions 044-073, 044-084,
044-086 and 044-087; the architecture and security findings of rounds 1 and 2.
