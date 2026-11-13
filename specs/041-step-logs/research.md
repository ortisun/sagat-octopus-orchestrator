# Research: step logs per run (design 041)

Registers of the design [docs/designs/041-step-logs.md](../../docs/designs/041-step-logs.md). Append-only:
numbers are never reused or renumbered; a gap is closed by evidence, not by deletion.

## Decisions

The owner questions these rest on were all answered as recommended (OD-2026-11-13-14).

| ID | Date | Decision | Why |
|---|---|---|---|
| D041-001 | 2026-11-13 | A step log is a diagnostic record beside the journal, never execution state | The journal is never purged and is written under the run lock; log volume there would slow replay and the hot path |
| D041-002 | 2026-11-13 | Transport is a new unary RPC, `AppendStepLogs`, in worker protocol minor 1.7 | A field on `Heartbeat` ties a batch to the lease's liveness; one on `CompleteStep` adds statements to the budgeted completion |
| D041-003 | 2026-11-13 | A batch is accepted only while its attempt holds the lease, checked by a read of `steps` without a lock | The fence that already exists; a lock would put ingest in the queue of leases and completions |
| D041-004 | 2026-11-13 | A record's identity is (attempt, generation, sequence); the stream key includes the generation | Two holders of one attempt number can then never collide, whatever recovery does |
| D041-005 | 2026-11-13 | The log tables have no foreign key to `runs` or `steps` | The key's `KEY SHARE` lock conflicts with the engine's `FOR UPDATE` on the run row |
| D041-006 | 2026-11-13 | Lines are read in (storing transaction id, row id) order below the snapshot's `xmin` horizon | A page is final and a tail never skips, without a per-run sequence taken under the run lock |
| D041-007 | 2026-11-13 | The table is not partitioned; purge is by batched `DELETE` | Dropping a partition does not fire the quota triggers of ADR-0027, and retention differs per tenant |
| D041-008 | 2026-11-13 | Secret values are removed by the SDK, by value; the server applies the shape and field-name rules | Only the worker process holds the values; a server read of the secrets per batch would be a disclosure to audit each time |
| D041-009 | 2026-11-13 | The existing `/runs/{runId}/stream` does not carry log lines | It has one resume point (the projection sequence) and is granted by `run:read` |
| D041-010 | 2026-11-13 | The Rust SDK gains an activity logger in this feature | `sdks/SPEC.md` §6.3 promises one and capture needs it |
| D041-011 | 2026-11-13 | Sequences at and above 2^62 belong to the server's markers | A marker then never collides with a worker's record |
| D041-012 | 2026-11-13 | The streams' counters are served by a route of their own, not added to `/attempts` | The OpenAPI digest pins existing operations byte for byte |
| D041-013 | 2026-11-13 | The ingest does not take the tenant's `tenant_retained_usage` row before its commit unless usage plus the batch is within 2 MiB of the limit; then it takes it as the last statement before the insert, under a 200 ms lock timeout. Refines how D041-003's transaction treats quota | `sagat_quota_lock_log_usage` holds the row a completion with output needs to commit; the commit-time finalizer already enforces the limit exactly (review: architecture B1, correctness M3, security M2) |
| D041-014 | 2026-11-13 | Capture lives in the per-attempt logger object of each SDK; nothing is attached to a shared logger. Redaction uses the union of the secret values held by every attempt in flight in the process | Python's `_TaskLog` adapts the worker-wide logger, so a handler there sees every attempt (review: security B1, architecture M4) |
| D041-015 | 2026-11-13 | The ingest makes every check a report makes short of the run lock: `worker:execute`, not the database-runner or runner role, membership, activity binding, session (subject and minor, through a session token added to the request) and a lease that is `LEASED` or `RUNNING`, unexpired, on a run neither finished nor stopping. Extends D041-003 | A lease-token match alone survives revocation (review: security M1, N1) |
| D041-016 | 2026-11-13 | The server scrubs in the application, before sizing, with both existing scrubbers (`sagat_domain::scripts` and `sagat_telemetry::redact` shapes, without its 2,048-character cut) and one placeholder `[REDACTED]`; the SDK redacts keys, numbers, exceptions and language context as well as messages. Extends D041-008 | Review: security M3, M4, architecture N1 |
| D041-017 | 2026-11-13 | Order within a batch: lock the stream, drop known sequences, fold `DROPPED` ranges, then the attempt, run and tenant allowances, insert with `RETURNING`, counters from the returned rows. Fields are stored as canonical JSON `text` so that the size computed in Rust equals what `sagat_quota_measure` measures | Review: correctness M2, security N7 |
| D041-018 | 2026-11-13 | A `DROPPED` marker names the first and last sequence it covers and carries no count; the server counts the unseen sequences of the range, saturating and clamped at the attempt ceiling. `last_sequence` (worker range) and `next_marker_sequence` (server range, from 2^62) are separate columns. Markers hold no log-classified bytes. Run-ceiling and tenant-quota drops are recorded only on the stream row. Batches after `final` are refused. Refines D041-011 | Review: security M7, correctness M5, N1, N6, N7 |
| D041-019 | 2026-11-13 | The ingest's first statement assigns its transaction id; the tail ends only when the snapshot horizon has passed the `xmax` seen at the first terminal poll and a further page is empty. The horizon is computed in the statement that reads, on `xid8`. Refines D041-006 | Otherwise a batch that saw the lease live can commit after `end` (review: correctness M1, architecture M1, correctness N3) |
| D041-020 | 2026-11-13 | One lock order for step-log writers: secrets lock and membership, stream, run usage, content fence, log fence, tenant usage row. The script marker locks only its own `source = script` stream and does not charge `run_log_usage` | Review: correctness M4, architecture M2 |
| D041-021 | 2026-11-13 | The script marker takes its generation from the `step.leased` event `record_execution` already reads; generation and instance path are nullable for `source = script` | `record_execution` finds the step by (tenant, run, node) and has no instance path (review: architecture M3) |
| D041-022 | 2026-11-13 | Ordinary SDKs offer 1.7, which carries 1.6's `ResolveDatabase` on the wire; the server refuses it without `database:execute`, and `sdks/SPEC.md` §7.1 says so | Review: architecture M5 |
| D041-023 | 2026-11-13 | The lock and no-foreign-key tests live in `tests/integration/claim-transition-efficiency/`, which the selector picks for every `crates/infra-postgres/` change | Review: architecture M6, correctness M6 |
| D041-024 | 2026-11-13 | Alerts fire on shedding and purge backlog only; ceiling drops are the customer's code and are shown on the run | Review: architecture M7 |
| D041-025 | 2026-11-13 | An admin-only, audited `DELETE /runs/{runId}/logs` (`run:logs:delete`, `run.logs_purged`); expiry is computed at purge time from the last line and the retention in force | Review: security M8 |
| D041-026 | 2026-11-13 | In a namespace whose published versions carry a classification annotation (`namespaces.has_classified_schemas`, set at publication and backfilled), reading step logs also needs the PII reveal grant; a script execution's `log` is read under the same rule | Owner, OD-2026-11-13-18 (O041-012, O041-013) |
| D041-027 | 2026-11-13 | `AppendStepLogs` has its own admission class and permits with a per-tenant sub-cap; capture defaults to off until the purge pass is merged; a separate switch turns the read routes off | Review: architecture B1, N8, security N8 |

## Gaps

| ID | Date | Gap | Closed by |
|---|---|---|---|
| G041-001 | 2026-11-13 | A secret the step's code transforms before logging (encoded, split, shorter than 6 bytes), or one obtained outside `ResolveSecrets`, is not recognised; a hand-written 1.7 client can skip the SDK's redaction | Open as a limit; accepted by the owner as stated (O041-011, OD-2026-11-13-14) and written into ADR-0085 |
| G041-002 | 2026-11-13 | The lines an attempt wrote after its last successful flush are lost when its lease or its network is lost; the stream shows "ended without its final flush" and cannot say how many | Open |
| G041-003 | 2026-11-13 | A script step's lines are not live and not interleaved with the run's log; the run's log holds a marker that points to the script execution | Open; a follow-up, since O041-009 chose the marker (OD-2026-11-13-14) |
| G041-004 | 2026-11-13 | With no foreign key to `runs`, feature 020 must delete a run's log streams and usage row when it deletes the run | Open; owned by feature 020 |
| G041-005 | 2026-11-13 | A deployment whose protocol ceiling is held below 1.6 by a missing store proof captures no step logs | Open |
| G041-006 | 2026-11-13 | SC-041-007 is measured on one machine on one day | Open |
| G041-007 | 2026-11-13 | `record_execution` matches a script report by (tenant, run, node); a script step inside a collection or loop may get a marker without its iteration identity | Open |

## Owner questions

The options and the reasoning are in the design, section "Owner decisions needed". The owner answered all eleven
on 2026-11-13 ("Aceitar todas"), recorded as OD-2026-11-13-14.

| ID | Date | Question | Decision | Answer |
|---|---|---|---|---|
| O041-001 | 2026-11-13 | Is capture on by default? | On at `info`, with a namespace switch and a worker option | Answered 2026-11-13, option (a), as recommended: OD-2026-11-13-14 |
| O041-002 | 2026-11-13 | Who may read step logs? | A new `run:logs:read` for admin, developer and operator, not viewer | Answered 2026-11-13, option (a), as recommended: OD-2026-11-13-14 |
| O041-003 | 2026-11-13 | Are reads audited? | One `run.logs_read` per subject and run per 300 seconds | Answered 2026-11-13, option (a), as recommended: OD-2026-11-13-14 |
| O041-004 | 2026-11-13 | Where are lines stored? | PostgreSQL, one row per line, ordinary table, for 0.3.0 | Answered 2026-11-13, option (a), as recommended: OD-2026-11-13-14 |
| O041-005 | 2026-11-13 | The ceilings | 8 KiB a message; 1 MiB and 10,000 records an attempt; 16 MiB and 100,000 a run; the shared `tenant_log_bytes` | Answered 2026-11-13, the defaults and option (a), as recommended: OD-2026-11-13-14 |
| O041-006 | 2026-11-13 | Retention | `step_log_retention_days`, default 14, 1 to 365 | Answered 2026-11-13, option (a), as recommended: OD-2026-11-13-14 |
| O041-007 | 2026-11-13 | What is kept at the attempt ceiling? | The first lines, a marker and a count | Answered 2026-11-13, option (a), as recommended: OD-2026-11-13-14 |
| O041-008 | 2026-11-13 | The live tail | A new server-sent event route, `/runs/{runId}/logs/stream` | Answered 2026-11-13, option (a), as recommended: OD-2026-11-13-14 |
| O041-009 | 2026-11-13 | Script steps in the first delivery | A marker that points to the script execution | Answered 2026-11-13, option (a), as recommended: OD-2026-11-13-14 |
| O041-010 | 2026-11-13 | What "in 0.3.0" requires | Server, six SDKs, routes and Studio together | Answered 2026-11-13, option (a), as recommended: OD-2026-11-13-14 |
| O041-011 | 2026-11-13 | The limit of redaction (G041-001) | Accept it as stated, with ADR-0085 | Answered 2026-11-13, the limit accepted as stated, as recommended: OD-2026-11-13-14 |
| O041-012 | 2026-11-13 | Do step logs need the PII reveal grant in a namespace with classified schemas? | Asked after review | Answered 2026-11-13, "Exigir a concessão PII": yes, `run:logs:read` plus the grant there: OD-2026-11-13-18 |
| O041-013 | 2026-11-13 | Is the `log` of a script execution closed to viewers too? | Asked after review | Answered 2026-11-13, "Fechar ao viewer": yes, it needs `run:logs:read`; release notes say so: OD-2026-11-13-18 |

## Review of 2026-11-13

The architecture, security and correctness reviews of commit `2e90dbb8` are recorded in
`docs/designs/041-step-logs.reviews/2026-11-13/`. Their findings are answered by D041-013 to D041-027, G041-007,
the owner's OD-2026-11-13-18 (O041-012, O041-013) and the revised text of the design, ADR-0084 and ADR-0085.
The duty of G041-004 is registered in feature 020's research as G1618.
