# Research: OCSF as the standard schema for security and audit events (design 040)

The registers of design 040 (`docs/designs/040-ocsf-event-standard.md`). Append-only: an entry is never renumbered;
a later entry may amend an earlier one by naming it.

## Decisions

| Id | Decision | Date |
|---|---|---|
| D040-001 | Pin OCSF 1.10.0, the version `https://schema.ocsf.io/api/version` reported on 2026-11-26. The first draft's 1.9.0 is older, and 1.9.0 deprecates User Access Management (3005) in favor of User Management (3007), which this mapping uses. | 2026-11-26 |
| D040-002 | Amended by D040-014 (four classes). Five classes cover the 127 actions: API Activity 6003, User Management 3007, Entity Management 3004, Datastore Activity 6005, Detection Finding 2004. Scheduled Job Activity 1006 was considered for schedules and rejected: it requires `device` and `job`, and the platform has no device to name, so filling it would invent data. | 2026-11-26 |
| D040-003 | Profiles: `trace` on events that carry a trace id (`trace.uid`), `security_control` on findings. No `cloud`, `host`, `container` or `datetime` profile: the platform has none of their facts. Amended by D040-011. | 2026-11-26 |
| D040-004 | `src_endpoint` is filled with the serving surface (`svc_name` `rest`, `grpc`, `engine`), because the audit record does not store the caller's address (Gap 040-001). | 2026-11-26 |
| D040-005 | The stream reads `audit_records` by `(tx_id, id)` below the snapshot's `xmin`; `tx_id` is an `xid8` column added nullable and then given the default `pg_current_xact_id()`, which rewrites no row. | 2026-11-26 |
| D040-006 | A sink stalls and never dead-letters: the audit table is the backlog and is kept forever, so a cursor never becomes invalid. Amended by D040-011 (a record refused alone is passed and listed) and D040-013 (a restore can invalidate cursors until the epoch step). | 2026-11-26 |
| D040-007 | Batches of at most 500 events and 1 MiB; a 10-second request timeout; back-off from 1 second to 5 minutes with jitter; `STALLED` after 5 consecutive failures; at most 4 sinks per tenant. | 2026-11-26 |
| D040-008 | A batch is signed with the platform's delivery scheme (`crates/application/src/signing.rs`, `v1`). Amended by D040-011: a new direction with the label `octopus-audit-export-v1`, a new delivery id per attempt, the bytes sent signed, two keys during a rotation. | 2026-11-26 |
| D040-009 | Below a `pii` ceiling the scrubbed `reason` is left out; the actor's subject (an OpenID Connect `sub`, the identifier `GET /audit` returns to the same administrator) is kept, because a security event without its actor is useless and the subject is an opaque identifier issued for this purpose. | 2026-11-26 |
| D040-010 | Egress policies are namespace-scoped (`egress_policies`), so a tenant sink names a home namespace whose egress policy allows its URL; amended by D040-011, its credentials are its own, not namespace secrets. | 2026-11-26 |
| D040-011 | Revision after the first security and correctness reviews (2026-11-26, `docs/designs/040-ocsf-event-standard.reviews/2026-11-26/round-1/`): withheld actions in the tenant query; the cursor never shown; the `pii` grant in every namespace, checked per batch and at commit; the authorizing subject and its per-batch check; sink-owned sealed credentials and a platform-generated signing key instead of namespace secrets; the direction `AuditExport` (`octopus-audit-export-v1`) signing the bytes sent with two keys during a rotation; the operator sink in configuration with one tenant per message and only its cursor persisted; `CURSOR_INVALID` after a restore; a sink state machine with a claimed, guarded commit; halving and refused records; `AuditAction::parse` and the unknown mapping; Detection Finding semantics; validation against the exported JSON Schema and the OCSF server; two migrations with concurrent index builds; delivery ids new per attempt; activation through the registry. | 2026-11-26 |
| D040-012 | Secret access grants map to API Activity Update with the secret as resource, because the grantee is only in the free-text reason; a membership's role is not filled for the same reason. | 2026-11-26 |
| D040-013 | Revision after the architecture review (2026-11-26, round 1): OCSF's mapping and serialization in `crates/application` with no encoder port (a port implemented in `crates/telemetry` would be a dependency cycle); the export in its own engine loop, not the scheduler tick; a claim renewed before each request and a commit after each accepted request; destination-level refusals pause the sink and pass no record; the operator sink's order is per tenant with the longest acknowledged prefix; a restore advances the transaction-id epoch (`pg_resetwal --epoch`) and the export refuses until it is done; idle sinks look read-only first and assign no transaction id; credentials sealed through `SecretCipher` with associated data per sink and column, part of design 015's inventory; a wire contract in AsyncAPI; the region epoch in every event and a tombstone after a lossy failover. | 2026-11-26 |
| D040-014 | Revision after the EGM implementation-readiness review (2026-11-26, round 1): Datastore Activity dropped (no writer records a connection as the resource), so four classes; `src_endpoint.svc_name` the constant `sagat`; `unmapped` holds only `sagat.action` and `sagat.region_epoch`, nested; the credentials sealed with today's `SecretCipher` and a length-prefixed associated data, so slice 2 needs nothing of design 015; one statement per no-transaction index migration; the idle back-off in memory; the storage, the ports and the signed fields written down; the sink contract in OpenAPI before the code; tenant sinks through `WebhookSender`, the operator sink through a new `AuditExportSender`; the `pii` recheck as one snapshot with its failure path; the OCSF server image built in the repository. | 2026-11-26 |
| D040-015 | Revision after the EGM critique (2026-11-26, round 1): cursors record their cluster and `CURSOR_INVALID` holds until `audit-export rebase`; the operator sink is one stream per tenant; halving is left first, depth first, over a contiguous prefix, and a lone refused record passes only on a sink that has had a `2xx`; one rule for a `pii` sink changed by a subject without the grant (`403` unless the request sets `internal`); one key id per sealed column; `startFrom` reads a 10-minute window; the region epoch is the epoch at sending and a tombstone rewinds the cursor to the lost window; `audit.sink_delivered` only for `pii` sinks. | 2026-11-26 |
| D040-016 | Revision after the security re-read (2026-11-26): the operator sink's destination is fingerprinted in `audit_operator_config`, so only a real change of destination applies its `start`, once; tenants are discovered by a global cursor and a new tenant's stream starts just below its first record; the two administrative reads (`system_identifier`, the oldest transaction-id holder) go through narrow `SECURITY DEFINER` functions; a key rotation re-establishes the authorizing subject; `/test` answers `unreachable` for every transport and resolution failure. | 2026-11-26 |
| D040-017 | After the architecture and correctness re-reads (2026-11-26): a lone refused record passes only beside an accepted sibling, with a cap of 10 per hour, and a sink's own `audit.sink_*` records stay out of its scope; the epoch step is mandatory for a logical restore and `rebase` refuses without it, covering the discovery cursor; every write of the pass carries the claim, revision and state predicate, every administrator's act raises the revision, and the pass releases its claim; the operator fingerprint covers the URL and the credential, keeps cursors on a change, and applies `start` only when it changes; the sealed sink columns join `reseal-secrets` (Task 040-26). | 2026-11-26 |

## Gaps

| Id | Gap | State |
|---|---|---|
| G040-001 | No caller address in the audit record; `src_endpoint` names the surface. | Open; a privacy decision of its own |
| G040-002 | Authentication failures are not audited, so no Authentication (3002) event exists until design 013 audits sign-ins. | Open; design 013 |
| G040-003 | No object-store sink and no Parquet for Amazon Security Lake. | Open; follow-up |
| G040-004 | Records written before the migration have `tx_id` null and are not streamed. | Accepted; readable through `GET /audit/ocsf` |
| G040-005 | The tombstone after a lossy failover needs feature 032's region epoch and failover report. | Open; Task 040-21 |
| G040-006 | `trace.uid` may be chosen by the caller. | Accepted; stated in the reference |
| G040-007 | On a managed service without `pg_resetwal`, only a physical restore keeps the export. | Open; the runbook says so |
| G040-008 | `startFrom` and `rebase` read a 10-minute window; a record of a transaction that lived longer than 10 minutes can fall below the cursor (the platform's pools bound statements, not transactions; the oldest holder is alerted at 5 minutes). | Open; a stated residual risk |

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
