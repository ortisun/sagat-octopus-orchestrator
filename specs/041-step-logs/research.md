# Research: step logs per run (design 041)

Registers of the design [docs/designs/041-step-logs.md](../../docs/designs/041-step-logs.md). Append-only:
numbers are never reused or renumbered; a gap is closed by evidence, not by deletion.

## Decisions

Each holds as long as the owner question beside it, if any, is answered as recommended.

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

## Gaps

| ID | Date | Gap | Closed by |
|---|---|---|---|
| G041-001 | 2026-11-13 | A secret the step's code transforms before logging (encoded, split, shorter than 6 bytes), or one obtained outside `ResolveSecrets`, is not recognised; a hand-written 1.7 client can skip the SDK's redaction | Open; put to the owner as O041-011 |
| G041-002 | 2026-11-13 | The lines an attempt wrote after its last successful flush are lost when its lease or its network is lost; the stream shows "ended without its final flush" and cannot say how many | Open |
| G041-003 | 2026-11-13 | A script step's lines are not live and not interleaved with the run's log; the run's log holds a marker that points to the script execution | Open; follow-up after O041-009 |
| G041-004 | 2026-11-13 | With no foreign key to `runs`, feature 020 must delete a run's log streams and usage row when it deletes the run | Open; owned by feature 020 |
| G041-005 | 2026-11-13 | A deployment whose protocol ceiling is held below 1.6 by a missing store proof captures no step logs | Open |
| G041-006 | 2026-11-13 | SC-041-007 is measured on one machine on one day | Open |

## Owner questions

The options and the reasoning are in the design, section "Owner decisions needed".

| ID | Date | Question | Recommendation | Answer |
|---|---|---|---|---|
| O041-001 | 2026-11-13 | Is capture on by default? | On at `info`, with a namespace switch and a worker option | Open |
| O041-002 | 2026-11-13 | Who may read step logs? | A new `run:logs:read` for admin, developer and operator, not viewer | Open |
| O041-003 | 2026-11-13 | Are reads audited? | One `run.logs_read` per subject and run per 300 seconds | Open |
| O041-004 | 2026-11-13 | Where are lines stored? | PostgreSQL, one row per line, ordinary table, for 0.3.0 | Open |
| O041-005 | 2026-11-13 | The ceilings | 8 KiB a message; 1 MiB and 10,000 records an attempt; 16 MiB and 100,000 a run; the shared `tenant_log_bytes` | Open |
| O041-006 | 2026-11-13 | Retention | `step_log_retention_days`, default 14, 1 to 365 | Open |
| O041-007 | 2026-11-13 | What is kept at the attempt ceiling? | The first lines, a marker and a count | Open |
| O041-008 | 2026-11-13 | The live tail | A new server-sent event route, `/runs/{runId}/logs/stream` | Open |
| O041-009 | 2026-11-13 | Script steps in the first delivery | A marker that points to the script execution | Open |
| O041-010 | 2026-11-13 | What "in 0.3.0" requires | Server, six SDKs, routes and Studio together | Open |
| O041-011 | 2026-11-13 | The limit of redaction (G041-001) | Accept it as stated, with ADR-0085 | Open |
