# Approval read of design 040: correctness (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: 09e6543020101d0db05cabc1f7a853c95a0148b6
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 0, Nit 3

Previous re-read: 1 fixed. 2 fixed: the scope is narrowed to the sink's own `audit.sink_record_refused`, and Requirement 040-009, the query and section 5 now agree. 3 fixed. 4 fixed. 5 partly fixed: the "where a sink starts" text now points at Gap 040-008, but `rebase` still overclaims (Nit 2). 6 partly fixed: `discovery_cursor_cluster`, `release` and Task 040-26's compare are in; the storage text trails (Nit 3). The changes for Decisions 040-018 and 040-019 bring in no new Blocker or Major.

1. **Nit. The port cannot tell `start: now` apart from a time** (lines 539-541, 577, 604). `start` reaches the store as `Option<OffsetDateTime>` and `start_at` is a timestamp, so the engine has to turn `now` into a time at each startup. That time never matches the stored `start_at`, so every revision raise re-applies the start and moves the cursors past the backlog. Task 040-24's test would catch this, but the signature points the builder at the wrong code. Fix: `enum StartAt { Now, At(OffsetDateTime) }`, stored as it was configured. Confirmed in text.
2. **Nit. `rebase` still claims "nothing is skipped"** (line 429). It uses the same 10-minute window that Gap 040-008 names for `rebase` too. Point it at the gap instead.
3. **Nit. The storage comment for `audit_operator_export` lists no refused-window columns** (lines 579-581). Section 5 (line 532) and Task 040-24 both say the operator streams have the refusal count. Add `refused_window_start` and `refused_in_window` to that comment.

Read: `docs/designs/040-ocsf-event-standard.md`, `docs/designs/040-ocsf-event-standard.reviews/2026-11-26/reread-2/correctness.md`, `specs/040-ocsf-event-standard/research.md` (Decisions 040-018 and 040-019).
