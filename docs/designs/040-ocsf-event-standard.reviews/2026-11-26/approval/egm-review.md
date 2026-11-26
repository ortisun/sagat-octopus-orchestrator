# Approval read of design 040: egm-review (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: 09e6543020101d0db05cabc1f7a853c95a0148b6
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the reviewer's output as returned, with this header added and machine paths made relative. The EGM review of design
040 ran its lens agents with the accepted EGM plugin (commit f0ba151b): stage 9, implementation readiness with codebase
details (`../round-1/egm-implementation-readiness.md`, 13 Majors) and stages 5 to 8 folded into one critique with the
zoom-out and comprehension lenses (`../round-1/egm-design.md`, 7 Majors), each run once as an agent with no conversation
history and spawned one at a time; then a re-read (`../reread-2/egm-implementation-readiness.md`, 3 Majors) and this
confirmation read of the fixes. The folding of stages 5 to 8 into one run is a departure from the plugin's five separate
stages, made to stay within the session's agent limit; it is stated here rather than hidden.

Counts: Blocker 0, Major 1, Nit 3

Previous re-read: 1 fixed, 2 fixed apart from leftovers (Nits 2 and 3), 3 fixed, 4 partly fixed (two stale references remain, Nit 1), 5 fixed, 6 fixed. Decision 040-019 added no Blocker. It added one Major, the `start` signature (finding 1).

1. **Major: The new `adopt_config` signature leaves `start: now` to guesswork, and one reading brings back the skipped backlog.**
   - Where: lines 538-541 and 604, the `start_at` column at line 577, Task 040-24. The text compares the configured `start` ("`now`, or a time") with the stored `start_at`, but the port takes `start: Option<OffsetDateTime>`. If `now` is turned into a timestamp first, every revision raise sees a "different" value and skips each tenant's backlog again. If `None` means `now`, the fresh-row case is undefined.
   - Fix: say how `now` is stored and compared (for example a start kind next to `start_at`), what `None` means, and which cursors "applying `start`" moves (the discovery cursor, every tenant's cursor, or both).

2. **Nit: Two stale cross-references are left from the earlier Nit 4.** Section 4 (line 377) still opens with "Two migrations." but then describes four files. Success criterion 040-002 (line 180) still says "contract job", while Task 040-6 says "contract suites".

3. **Nit: Task 040-11 (lines 793-795) no longer matches the new `AuditExportStore` trait.** It lists `pause`, which is not in the trait, and leaves out `set_state`, `record_failure` and `release`. No method says who resets `failures` and moves a `STALLED` sink back to `ACTIVE` (with `audit.sink_resumed`) after a success. `Committed` has no variant for it.

4. **Nit: The operator stream's guards are not carried through.**
   - Line 607 says every operator-stream write compares the configuration revision the engine adopted, but neither `SinkRef::Operator(tenant)` nor `claim` carries that revision.
   - Task 040-24 names refused-window columns on `audit_operator_export` that the storage comment (lines 579-581) does not list.
   - `discover`'s return type cannot report a discovery cursor from another cluster (`CURSOR_INVALID`).

Files read: `docs/designs/040-ocsf-event-standard.md`, `docs/designs/040-ocsf-event-standard.reviews/2026-11-26/reread-2/egm-implementation-readiness.md`, `specs/040-ocsf-event-standard/research.md` (line 28).
