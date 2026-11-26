# OCSF as the standard schema for security and audit events

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

The platform hands its security-relevant events to the outside in the Open Cybersecurity Schema Framework (OCSF,
<https://schema.ocsf.io/>), version 1.10.0. An administrator reads the audit trail of their tenant as OCSF events, or
configures a sink that receives it as a stream; the operator of the deployment can configure one stream across all
tenants for their own security team. A security information and event management system (SIEM) or a security data lake
that reads OCSF then takes the platform's events without a parser written for this product.

OCSF is a **projection**. The audit record in PostgreSQL (`audit_records`, `crates/domain/src/audit.rs`) stays the
authority and its native read (`GET /audit`) stays as it is. One table in the domain maps each of the 127 audit actions
to exactly one OCSF class, activity, severity and status; the match is exhaustive, so an action without a mapping does
not compile. An adapter builds the OCSF JSON. A stream is read from the audit table itself through a cursor that is
safe against commit order, so writing an audit record costs nothing more than it does today.

The owner asked for it on 2026-11-13 ("Adicione mais uma especificacao de ter o https://schema.ocsf.io/ como padrao",
Owner decision 2026-11-13-6) and put it in release 0.3.0 on 2026-11-24 (Owner decision 2026-11-24-1). The seven open
questions of the first draft (O040-001 to O040-007) are decided here under the owner's delegation (Owner decision
2026-11-24-4), each with its reasoning under Owner decisions needed, and the rule is proposed as
[Architecture decision 0150](../adr/ADR-0150-security-and-audit-events-leave-as-ocsf-projections-of-the-native-record.md).
The registers are in `specs/040-ocsf-event-standard/research.md`.

### Terms

- **Audit record.** One row of `audit_records`: tenant, actor (the OpenID Connect `sub` of the caller, or a `system:`
  subject), action, resource type and id, an optional reason, an optional trace id, a time. Append-only: triggers refuse
  `UPDATE`, `DELETE` and `TRUNCATE` for every role but the owner.
- **Mapping.** For one audit action: an OCSF class (`class_uid`), an activity (`activity_id`), a severity
  (`severity_id`), a status (`status_id`) and the objects the class requires, filled from the record.
- **Sink.** A destination that receives the OCSF stream: a tenant sink (configured by a tenant administrator through
  the API, one tenant's records) or the operator sink (configured by the deployment, every tenant's records).
- **Export cursor.** The position of a sink in the audit trail: the pair (transaction id, record id) of the last
  record it has received.

## The Problem

The audit trail has a shape of its own: `AuditRecord` with 127 dotted actions (`membership.granted`,
`payload.pii_revealed`, `business_id.conflict` and so on, `AuditAction::as_str`), read by an administrator through
`GET /audit` (`specs/001-platform-foundation/contracts/openapi.yaml`, operation `listAuditRecords`) with filters on
action, resource, actor and time and a cursor. A security team that wants these events in its SIEM must learn that
shape, write a parser, poll the endpoint, and keep the parser current as actions are added. Every product they run asks the same of them. OCSF exists to remove that work: one vocabulary of
classes (API Activity 6003, User Management 3007, Detection Finding 2004 and so on), one base event, one set of objects
for the actor, the resource and the outcome, and tools that already read it (Amazon Security Lake, Splunk, Microsoft
Sentinel, the open-source validators).

Two facts of today's code shape the answer:

- `audit_records` has no sequence: its key is a random UUID and `created_at` is taken by the writing transaction, which
  may commit after a later one. A reader that pages by `created_at` can miss a record that commits late. `GET /audit`
  answers newest first and does not promise a stream; a sink must.
- The trail is written by every module of the platform in its own transaction (more than 300 uses of
  `AuditAction` across the crates, inserted by four store modules of `crates/infra-postgres`). A design that adds a write per audit record touches every one of them and the
  cost of every audited transaction.

Temporal publishes no security-event standard; Windmill's audit logs are its own JSON. A standard, validated security
stream is a reason for a regulated customer to choose this platform.

## Requirements

### Functional

- **Requirement 040-001** Every audit action MUST have exactly one mapping: an OCSF class, an activity, a severity and a
  status rule, kept in one table in the domain (`crates/domain/src/audit_ocsf.rs`) whose `match` over `AuditAction` is
  exhaustive, so that adding an action without a mapping fails to compile. The appendix of this design lists the mapping
  of every action that exists on 2026-11-26.
- **Requirement 040-002** An administrator MUST be able to read the audit trail of their tenant as OCSF events through
  `GET /audit/ocsf` (operation `listAuditRecordsOcsf`), with the permission, the filters, the paging, the exclusions and
  the self-audit of `GET /audit` (`audit:read`; `access.denied.cross_tenant` not returned; `audit.read` recorded with the
  same five-minute collapse). The native `GET /audit` MUST stay unchanged.
- **Requirement 040-003** Each event MUST carry the attributes OCSF 1.10.0 requires of its class with the profiles this
  design declares (the base event's `class_uid`, `category_uid`, `activity_id`, `type_uid`, `severity_id`, `time`,
  `metadata`, and each class's required objects), and the platform's facts in OCSF objects: the actor as `actor.user.uid`
  (the subject), the resource as `resources[0]` (`type`, `uid`), the outcome as `status_id`, the closed reason as
  `status_detail`, the trace id as `trace.uid` (profile `trace`), the record id as `metadata.uid`, the tenant id as
  `metadata.tenant_uid`, the product as `metadata.product` (`name` "Sagat Octopus Orchestrator", `vendor_name` "Sagat",
  `version` the binary's version) and the schema version as `metadata.version`.
- **Requirement 040-004** The OCSF version MUST be pinned (1.10.0) and stated in every event. The schema files of that
  version MUST be vendored in the repository with their SHA-256 recorded; moving to another version is a new
  architecture decision with the validation suite run again, never a side effect of a dependency update.
- **Requirement 040-005** An OCSF event MUST disclose nothing that the native record of the same reader does not: no
  payload, no business identifier, no secret value, no field of another tenant. The free-text `reason` MUST be scrubbed
  as `GET /audit` scrubs it, and MUST be left out of an event sent to a sink whose ceiling is below `pii` (Requirement
  040-010).
- **Requirement 040-006** A platform fact that OCSF has no attribute for (the dotted action name, a workflow, run or
  schedule id that is not the resource, the namespace) MUST go in `unmapped`, as OCSF prescribes for a producer without
  a registered extension, with keys that are closed per action and documented in the generated reference.
- **Requirement 040-007** Sandbox violations (`script.violation`), egress denials (`egress.denied`) and denied attempts
  to reach another tenant's resource (`access.denied.cross_tenant`) MUST map to Detection Finding (2004) with the
  `security_control` profile, so that a SIEM sees them as detections without a rule of its own. Every other action maps
  to an activity class (appendix).
- **Requirement 040-008** A tenant administrator holding `audit:sink:manage` MUST be able to create, change, pause,
  resume and delete a tenant sink of kind `https`. Secrets and egress rules are namespace-scoped in this platform, so a
  sink names a home namespace: its HTTPS endpoint MUST be allowed by that namespace's egress policy (feature 009), its
  authentication header value is a secret reference of that namespace, and each body is signed with HMAC-SHA-256 under
  a second secret reference of it; the creator MUST hold the right to use both secrets there, a body format (`ndjson`, optionally gzip), and a ceiling
  (`internal` by default, `pii` allowed under Requirement 040-010). At most 4 sinks per tenant. Every change is audited.
- **Requirement 040-009** A sink MUST receive every audit record of its scope that commits after the sink was created,
  at least once, in the order of the export cursor, with `metadata.uid` stable across redeliveries so that a receiver
  can drop a duplicate. A sink that keeps failing MUST NOT skip records: it backs off, is reported as `STALLED`, and
  resumes from its cursor when the endpoint answers. Records are never dead-lettered.
- **Requirement 040-010** The uniform rule of Owner question 1081 (decided 2026-11-24, delegate Owner decision
  2026-11-24-4, Decision 1617 of feature 020) MUST govern what leaves through a sink: the ceiling defaults to `internal`;
  a ceiling of `pii` is set only by an administrator who holds the reveal grant (feature 016), the grant is checked again
  for every batch, its revocation pauses the sink (`PAUSED_GRANT_REVOKED`), every batch is audited by a collapsed record
  (`audit.sink_delivered`, one per sink and minute with a count), and `secret` never leaves.
- **Requirement 040-011** The operator of the deployment MAY configure one operator sink through the configuration of
  the execution engine (never through the API): every tenant's records, including `access.denied.cross_tenant`, each
  event naming its tenant in `metadata.tenant_uid`, with the same delivery rules. It is the only stream that crosses the
  tenant boundary, and it goes only to the operator, who already holds the database.
- **Requirement 040-012** The mapping MUST be published as reference documentation generated from the table
  (`docs/reference/ocsf-mapping.md`), and a test MUST fail when the page is behind the table.

### Non-functional

- **Non-functional requirement 040-001** Writing an audit record MUST cost what it costs today, plus one 8-byte column
  filled by a default: no added statement, row, lock or trigger. Held by a budget test.
- **Non-functional requirement 040-002** `GET /audit/ocsf` MUST run the same single query as `GET /audit` for the same
  filters and page size, and build each event in memory in bounded time (no query per record).
- **Non-functional requirement 040-003** With one sink and an endpoint that answers within 100 ms, a record MUST reach
  the endpoint within 10 seconds of its commit at the 99th percentile, at 200 audit records per second per tenant, on a
  stated environment (a target, measured by Task 040-17, not a promise).
- **Non-functional requirement 040-004** No tenant, namespace or sink identifier is a metric label (Principle V); the
  export metrics carry the sink kind and the outcome only.

### Success criteria

- **Success criterion 040-001** For every audit action, a record built by a fixture is projected and validated against
  the vendored OCSF 1.10.0 schema with its declared profiles, and passes: 127 of 127 today, and every action added later.
- **Success criterion 040-002** A sample of each class used (API Activity, User Management, Entity Management, Datastore
  Activity, Detection Finding) is accepted by the open-source OCSF validator (`ocsf-validator`, pinned) with no custom
  extension.
- **Success criterion 040-003** A sink receives every one of 10,000 records written by 16 concurrent transactions that
  commit out of the order of their `created_at`, with the process killed once between a delivery and the advance of
  its cursor: every record arrives, some twice with the same `metadata.uid`, none is missed.
- **Success criterion 040-004** A cross-tenant test: a tenant sink, `GET /audit/ocsf` and the generated events of tenant
  A never contain a record, an id or a count of tenant B, as the application role and as the runtime role.
- **Success criterion 040-005** The generated reference equals the table, and adding an action without a mapping fails
  the build (a compile-fail test).

### User stories

- **User story 040-01** As a tenant's security engineer, I point the platform at my SIEM's HTTPS collector and see who
  did what to which resource, classified and timed, without writing a parser.
- **User story 040-02** As the operator of a deployment, I send every tenant's audit and detection events to my own
  security data lake, including the cross-tenant probes the tenants cannot see.
- **User story 040-03** As a developer of the platform, I cannot add an audit action that the SIEMs will not understand.

## Goals

- Integrating a SIEM is configuration: an endpoint, a secret reference, a ceiling.
- A mapping that cannot drift: one table, total by construction, validated against the pinned schema and rendered into
  the reference.
- A stream that loses nothing and slows nothing: no added write, at-least-once, ordered, resumable.

## Non-Goals

- **Replacing the stored audit record.** PostgreSQL keeps the native record as the authority; OCSF is a projection.
- **Telemetry in OCSF.** Traces, metrics and logs stay OpenTelemetry (design 015). Log lines are not OCSF events
  (Owner question 040-007 below).
- **Operational alerts as OCSF findings.** `operational_alerts` (`OUTBOX_STUCK`, `LEASE_NOT_REAPED` and the others) are
  operations, not security; they stay webhook events of feature 008 and metrics.
- **A connector per vendor.** One HTTPS sink speaks to every collector that takes NDJSON over HTTPS with a header.
- **An object-store sink and Parquet** for Amazon Security Lake in 0.3.0: a follow-up (Gap 040-003).
- **Ingesting OCSF from other systems.**
- **A registered OCSF extension** in 0.3.0 (Owner question 040-005 below).

## Open Questions

None for the owner; the seven are decided below. Technical gaps, registered in `specs/040-ocsf-event-standard/research.md`:

- **Gap 040-001** The audit record does not store the caller's network address, so `src_endpoint` (required by API
  Activity and Datastore Activity) carries the platform surface that served the act (`svc_name` `rest`, `grpc` or
  `engine`), not the caller's address. Storing the address is a privacy decision of its own, not taken here.
- **Gap 040-002** Authentication failures are logged and counted, not audited, so OCSF Authentication (3002) events
  come only when design 013 (federated login) audits sign-ins; its actions take their mapping then.
- **Gap 040-003** Amazon Security Lake takes OCSF as Parquet in a bucket; the object-store sink is a follow-up.
- **Gap 040-004** The records written before the migration have no transaction id and are not streamed; they stay
  readable through `GET /audit/ocsf`.

## Owner decisions needed

None open. The seven questions of the first draft are decided under Owner decision 2026-11-24-4 ("Quero que os agents
continuem por mim. Tomem a melhor decisão"), each recorded as a delegate row (Owner decisions 2026-11-26-100 to
2026-11-26-106 in `docs/owner-decisions.md`) and in `specs/040-ocsf-event-standard/research.md`:

| Question | Decision | Why this and not the others |
|---|---|---|
| O040-001 Scope | The whole audit trail, with violations and denials as Detection Findings (Requirement 040-007), in 0.3.0. Operational alerts stay out | The audit trail is what a SIEM needs; the three detection actions are the platform's own security controls firing, which a SIEM must see as findings. Alerts are operations: in a SIEM they would be noise with a security label |
| O040-002 Delivery | Both: read on request (`GET /audit/ocsf`) and push to sinks | A read alone makes every customer write a poller and a cursor, which is the parser problem again; push is what SIEMs are built to take |
| O040-003 Who configures a sink | Tenant administrators for their tenant (API, `audit:sink:manage`); the operator for every tenant (deployment configuration only) | A tenant owns its trail; the operator already holds the database and needs the cross-tenant probes no tenant may see. A cross-tenant stream configurable through the API would be a new way out of the tenant boundary |
| O040-004 Version and profiles | OCSF 1.10.0, the stable version that `https://schema.ocsf.io/api/version` reported on 2026-11-26; profiles `trace` on every event and `security_control` on findings; no `cloud`, `host` or `container` profile | 1.10.0 is current; the 1.9.0 of the first draft is older and deprecates User Access Management (3005), which this mapping would have used. Declaring a profile makes its attributes required; the platform has trace ids and controls, not hosts or cloud accounts |
| O040-005 Extension | No registered extension in 0.3.0; platform facts in `unmapped` with closed keys | Registering an extension takes an identifier from the OCSF project and a review cycle outside this repository; receivers ignore an unknown extension and keep `unmapped`. A follow-up can register one without changing a mapping |
| O040-006 The native shape | Stays the default and is never removed; OCSF is a second operation, `GET /audit/ocsf` | A separate operation has a typed OpenAPI schema and generated SDK methods; content negotiation on one path would give one operation two response schemas. Removing the native shape would break every caller for no gain |
| O040-007 Logs | Only durable records become OCSF; log lines stay OpenTelemetry logs | A log line is lossy by design (ADR-0084); a SIEM fed with lossy security lines would believe a gap is a quiet period |

Architecture decision 0150 is proposed with this design and accepted with it (delegate, Owner decision 2026-11-24-4).

## Critical User Journeys (CUJs)

### CUJ 1: A security team connects its SIEM

*   **Given**: a tenant administrator with `audit:sink:manage`, a namespace `security` whose egress policy allows the
    SIEM collector at `https://siem.example.net/ingest`, and two secrets of that namespace: the collector's token and a
    signing key.
*   **When**: they create a sink `{kind: https, namespace: security, url, authorization: {secretRef: siem-token}, signing: {secretRef:
    siem-signing}, format: ndjson, gzip: true}`; a minute later a developer is granted a role.
*   **Then**: the SIEM receives a User Management event (3007, activity Assign Roles 16, status Success) with the actor's
    subject, the membership as resource, the trace id, and `metadata.uid` the audit record's id, within seconds; the
    sink shows its cursor and lag; the creation of the sink is itself an audit record and an event.

### CUJ 2: The collector is down for an hour

*   **Given**: the sink of CUJ 1.
*   **When**: the collector answers `503` for an hour while 50,000 records are written, and the engine is restarted in
    the middle.
*   **Then**: the sink is `STALLED` after its fifth failure and shows the lag; nothing is dropped; when the collector
    answers again the sink sends the backlog in order, in batches of at most 500 events and 1 MiB, from the cursor it had
    committed; the batch in flight at the restart is sent again with the same `metadata.uid` values.

### CUJ 3: A developer adds an audit action

*   **Given**: a change that adds `AuditAction::FooChanged`.
*   **When**: it is built.
*   **Then**: the build fails at the exhaustive `match` of `audit_ocsf.rs` until the action has a mapping; then the
    reference test fails until `docs/reference/ocsf-mapping.md` is regenerated; then the validation test checks the new
    action's event against the pinned schema.

### CUJ 4: A script tries to escape its sandbox

*   **Given**: the operator sink and a tenant sink.
*   **When**: a script's attempt is stopped by the sandbox and `script.violation` is recorded.
*   **Then**: both sinks receive a Detection Finding (2004, Create) with severity Medium, `finding_info.title`
    "Sandbox violation", `finding_info.uid` the audit record's id, and the script as resource; no value of the script's
    input or output is in it.

## Technical Plan (Architecture)

### 1. The mapping in the domain

`crates/domain/src/audit_ocsf.rs` holds pure data and no serialization (Principle II):

```rust
pub enum OcsfClass { ApiActivity, UserManagement, EntityManagement, DatastoreActivity, DetectionFinding }
pub struct OcsfMapping { pub class: OcsfClass, pub activity: u8, pub severity: OcsfSeverity, pub status: OcsfStatusRule }
pub enum OcsfStatusRule { Success, Failure }
pub const fn mapping(action: AuditAction) -> OcsfMapping { match action { /* one arm per action, no wildcard */ } }
```

`OcsfClass` gives `class_uid` and `category_uid` (6003/6, 3007/3, 3004/3, 6005/6, 2004/2); `type_uid` is computed as
`class_uid * 100 + activity_id`, as OCSF defines it. The arms carry no wildcard, which a clippy lint
(`clippy::wildcard_enum_match_arm`, denied in this module) holds. The appendix is the table on 2026-11-26.

### 2. The projection and its codec

`crates/application/src/audit_ocsf.rs` turns an `AuditRecord` and its mapping into an `OcsfEvent` (an application type
holding the attributes the five classes use) and applies the disclosure rules: the reason scrubbed as `GET /audit` does
(`crates/telemetry/src/redact.rs`), and left out below a `pii` ceiling. `crates/api-http` owns the JSON codec of the read;
the sink adapter in `crates/infra-postgres` and the HTTPS client reuse the same codec from a module of its own
(`crates/application` defines the port `OcsfEncoder`, `crates/api-http` and the engine wire one implementation from a new
small adapter module `crates/telemetry/src/ocsf.rs`, which already depends on `serde_json`). Per class, the required
objects are filled as follows:

| Class | Required object | Filled from |
|---|---|---|
| API Activity 6003 | `api` | `operation` the dotted action, `service.name` "sagat-octopus-orchestrator" |
| API Activity 6003, Datastore Activity 6005 | `actor`, `src_endpoint` | `actor.user.uid` the subject (`type_id` 3 Service for a `system:` subject); `src_endpoint.svc_name` the surface (Gap 040-001) |
| User Management 3007 | `user` | the member or grantee named by the resource id |
| Entity Management 3004 | `entity` | `type` the resource type, `uid` the resource id |
| Datastore Activity 6005 | `database` | `name` the connection's resource id, `type_id` 99 |
| Detection Finding 2004 | `finding_info` | `uid` the record id, `title` from a closed table per action, `analytic.name` the control (`sandbox`, `egress`, `tenant-boundary`) |

`unmapped` holds `sagat.action` (always) and, per action, the closed keys the appendix lists. `metadata.profiles` is
`["trace"]`, plus `"security_control"` on findings.

### 3. The read

`GET /audit/ocsf` reuses the query of `listAuditRecords` (same filters, cursor and limit; `audit:read`; cross-tenant
denials excluded; `audit.read` recorded) and answers `{items: [<OCSF event>], nextCursor}` with
`Cache-Control: no-store`. The OpenAPI document gains the operation and a schema `OcsfEvent` that names the base
attributes and leaves the class objects open (`additionalProperties: true`), because the OCSF schema, not OpenAPI, is
the contract of their shape; the validation test checks them.

### 4. The stream: a cursor safe against commit order

A migration adds `audit_records.tx_id xid8` (nullable, then `SET DEFAULT pg_current_xact_id()`, which rewrites no row;
rows written before stay `NULL`, Gap 040-004) and two partial indexes where `tx_id` is not null: `(tenant_id, tx_id, id)`
for tenant sinks and `(tx_id, id)` for the operator sink, so a tenant's sink never scans another tenant's rows. Both
are empty when built (no row has a `tx_id` yet), but the build still reads the table under a `SHARE` lock; as for the
purge index of `20270119000000_batch_purge`, the migration uses `CREATE INDEX IF NOT EXISTS`, and the runbook tells the
operator of a large trail to build both `CONCURRENTLY` before the upgrade. Every insert gets the id of the transaction
that writes it, at no extra statement. A transaction id below the snapshot's `xmin` belongs to a transaction that has
ended, so the export reads, for a sink with cursor `(c_tx, c_id)`:

```sql
SELECT ... FROM audit_records
 WHERE tx_id < pg_snapshot_xmin(pg_current_snapshot())
   AND (tx_id, id) > ($c_tx, $c_id)
   AND tenant_id = $tenant            -- left out for the operator sink, which reads the second index
 ORDER BY tx_id, id LIMIT 500
```

No record can later appear below a cursor that this query passed: every transaction below `xmin` has committed or
aborted, and an aborted one's rows are invisible. A long transaction holds `xmin` back, which delays the stream (the lag
metric shows it) and loses nothing. `xid8` is 64 bits and does not wrap around.

### 5. Sinks and the export pass

A table `audit_sinks` (tenant, id, home namespace, kind `https`, url, the two secret references, format, gzip, ceiling, state, cursor
`(cursor_tx, cursor_id)`, failures, `next_attempt_at`, the creator and the reveal-grant check time) holds tenant sinks;
the operator sink is a row with `tenant_id` null created from configuration at start. A pass `audit_export` in the
execution engine runs on the shards the replica holds (ADR-0020): a sink belongs to the shard of its tenant, the operator
sink to shard 0. For a due sink, the pass:

1. reads the sink row and checks the fence, the egress rule of the URL and, for a `pii` ceiling, the creator's reveal
   grant (revoked: the sink goes `PAUSED_GRANT_REVOKED`, audited, and stops);
2. reads up to 500 records after the cursor (section 4), projects them, encodes NDJSON up to 1 MiB, resolves the two
   secrets of the home namespace for this call (feature 009's resolution, audited as `secret.resolved`), signs the body
   with the platform's delivery scheme (`crates/application/src/signing.rs`, version `v1`) under a label of its own,
   `audit-export`, so that a batch can never pass for a webhook delivery signed with the same secret, with the batch id
   as the event id and `ocsf.batch` as the type, and sends it through the egress port of the home namespace
   (`crates/infra-egress`) with the headers of a webhook delivery (`x-octopus-signature`, `x-octopus-timestamp`,
   `x-octopus-event-id`, `x-octopus-event-type`, `x-octopus-delivery-id`), a 10-second timeout, and redirects and
   proxies disabled;
3. on a `2xx`, advances the cursor in one transaction fenced on the sink's shard and writes the collapsed
   `audit.sink_delivered` record; on a failure, counts it and sets `next_attempt_at` with an exponential back-off from 1
   second to 5 minutes with jitter; after 5 consecutive failures the state is `STALLED` (audited once) until a success.

A crash between the send and the cursor commit sends the batch again; the receiver drops duplicates by `metadata.uid`.
The pass holds no lock across the HTTP call.

### 6. Surfaces

- REST, under `audit:sink:manage` (a new permission, granted to the `admin` role by the role migration):
  `GET/POST /audit/sinks`, `GET/PATCH/DELETE /audit/sinks/{sinkId}`, `POST /audit/sinks/{sinkId}:pause` and `:resume`,
  and `POST /audit/sinks/{sinkId}:test` (sends one synthetic event marked `metadata.labels: ["test"]`). Typed errors with
  `code`, `retryable`, `traceId`.
- Audit actions: `audit.sink_created`, `audit.sink_changed`, `audit.sink_deleted`, `audit.sink_paused`,
  `audit.sink_resumed`, `audit.sink_stalled`, `audit.sink_delivered` (collapsed per sink and minute), each with its own
  mapping (Entity Management for the first five, API Activity for the last two).
- Metrics: `audit_export_events_total{kind, outcome}`, `audit_export_lag_seconds{kind}` (a histogram of commit to
  acknowledgement), `audit_export_batches_total{kind, outcome}`, `audit_sinks{state}`; an alert rule on a sink stalled
  for 15 minutes in `infra/observability/alerts.yaml`.
- Spans: `audit.export.batch` per batch with the sink kind and the count, never the URL's path or a value.
- Studio: a page under tenant settings to list, create, test, pause and resume sinks, showing lag and state.
- Runbook: `docs/runbooks/audit-export.md` (configure, rotate the two secrets, read the lag, what `STALLED` means, how
  to replay from an earlier cursor by recreating the sink with a start time).

### 7. Rollout

The column and the table are additive. A binary older than 0.3.0 writes audit rows that get `tx_id` from the default,
so a mixed fleet streams correctly. The read is a new operation. The export pass runs only where its code exists; a
sink created while an old engine owns the shard waits until a new one does (the instance registry's capability
`audit-export-v1` gates `POST /audit/sinks` until every engine lists it, ADR-0082).

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A lossy mapping hides what happened | The native record stays the authority; the dotted action is always in `unmapped.sagat.action`; the mapping is reviewed per action in the appendix |
| The stream misses a record that commits late | The cursor is ordered by transaction id and reads only below the snapshot's `xmin` (section 4); Success criterion 040-003 forces commits out of order |
| A long transaction stalls every sink | It delays, never loses; the lag metric and alert show it; statement and idle-in-transaction timeouts bound it |
| A sink leaks across tenants | A tenant sink's query carries its tenant; the cross-tenant test (Success criterion 040-004); only the deployment configures the operator sink |
| Personal data leaves without a decision | The reason is left out below `pii`; `pii` needs the reveal grant, checked per batch (Requirement 040-010) |
| OCSF changes under us | Pinned 1.10.0, vendored with its digest; an upgrade is an architecture decision with the validation suite rerun |
| A receiver is flooded by duplicates after a crash | At most one batch (500 events) is resent per crash; `metadata.uid` dedupes |
| The export slows the engine | One pass, bounded batches, no lock across HTTP, a sink per tenant at most 4 |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | The audit record is written in its transaction as today; a sink's cursor advances in a fenced transaction after the endpoint acknowledges (section 5) |
| II. Modular domain, ports and adapters | Pass | The mapping is pure data in `crates/domain`; the projection in `crates/application`; the JSON codec and the HTTP client in adapters |
| III. External effects are at-least-once; sagas compensate | Pass | Delivery to a sink is at-least-once with a stable `metadata.uid`; nothing is promised exactly once |
| IV. Compatibility is versioned and pinned | Pass | OCSF 1.10.0 pinned and vendored; additive OpenAPI operation; the native read unchanged; a mixed fleet streams (section 7) |
| V. Code, tenant boundary and secrets are isolated | Pass | Tenant-scoped queries and sinks; the operator sink only from configuration; secret references resolved per call and never stored in a sink row; no tenant or sink id as a metric label |
| VI. Observability and audit are part of the feature | Pass | Every sink act audited; metrics with closed labels; spans per batch; a stalled-sink alert |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass with conditions | The stream is read from the audit table by a commit-safe cursor instead of an outbox row per record; the guarantee is the same (nothing lost, redelivery tolerated); Complexity Tracking |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Validation of every action against the pinned schema, the out-of-order and crash test, the cross-tenant battery, the reference drift test |
| Delivery Constraints | Pass | PostgreSQL as authority, no new component, no new region rule; the latency target is measured on a stated environment |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| `tx_id xid8` and a cursor below `xmin` instead of an outbox row per audit record | A stream must not miss a late commit, and the trail is written from more than 300 places in the code | An outbox row per record: a second write in every audited transaction and a change at every call site, for the same guarantee. Paging by `created_at`: misses late commits |
| A sink table and a pass | Push is what SIEMs take | Read only: every customer writes a poller with the same commit-order problem |
| A separate read operation | A typed schema per operation | Content negotiation on `GET /audit`: one operation, two schemas |

## Alternatives Considered

- **Storing audit records natively in OCSF.** It ties the authority to an external schema's versions and rewrites every
  writer; a projection gives the same events.
- **A vendor format (CEF, LEEF, ECS).** OCSF is the one the owner named, is vendor-neutral and is what the security data
  lakes read.
- **An outbox row per audit record** (Principle VII's usual shape). Rejected above; the cursor has the same guarantee
  without the second write.
- **Logical decoding (a replication slot) of `audit_records`.** Ordered by commit and cheap to read, but a slot that a
  stalled sink does not consume holds write-ahead log on the primary without bound, a risk to the authority itself.
- **OCSF Detection Findings for every denial.** A SIEM already alerts on failed API activity; only the three platform
  controls are findings.

## Non-Functional Requirements

See Requirements, Non-functional (Non-functional requirements 040-001 to 040-004).

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Three slices, each a pull request, in this order. Slice 1 touches `crates/domain`, `crates/application`,
`crates/telemetry` and `crates/api-http`; slice 2 touches `crates/infra-postgres`, `crates/scheduler`,
`apps/execution-engine`; slice 3 touches the Studio and the documents.

- [ ] T040-1 Decide Owner questions 040-001 to 040-007 and write the design out; record the delegate rows.
  Refs: Requirement 040-001
- [ ] T040-2 Vendor the OCSF 1.10.0 schema (classes, objects, dictionary, the `trace` and `security_control` profiles)
  under `crates/telemetry/ocsf/1.10.0/` with a `SHA256SUMS` file and a test that the files match it.
  Refs: Requirement 040-004
- [ ] T040-3 Add `audit_ocsf.rs` to the domain with the mapping of every action (the appendix), no wildcard arm, and a
  compile-fail test that an unmapped action does not build.
  Refs: Requirement 040-001, Requirement 040-007, Success criterion 040-005
- [ ] T040-4 Add the projection in the application with the disclosure rules (scrubbed reason; reason left out below
  `pii`; `unmapped` keys closed per action) and unit tests per class.
  Refs: Requirement 040-003, Requirement 040-005, Requirement 040-006
- [ ] T040-5 Add the encoder in `crates/telemetry/src/ocsf.rs` and a test that validates the event of every action
  against the vendored schema with its profiles.
  Refs: Requirement 040-003, Success criterion 040-001
- [ ] T040-6 Validate one sample per class with the pinned `ocsf-validator` in CI (a script under `scripts/`, run by the
  contract job).
  Refs: Success criterion 040-002
- [ ] T040-7 Add `GET /audit/ocsf` with the OpenAPI operation and schema, the same permission, filters, exclusions and
  self-audit as `GET /audit`, and the contract baseline.
  Refs: Requirement 040-002
- [ ] T040-8 Generate `docs/reference/ocsf-mapping.md` from the table and a test that fails when it is behind.
  Refs: Requirement 040-012, Success criterion 040-005
- [ ] T040-9 Write the migration: `audit_records.tx_id` (nullable, then the default, no rewrite), the two indexes, the
  `audit_sinks` table, grants to the runtime role, classification entries, a down migration that refuses while a sink
  exists, and the `audit:sink:manage` permission for `admin`.
  Refs: Requirement 040-008, Requirement 040-009, Non-functional requirement 040-001
- [ ] T040-10 Add the sink resource to the REST API (create, change, pause, resume, delete, test) with the egress and
  secret-reference checks, the reveal-grant check for `pii`, the limit of 4 per tenant, and the audit actions.
  Refs: Requirement 040-008, Requirement 040-010
- [ ] T040-11 Add the `audit_export` pass: fenced per shard, the cursor query below `xmin`, the batch and byte bounds,
  signing, the back-off, `STALLED`, the per-batch grant check and `PAUSED_GRANT_REVOKED`.
  Refs: Requirement 040-009, Requirement 040-010
- [ ] T040-12 Add the operator sink from the engine's configuration, with the cross-tenant denials included and its
  configuration documented.
  Refs: Requirement 040-011
- [ ] T040-13 Fault test: 16 concurrent writers committing out of order, a kill between send and cursor commit; every
  record delivered, duplicates only with the same `metadata.uid`.
  Refs: Success criterion 040-003, Requirement 040-009
- [ ] T040-14 Budget test: an audited transaction issues the same statements as before the migration.
  Refs: Non-functional requirement 040-001
- [ ] T040-15 Cross-tenant and privilege tests for the read, the sink resource and the pass, also as the runtime role.
  Refs: Success criterion 040-004
- [ ] T040-16 Metrics, spans, the stalled-sink alert rule with its rule test in `infra/observability/tests/`, and the
  capability `audit-export-v1` in the instance registry.
  Refs: Requirement 040-009, Non-functional requirement 040-004
- [ ] T040-17 Measure Non-functional requirement 040-003 on a stated environment and write the dated report in
  `specs/040-ocsf-event-standard/validation-report.md`.
  Refs: Non-functional requirement 040-003
- [ ] T040-18 Studio: the sinks page (list, create, test, pause, resume, lag and state).
  Refs: Requirement 040-008
- [ ] T040-19 Write the runbook, the quickstart of CUJ 1 with the failure scenario of CUJ 2, and update
  `docs/traceability.md`, the roadmap row and the release notes.
  Refs: Requirement 040-012

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the three
checkers, the unit tests of the crates it changes and the suites it touches, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`
where it adds a statement. A test is seen failing for the right reason before the code that passes it.

- Slice 1: the domain and projection unit tests; the validation of every action against the vendored schema (Success
  criterion 040-001); the `ocsf-validator` samples; the OpenAPI baseline; the reference drift test; a hand mutation
  (map one action to a class whose required object is missing) that the validation test catches.
- Slice 2: a new target `integration_ocsf_export` on real PostgreSQL with a local HTTPS receiver; the fault test of Task
  040-13; the budget test; the cross-tenant battery as both roles; the migration's up, down and privilege audit.
- Slice 3: the Studio's `npm run api && npm run check:api` and its unit tests; the quickstart run end to end including
  the outage of CUJ 2.

## Appendix: the mapping of every audit action (2026-11-26)

Classes: **API** = API Activity 6003 (activities 1 Create, 2 Read, 3 Update, 4 Delete, 5 List, 99 Other); **UM** = User
Management 3007 (14 Assign Privileges, 15 Remove Privileges, 16 Assign Roles); **EM** = Entity Management 3004 (1 Create,
3 Update, 4 Delete, 6 Enroll, 7 Unenroll, 8 Enable, 9 Disable, 12 Suspend, 13 Resume); **DS** = Datastore Activity 6005
(1 Read, 5 Write, 8 List, 99 Other); **DF** = Detection Finding 2004 (1 Create). Severity is Informational (1) unless the
row says Low (2) or Medium (3). Status is Success unless the row says Failure. Every event carries
`unmapped.sagat.action`.

| Actions | Class, activity | Severity, status |
|---|---|---|
| `tenant.provisioned`, `workspace.created`, `namespace.created` | EM 1 | |
| `membership.granted` | UM 16 | |
| `payload.reveal_granted`, `secret.access_granted` | UM 14 | Low |
| `payload.reveal_revoked`, `secret.access_revoked` | UM 15 | Low |
| `quota.changed`, `egress.policy_changed`, `publication.policy_changed`, `secret.pools_changed`, `database.connection_changed` | EM 3 | Low |
| `event.endpoint_created`, `webhook.subscription_created`, `broker.subscription_created`, `connector.created` | EM 1 | |
| `event.endpoint_changed`, `webhook.subscription_changed`, `broker.subscription_changed`, `connector.changed` | EM 3 | |
| `event.endpoint_deleted`, `webhook.subscription_deleted`, `broker.subscription_deleted`, `connector.deleted` | EM 4 | |
| `webhook.subscription_disabled`, `broker.subscription_disabled` | EM 9 | |
| `worker.bound` | EM 6 | |
| `worker.unbound` | EM 7 | |
| `worker.drained` | EM 9 | |
| `worker.undrained` | EM 8 | |
| `worker.denied` | EM 99 | Low, Failure |
| `workflow.created`, `script.created`, `schedule.created`, `batch.created`, `blob.uploaded`, `secret.stored`, `publication.requested` | API 1 | |
| `workflow.draft_updated`, `workflow.published`, `workflow.activated`, `script.changed`, `script.published`, `script.built`, `schedule.updated`, `run.updated`, `iceberg.revert_requested` | API 3 | |
| `schedule.paused`, `run.paused` | API 3 | |
| `schedule.resumed`, `run.resumed`, `schedule.auto_paused` | API 3 | |
| Deletions: `script.deleted`, `schedule.deleted`, `blob.deleted`, `secret.deleted`, `run.logs_deleted`, `secret.provider_retired` | API 4 | |
| `run.queried`, `run.logs_read`, `payload.inspected`, `payload.disclosed`, `secret.resolved`, `blob.read`, `schedule.input_disclosed`, `audit.read` | API 2 | |
| `payload.pii_revealed` | API 2 | Low |
| `database.result_read` | DS 1 | |
| `database.catalog_browsed` | DS 8 | |
| `iceberg.write_committed` | DS 5 | |
| `destination.decided` | DS 99 | |
| `database.denied` | DS 99 | Low, Failure |
| `run.started`, `run.continued`, `run.rerun`, `run.reset`, `run.reset_requested`, `schedule.triggered`, `schedule.backfill_requested`, `event.published` | API 1 | |
| `run.cancel_requested`, `schedule.backfill_cancelled`, `batch.cancelled`, `approval.cancelled`, `publication.withdrawn` | API 4 | |
| `run.finished`, `batch.finished`, `run.signalled`, `run.marked_for_review`, `approval.opened`, `approval.decided`, `approval.expired`, `publication.approved`, `publication.superseded`, `saga.step_retried`, `saga.step_skipped`, `saga.step_resolved`, `script.tested`, `webhook.delivery_redriven`, `broker.delivery_redriven`, `connector.message_redriven`, `connector.message_discarded`, `capacity.member.finished`, `capacity.member.vacated`, `capacity.member.acquired`, `collection.leaf_success_certified`, `secret.provisioning_started`, `secret.provider_abandoned`, `secret.attempt_pins_released`, `business_id.conflict` | API 99 | |
| `run.input_refused`, `run.storage_quota_exceeded`, `run.data_limit_exceeded`, `run.collection_chain_failed`, `event.rejected`, `publication.rejected` | API 99 | Failure |
| `access.denied.permission`, `payload.inspection_denied`, `payload.reveal_denied`, `approval.denied`, `secret.denied`, `secret.delivery_refused`, `publication.denied` | API 99 | Low, Failure |
| `script.violation` | DF 1 (`analytic.name` `sandbox`, title "Sandbox violation") | Medium, Failure |
| `egress.denied` | DF 1 (`analytic.name` `egress`, title "Egress denied") | Medium, Failure |
| `access.denied.cross_tenant` | DF 1 (`analytic.name` `tenant-boundary`, title "Cross-tenant access denied"); operator sink only | Medium, Failure |

The table covers the 127 values of `AuditAction::as_str` on 2026-11-26; Task 040-3 turns it into the `match` and the
compile-fail test holds it total. A reviewer who finds an action missing here finds it missing in the code too.
