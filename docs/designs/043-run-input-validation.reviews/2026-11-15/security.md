# Security review of design 043 (2026-11-15)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: ce2e10e2eab95102e658d370dcef1fcd4a48faeb
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept, numbered across the
report. The reviewer read files only and ran nothing.

## Major

1. **The doc does not say that trigger-now and a private rerun hide the field and the rule, and implies they show them**
   (lines 103-105, 117-121, 326, 331, 541-551; ADR-0090 decision 8). A caller with the trigger permission but no
   disclosure of the schedule's stored input would learn which property fails and how. The code already answers `/` and
   `other` (D043-144, D043-148). Fix: state it in FR-043-003, rows 8 and 13, Technical Plan 5 and the ADR.
2. **A schema the pre-check accepts can be too heavy to cache, so it is compiled on every start** (lines 196-201,
   219-221, 476-477, 510-520). Each pattern weighs at least 640 KiB (`pattern.rs`), a tenant holds at most 16 MiB of the
   cache (D043-105), an entry heavier than that is never kept, and the pre-check allows 32 patterns: 26 or more make
   every start (every event, every child) rerun the pre-check and the regex builds under a permit, outside the work
   counter and the backstop. NFR-043-PERF and SC-043-004 do not hold for such a schema. Fix: refuse a compiled weight
   above the share at publication, or lower the per-pattern weight or `MAX_PATTERNS`; test that the largest admitted
   schema is kept, and measure its compile time.

## Nit

3. The doc describes a graded-disclosure check for repeats that was not built (code always answers `Root`, D043-121,
   G043-040); write "always `/` and `other`".
4. A compile that panics is transient and never remembered (D043-106) and is not in the doc's `SchemaRefusal`; a schema
   that triggers a compiler bug is retried forever. Catch it, log it without the schema, remember it, or bound retries.
5. The inventory asks for the owner's credentials but only reads; say it runs as the runtime role.
6. "Prints no schema text" (line 642): it prints keywords and pointers, which are author text; cite the escaping (D043-111).
7. The permit wait queue has no bound and is scanned under one mutex on every grant; lines 404-406 contradict themselves.
8. The row 14 write check does nothing when no gate is wired (`schedules.rs`, D043-125), unlike D043-120's fail-closed rule.
9. The sentinel test of FR-043-014 does not cover the journal, the schedule action record, batch items, pending resets,
   the parent step's failure event and the inventory output.

Classes looked at with nothing found: tenant boundary, authorization and admission, disclosure in the validator's
output, injection and parsing, least privilege.

Counts: Blocker 0, Major 2, Nit 7
