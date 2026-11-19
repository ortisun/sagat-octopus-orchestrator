# EGM stage 8 of the third approval run of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 6dceb5482d1a5a27b22be4018ed2df83e106137b
Reviewer: egm-review (stage 8)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Claude-Code: 2.1.325
Note: a fresh `egm:egm-zoom-out` agent, dispatched after stages 5 to 7 and given their report `egm-stages-5-7-approval-3.md`, the full document, the standard and the reading list. Recorded condensed; no severity was lowered. The agent counts only what it upheld.

## Merge

- Major: K3-1, the namespace secrets lock held by the ingest delays managed-database admission and every lease, heartbeat and completion queued behind its exclusive waiter (`crates/infra-postgres/src/execution.rs:1023-1027`, `execution/database.rs:113`); read the membership without that lock (an ADR-0084 Note for the owner), or restate the bound and measure it.
- Major: K3-2, remove the final batch's 1-second wait for a ticket, which pins a class slot (`crates/api-grpc/src/limits.rs:675-696`).
- Major: K3-6 with T3-4, O041-025 (a) is wider than the report predicate (`crates/application/src/worker.rs:2827-2830`); state the exact predicate and reuse it as a domain function.
- Major: K3-7, C3-1, S3-1, T3-2 merged: ADR-0085 against the design (trigger, backfill, read switch, `mustNotLeak`) and Constitution V; give ADR-0085 its own Note in T041-33.
- Major: S3-2 with T3-3, G041-017 is an open principle V gap missing from the Check; put it to the owner for written acceptance.
- Major: C3-2, derive a script item's state from the execution; the marker stream only affects the whole-run tab.
- Major: K3-8, add `(tenant_id, stream_id, stored_xid, sequence)` and state how the Studio pages through empty pages.
- Major: Z3-2, the exact run ceiling; ask the owner before the spike to drop it (recommended) or take the unlocked sum, which makes O041-027 and O041-020 moot.
- Major: Z3-3, write the marker in a short transaction of its own after the record commits; removes the `take_log_prefix` variant and a lock-order participant; needs a Note on the Amendment.

## Merge partly

- Major: Z3-1, make the attempt ceilings constants, keep the retention and one capture switch; check whether O041-005 decided tunability.
- Major: K3-3, drop a record before formatting or redacting when the buffer, the process bound or `FINAL_ONLY` already refuses it; reject redaction on the sender.
- Major: K3-4, a risk row and a saturated-ingest workload in T041-31; reject an SDK breaker for now.
- Major: K3-5, a risk row, runbook sizing and an explicit `tenant_step_log_bytes` in T041-31 and T041-9; reject a new tenant signal.
- Nits (downgraded): Z3-4 (detail follows the repository's practice; shorten T1 to T10 to invariants), T3-1, T3-5, T3-6; O041-026 missing only from the pass sentence; the "Answered" history.

## Reject

- Shared tail caps; stream-row churn; sender-side redaction and the breaker.

## Notable Nits merged

- "Usage row" for two locks; FR-041-002 against script markers; the capacity rate against the spike's two batches a step; T041-28 before T041-11; T041-2 and the header date; "five stages"; ADR-0027 not linked; `isolated_server` duplicates `start_server`; re-enabling capture waits for re-registration; SDKs and the credential shapes; a process serving two tenants; the audit per poll; rollback with a quota row.

## Missed

- Major: the per-batch work (about 27 round trips by reading) is not stated and is likely too heavy for the capacity rule; state it, and make the simplifications above before T041-31.
- Nit: a multi-pattern matcher may need a third-party dependency.
- Nit: building the classified index blocks publication during the upgrade; not in the Risks table.

Counts: Blocker 0, Major 14, Nit 21
