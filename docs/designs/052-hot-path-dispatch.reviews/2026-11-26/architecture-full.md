# Architecture review of design 052, full round of 2026-11-26

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: d734e0266bc4333e3e2cf8944a37034d296ab278
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only,
ran nothing, could not list branches, and did not repeat the reports beside this one.

## Blocker

1. The bound counts probes skipped for reasons unrelated to contention (a definition this build cannot read, gap G460;
   a protocol minor too high; refusals), so in a rolling upgrade the older process's polls end on the oldest unreadable
   runs and serve nothing ("a rolled-back reader keeps working"). Fix: count only contention (`begin_claim` returned
   none, or the step was leased meanwhile); a test of 9 unreadable runs before 1 readable; ADR-0133's Compatibility.

## Major

1. The registry cannot be built in the application crate as designed: `tokio` has only `rt` there, and ADR-0092's
   check refuses more. Fix: a registry without a runtime (counters behind a std mutex and wakers, or a port); parking
   and the 10 ms timer in `crates/api-grpc`.
2. The wake is a second internal message path outside the outbox (principle VII, ADR-0002, ADR-0069), unrecorded, and
   absent where the transport is Redis (the transports are exclusive). Fix: ADR-0134 refines ADR-0002 and ADR-0069 and
   states the wake carries no record and is never the only signal; a port on the selected transport (core NATS, Redis
   `PUBLISH`); `worker-runtime-protocol.md` in Task 052-1.
3. The wake subject is an undocumented, unversioned wire contract exchanged across releases. Fix: an internal, unstored
   channel in the AsyncAPI file under the baseline check, a reader rule, the permission in Helm and Compose.
4. A best-effort publish can block the request path while the broker is down (a full client channel). Fix: `try_send`
   into a bounded queue drained by one task; drop and count; a test with the broker stopped.
5. Wakes go to streams that cannot take work (a full session parks like any other), so the cap can spend a wake on a
   full stream, and under saturation each full stream claims on every commit. Fix: a full or drained stream does not
   register for wakes; the cap chooses among registered streams; a test.
6. Returning a refused stream to parking changes ADR-0029 decisions 3 and 7 and `stream-completion.md` rule 7 without a
   record. Fix: keep ending the stream on refusal, or record and amend.
7. ADR-0133 exceeds Owner decision 2026-11-12-3's limits ("within FR-038-004's", "it never removes a candidate") and
   conflicts with FR-019-013 (rotate only within one priority). Fix: cite it with a delegate row; the spread within one
   priority band; a gap row of feature 019; annotate FR-038-004 and `poll_order.rs`.
8. Rules without failing checks: every test holding with the wake lost; the core-publish prefix; wake keys never as
   labels; nothing published on a failed commit. Fix: a CI configuration with the wake off; unit tests; the telemetry
   vocabulary; rows in `architecture-principles.md`.

## Nit

1. Design 053's Requirement 053-006 cites Requirements 052-001, 052-004 and 052-005 in their earlier meanings; withdraw
   the old identifiers or renumber; never reuse.
2. ADR-0133 does not weigh design 038's own "fetch again after k skips", nor jumping to another offset instead of ending.
3. The process-local state (the spread flags, the per-key counters) has no size limit.
4. The settings' documentation, Helm, Compose and chart test are missing from the change table.
5. No span for the wake's publish and receipt, and the woken claim unmarked.

Counts: Blocker 1, Major 8, Nit 5
