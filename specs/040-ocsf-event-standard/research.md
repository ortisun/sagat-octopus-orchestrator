# Research: OCSF as the standard schema for security and audit events (design 040)

The registers of design 040 (`docs/designs/040-ocsf-event-standard.md`). Append-only: an entry is never renumbered;
a later entry may amend an earlier one by naming it.

## Decisions

| Id | Decision | Date |
|---|---|---|
| D040-001 | Pin OCSF 1.10.0, the version `https://schema.ocsf.io/api/version` reported on 2026-11-26. The first draft's 1.9.0 is older, and 1.9.0 deprecates User Access Management (3005) in favor of User Management (3007), which this mapping uses. | 2026-11-26 |
| D040-002 | Five classes cover the 127 actions: API Activity 6003, User Management 3007, Entity Management 3004, Datastore Activity 6005, Detection Finding 2004. Scheduled Job Activity 1006 was considered for schedules and rejected: it requires `device` and `job`, and the platform has no device to name, so filling it would invent data. | 2026-11-26 |
| D040-003 | Profiles: `trace` on every event (the trace id is `trace.uid`), `security_control` on findings. No `cloud`, `host`, `container` or `datetime` profile: declaring one makes its attributes required. | 2026-11-26 |
| D040-004 | `src_endpoint` is filled with the serving surface (`svc_name` `rest`, `grpc`, `engine`), because the audit record does not store the caller's address (Gap 040-001). | 2026-11-26 |
| D040-005 | The stream reads `audit_records` by `(tx_id, id)` below the snapshot's `xmin`; `tx_id` is an `xid8` column added nullable and then given the default `pg_current_xact_id()`, which rewrites no row. | 2026-11-26 |
| D040-006 | A sink stalls and never dead-letters: the audit table is the backlog and is kept forever, so a cursor never becomes invalid. | 2026-11-26 |
| D040-007 | Batches of at most 500 events and 1 MiB; a 10-second request timeout; back-off from 1 second to 5 minutes with jitter; `STALLED` after 5 consecutive failures; at most 4 sinks per tenant. | 2026-11-26 |
| D040-008 | A batch is signed with the platform's delivery scheme (`crates/application/src/signing.rs`, `v1`) under the label `audit-export`, with the batch id as event id and `ocsf.batch` as type, in the headers of a webhook delivery (`crates/scheduler/src/deliveries.rs`), so a receiver that verifies one verifies the other, and the label keeps a batch from passing for a webhook delivery under the same secret. | 2026-11-26 |
| D040-009 | Below a `pii` ceiling the scrubbed `reason` is left out; the actor's subject (an OpenID Connect `sub`, the identifier `GET /audit` returns to the same administrator) is kept, because a security event without its actor is useless and the subject is an opaque identifier issued for this purpose. | 2026-11-26 |
| D040-010 | Secrets and egress policies are namespace-scoped (`secrets`, `egress_policies`), so a tenant sink names a home namespace whose egress policy allows its URL and whose secrets it uses; the operator sink takes its URL and secrets from the engine's configuration. | 2026-11-26 |

## Gaps

| Id | Gap | State |
|---|---|---|
| G040-001 | No caller address in the audit record; `src_endpoint` names the surface. | Open; a privacy decision of its own |
| G040-002 | Authentication failures are not audited, so no Authentication (3002) event exists until design 013 audits sign-ins. | Open; design 013 |
| G040-003 | No object-store sink and no Parquet for Amazon Security Lake. | Open; follow-up |
| G040-004 | Records written before the migration have `tx_id` null and are not streamed. | Accepted; readable through `GET /audit/ocsf` |

## Owner questions

All decided on 2026-11-26 under Owner decision 2026-11-24-4 (delegate); the reasoning is in the design's table under
Owner decisions needed.

| Id | Question | Decision | State |
|---|---|---|---|
| O040-001 | Scope | The whole audit trail; three platform controls as Detection Findings; operational alerts out | Decided, Owner decision 2026-11-26-100 |
| O040-002 | Delivery | Read and push | Decided, Owner decision 2026-11-26-101 |
| O040-003 | Who configures a sink | Tenant administrators (API); the operator (configuration only) | Decided, Owner decision 2026-11-26-102 |
| O040-004 | Version and profiles | 1.10.0; `trace`, `security_control` | Decided, Owner decision 2026-11-26-103 |
| O040-005 | Extension | None in 0.3.0; `unmapped` | Decided, Owner decision 2026-11-26-104 |
| O040-006 | The native shape | Stays; OCSF is a second operation | Decided, Owner decision 2026-11-26-105 |
| O040-007 | Logs | Durable records only | Decided, Owner decision 2026-11-26-106 |
