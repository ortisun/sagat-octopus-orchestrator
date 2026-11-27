# Architecture review of design 044, round 5 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 9b64ac22698c34d847c9658b8a08b5a5c68814d2
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. All seven round-4 Majors
are closed in the design text (journal self-contained, reset inside a saga, publication against removal, design 045's
level, inbox fallback, decision 0106 drift, feature 022 flow control). The three Majors below are new.

## Major

1. The constitution 5.0.0 draft, principle IV, says a completion that ends the run is refused while an accepted signal,
   update or cancellation is unseen, and that a completion commits all or nothing; Decision 044-125 commits the other
   commands and defers only the terminal. Register a gap for the draft's wording and add it to the principle I and IV
   rows, or go back to refusing the whole completion.
2. Architecture decision 0108 decision 5 makes every schema-less code payload `pii` as a whole, which diverges from
   accepted Architecture decision 0033 (no schema means structure only for everyone) and extends the closed carrier set
   of Architecture decision 0036 decision 6. State that it refines both, with the reason; a test; widen the
   `classification_rules` check in Task 044-6.
3. The call-site check "by the compiler's resolution in a rustdoc JSON listing" cannot be built: rustdoc JSON lists no
   calls inside bodies and is unstable on the pinned stable toolchain. Use the source-text pattern of
   `managed_kind_sites.rs`, parsed with `syn`, over every method name, with a negative fixture; extend to every accessor.

## Nit

4. Task 044-31 and Decision 044-100 keep the old `PgRunTx::append_events` signature.
5. Node spec 1 KiB against 4 KiB in Architecture decision 0103 and a test name.
6. Lines still requiring `signal-inbox-v1` for every code version.
7. What `sagat_move_code_run` checks: the decision against section 7.3.
8. A completion below `S(k)`: `UNHANDLED_EVENTS` in the decision, `NONDETERMINISM_RISK` in the design.
9. Architecture decision 0107: terminal with an open saga refused against deferred; flow-control fields missing.
10. "Enforced by" lines naming tests no task creates.
11. Reconciler waking-kind parity test; `verify_run` does not compare the overlay.
12. `code_query_requests.args` and `answer` as `content`; Decisions 044-104 and 044-123 not marked superseded.
13. The stop-and-deadline principle row needs the workflow-task exemption in Task 044-1.
14. A reset copies `run.cancel_requested` without setting `code_cancel_deadline_at`.

Counts: Blocker 0, Major 3, Nit 11
