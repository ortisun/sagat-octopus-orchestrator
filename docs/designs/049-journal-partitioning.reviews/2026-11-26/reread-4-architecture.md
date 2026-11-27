# Targeted re-read 4 of design 049: architecture (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: bfa1714689a52996e821272fbeef657cfd69a28b
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the fixes of re-read 3, not a full review. The reviewer read files only and ran nothing.

Re-read 3: Majors A and B and Nits a to d resolved.

## Major

1. Design 053's Task 053-5 merges before design 049's Task 049-13, so when it lands no capability is permanent: the
   check's list is empty and its permanence test has nothing to run against. Fix: test the guard on a test-only
   capability in a disposable database; write the empty check `permanent = false`; Task 049-13 runs the test on
   `journal-buckets-v1`; ADR-0132 cites both. (Design 053's task list; not blocking design 049's partitioning.)
2. Design 053's first deactivation phase does not move the requirement epoch, and today's guard refuses every update, so
   processes never learn of it. Fix: the first phase moves the epoch with a history row; the guard allows only active to
   draining on a row that is not permanent; the deferred check covers updates; `is_active` treats `draining` as inactive;
   the test `a_draining_capability_stops_new_uses_at_the_next_heartbeat`. (Design 053; design 049's capability is
   permanent and never drains.)

## Nit

1. A check does not set a value: `activate_v2` must write `permanent`.
2. Where the requirement set comes from at registration is not named (`Registered::Serving` carries the epoch only).
3. ADR-0132's "Enforced by" omits the new tests of Tasks 049-5 and 049-13.
4. Design 053's lines break mid-sentence and run past the wrap width.

Counts: Blocker 0, Major 2, Nit 4

All were fixed in design 053 and ADR-0132 at `44d6a0b3`, with design 049's text unchanged: the test-only capability and
Task 049-13's run on `journal-buckets-v1` (Major 1); the epoch moved by the first phase, the guard's single update, the
deferred check, `draining` inactive and the test (Major 2); `activate_v2` writes `permanent` (Nit 1);
`instance_register_v2` and `instance_heartbeat_v2` return the requirement set (Nit 2); the ADR's "Enforced by" (Nit 3);
the wrapping (Nit 4).
