# OCSF as the standard schema for security and audit events

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

The platform hands its security-relevant events to the outside in the Open Cybersecurity Schema Framework (OCSF,
<https://schema.ocsf.io/>), version 1.10.0. An administrator reads the audit trail of their tenant as OCSF events, or
configures a sink that receives it as a stream; the operator of the deployment can configure one stream of every
tenant's events for their own security team. A security information and event management system (SIEM) or a security
data lake that reads OCSF then takes the platform's events without a parser written for this product.

OCSF is a **projection**. The audit record in PostgreSQL (`audit_records`, `crates/domain/src/audit.rs`) stays the
authority and its native read (`GET /audit`) stays as it is. One table in the domain maps each audit action (127 on
2026-11-26, 134 with the seven this design adds) to exactly one OCSF class, activity, severity and status; the match is
exhaustive, so an action without a mapping does not compile. An adapter builds the OCSF JSON. A stream is read from the
audit table itself through a cursor that is safe against commit order, so writing an audit record adds no statement.

The owner asked for it on 2026-11-13 ("Adicione mais uma especificacao de ter o https://schema.ocsf.io/ como padrao",
Owner decision 2026-11-13-6) and put it in release 0.3.0 on 2026-11-24 (Owner decision 2026-11-24-1). The seven open
questions of the first draft (O040-001 to O040-007) are decided here under the owner's delegation (Owner decision
2026-11-24-4), each with its reasoning under Owner decisions needed, and the rule is
[Architecture decision 0150](../adr/ADR-0150-security-and-audit-events-leave-as-ocsf-projections-of-the-native-record.md).
This revision answers the first security and correctness reviews of 2026-11-26
(`docs/designs/040-ocsf-event-standard.reviews/2026-11-26/round-1/`). The registers are in
`specs/040-ocsf-event-standard/research.md`.

### Terms

- **Audit record.** One row of `audit_records`: tenant, actor (the OpenID Connect `sub` of the caller, or a `system:`
  subject), action, resource type and id, an optional reason, an optional trace id, a time. Append-only: triggers refuse
  `UPDATE`, `DELETE` and `TRUNCATE` for every role but the owner. It has no namespace column.
- **Mapping.** For one audit action: an OCSF class (`class_uid`), an activity (`activity_id`), a severity
  (`severity_id`), a status (`status_id`) and the objects the class requires, filled from the record.
- **Withheld action.** An action `GET /audit` never returns to a tenant (`access.denied.cross_tenant` today, the set
  `WITHHELD_FROM_TENANT` that `crates/infra-postgres/src/operations.rs` filters); the same set is withheld from tenant
  sinks.
- **Sink.** A destination that receives the OCSF stream: a tenant sink (created by a tenant administrator through the
  API, one tenant's records) or the operator sink (configured by the deployment, every tenant's records, one tenant per
  message).
- **Authorizing subject.** The administrator whose authority a tenant sink runs under: its creator, or the last person
  who changed its destination, credentials or ceiling, or resumed it.
- **Export cursor.** The position of a sink in the audit trail: the pair (transaction id, record id) of the last record
  it has passed. It is internal and never shown (it would reveal the cluster's transaction rate).

## The Problem

The audit trail has a shape of its own: `AuditRecord` with 127 dotted actions (`membership.granted`,
`payload.pii_revealed`, `business_id.conflict` and so on, `AuditAction::as_str`), read by an administrator through
`GET /audit` (`specs/001-platform-foundation/contracts/openapi.yaml`, operation `listAuditRecords`) with filters on
action, resource, actor and time and a cursor. A security team that wants these events in its SIEM must learn that
shape, write a parser, poll the endpoint, and keep the parser current as actions are added. Every product they run asks
the same of them. OCSF exists to remove that work: one vocabulary of classes (API Activity 6003, User Management 3007,
Detection Finding 2004 and so on), one base event, one set of objects for the actor, the resource and the outcome, and
tools that already read it (Amazon Security Lake, Splunk, Microsoft Sentinel).

Two facts of today's code shape the answer:

- `audit_records` has no sequence: its key is a random UUID and `created_at` is taken by the application before the
  writing transaction commits, which may be after a later one's. A reader that pages by `created_at` can miss a record
  that commits late. `GET /audit` answers newest first and does not promise a stream; a sink must.
- The trail is written by many modules in their own transactions (more than 300 uses of `AuditAction`), inserted by six
  statements in four store modules of `crates/infra-postgres`. A design that adds a write per audit record adds it to
  every audited transaction.

Temporal publishes no security-event standard; Windmill's audit logs are its own JSON. A standard, validated security
stream is a reason for a regulated customer to choose this platform.

## Requirements

### Functional

- **Requirement 040-001** Every audit action MUST have exactly one mapping: an OCSF class, an activity, a severity and a
  status, kept in one table in the domain (`crates/domain/src/audit_ocsf.rs`) whose `match` over `AuditAction` has no
  wildcard arm. `AuditAction` gains `ALL` and `parse` generated by one macro with the enum, with a round-trip test, so
  that a stored action string maps back to the enum; a string no variant matches (written by a newer binary) is
  projected as API Activity 99, severity Unknown (0), with the raw string in `unmapped.sagat.action`, never an error.
  The appendix lists the mapping of every action on 2026-11-26. The status describes the act the record states (a
  refusal is Failure), not the later outcome of the work it started.
- **Requirement 040-002** An administrator MUST be able to read the audit trail of their tenant as OCSF events through
  `GET /audit/ocsf` (operation `listAuditRecordsOcsf`), with the permission, the filters, the paging, the withheld
  actions and the self-audit of `GET /audit` (`audit:read`; `audit.read` recorded with the same five-minute collapse).
  The native `GET /audit` MUST stay unchanged.
- **Requirement 040-003** Each event MUST carry the attributes OCSF 1.10.0 requires of its class (the base event's
  `class_uid`, `category_uid`, `activity_id`, `type_uid`, `severity_id`, `time`, `metadata`, and each class's required
  objects), and the platform's facts in OCSF objects: the actor as `actor.user.uid` (the subject; on a finding in
  `evidences[0].actor`), the resource as `resources[0]` (`type`, `uid`), the outcome as `status_id`, the reason
  (scrubbed, and left out below a `pii` ceiling, Requirement 040-005) as `status_detail`, the trace id as `trace.uid`
  with the profile `trace` declared only on an event that has one, the record id as `metadata.uid`, the tenant id as
  `metadata.tenant_uid`, the product as `metadata.product` (`name` "Sagat Octopus Orchestrator", `vendor_name` "Sagat",
  `version` the release's major and minor only) and the schema version as `metadata.version`. Every string attribute
  is capped (1,024 characters for an identifier, 2,048 for the reason, as `scrub` already cuts it) and a cut value ends
  with the marker `…[truncated]`.
- **Requirement 040-004** The OCSF version MUST be pinned (1.10.0) and stated in every event. The JSON Schema that the
  OCSF server of that version exports for each class used, with the declared profiles, MUST be vendored in the
  repository with its SHA-256 recorded; moving to another version is a new architecture decision with the validation
  suite run again, never a side effect of a dependency update.
- **Requirement 040-005** An OCSF event MUST disclose nothing that the native record of the same reader does not: no
  payload, no business identifier, no secret value, no field of another tenant, no value parsed out of the free-text
  reason. The reason MUST be scrubbed as `GET /audit` scrubs it (`sagat_telemetry::scrub`), and MUST be left out of an
  event sent to a sink whose ceiling is below `pii` (Requirement 040-010).
- **Requirement 040-006** A platform fact that OCSF has no attribute for (the dotted action name; the resource type when
  OCSF has no equivalent) MUST go in `unmapped`, as OCSF prescribes for a producer without a registered extension, with
  keys that are closed per action and documented in the generated reference.
- **Requirement 040-007** Sandbox violations (`script.violation`), egress denials (`egress.denied`) and denied attempts
  to reach another tenant's resource (`access.denied.cross_tenant`) MUST map to Detection Finding (2004) with the
  `security_control` profile: `status_id` 1 (New), `action_id` 2 (Denied), `disposition_id` 2 (Blocked),
  `finding_info.analytic.type_id` 1 (Rule) and its `name` the control. Every other action maps to an activity class.
- **Requirement 040-008** A tenant administrator holding `audit:sink:manage` and `audit:read` MUST be able to create,
  change, pause, resume, test and delete a tenant sink of kind `https`. A sink has a destination URL, a collector
  credential (a header name and a value), a signing key, a home namespace whose egress policy MUST allow the URL (egress
  policies are namespace-scoped in this platform), a body format (`ndjson`, optionally gzip) and a ceiling (`internal`
  by default, `pii` under Requirement 040-010). The URL and the collector credential are written once and never read
  back: they are sealed with the platform's key authority (design 015) in the sink's own row, are not namespace secrets
  and can never be delivered to a task, a webhook or any other consumer. The platform generates the signing key and
  returns it once, at creation and at each rotation. The API answers a sink by `scheme://host` and a fingerprint of the
  path. At most 4 sinks per tenant, counted under a lock of the tenant's row. Every act is audited.
- **Requirement 040-009** A sink MUST receive every record of its scope (its tenant's records less the withheld
  actions and the `audit.sink_delivered` records; for the operator sink every tenant's records less
  `audit.sink_delivered`) that commits after the sink's start, at least once, in the order of the export cursor, with
  `metadata.uid` stable across redeliveries so that a receiver drops a duplicate by it. A sink that keeps failing MUST
  NOT skip records: it backs off and becomes `STALLED`. A record the receiver refuses permanently, alone in its batch, is
  the one exception: it is recorded as refused (`audit.sink_record_refused`, the record id and the sink), listed by the
  API, and passed; it stays in the audit table, which is its dead-letter store (Principle VII).
- **Requirement 040-010** The uniform rule of Owner question 1081 (decided 2026-11-24, delegate Owner decision
  2026-11-24-4, Decision 1617 of feature 020) MUST govern what leaves through a tenant sink: the ceiling defaults to
  `internal`; a `pii` ceiling is set only by an authorizing subject who holds the effective reveal grant in every
  namespace of the tenant (`has_effective_payload_reveal_grant`, as the step-log quota rule reads it for data that spans
  the tenant), checked when it is set, before each batch and again in the transaction that advances the cursor; when it
  fails the sink is `PAUSED_GRANT_REVOKED` and at most the batch already in flight has left; every batch is counted in
  metrics and every pause audited; `secret` never leaves. The operator sink's ceiling is `internal`, fixed.
- **Requirement 040-011** The operator of the deployment MAY configure one operator sink through the execution
  engine's configuration (never through the API): its URL, collector credential, signing key and allowed hosts are held
  in memory from that configuration, and only its cursor is persisted. Each message it sends holds the records of one
  tenant, naming it in `metadata.tenant_uid`, so that messages keep tenant scope (Principle V). Its own evidence
  (deliveries, stalls, refusals) goes to the engine's logs and metrics, never into a tenant's trail.
- **Requirement 040-012** The mapping MUST be published as reference documentation generated from the table
  (`docs/reference/ocsf-mapping.md`), and a test MUST fail when the page is behind the table.
- **Requirement 040-013** Before each batch, the pass MUST check the authorizing subject's current authority: membership
  of the tenant, `audit:sink:manage` and `audit:read`. When it fails, the sink is `PAUSED_AUTHORITY_LOST`, audited.
  Any change of the URL, the credential, the home namespace or the ceiling, and any resume, makes the person who does it
  the authorizing subject; a `pii` ceiling then needs their grant (Requirement 040-010) or drops to `internal`.

### Non-functional

- **Non-functional requirement 040-001** Writing an audit record MUST add no statement, row, lock or trigger: one 8-byte
  column filled by a default and two entries in partial indexes. The write-ahead log bytes and the insert latency of
  10,000 audit inserts are measured before and after the migration and reported (Task 040-14).
- **Non-functional requirement 040-002** `GET /audit/ocsf` MUST run the same single query as `GET /audit` for the same
  filters and page size, and build each event in memory in bounded time (no query per record).
- **Non-functional requirement 040-003** With one sink and an endpoint that answers within 100 ms, the lag of a record
  (its acknowledgement time minus its `created_at`, which the application sets before the commit; the clocks of the
  engine and the control plane are assumed within 1 second) MUST be at most 10 seconds at the 99th percentile, at 200
  audit records per second per tenant, on a stated environment (a target, measured by Task 040-17).
- **Non-functional requirement 040-004** No tenant, namespace or sink identifier is a metric label (Principle V); the
  export metrics carry the sink kind, the state and the outcome only.

### Success criteria

- **Success criterion 040-001** For every audit action, records built by fixtures (with and without a trace id, with
  and without a reason, under both ceilings, with a `system:` actor) are projected and validated against the vendored
  JSON Schema, and pass: 134 of 134 when this design is built, and every action added later; an unknown action string
  projects and validates too.
- **Success criterion 040-002** A sample of each class used (API Activity, User Management, Entity Management, Datastore
  Activity, Detection Finding) is accepted by the validation API of the pinned OCSF server image, run by digest in the
  contract job.
- **Success criterion 040-003** Commit order, forced: transaction T1 inserts an audit record and is held open behind a
  barrier; T2, with a higher transaction id, commits; a full pass runs and its cursor stays below T1 and T2's record is
  not delivered; T1 is released and both records arrive in `(tx_id, id)` order. A fault hook between the receiver's
  `2xx` and the cursor commit kills the pass: the batch is sent again, every record arrives, duplicates carry the same
  `metadata.uid`. With events of about 3 KB the byte bound cuts batches, and no record id is missing at a batch edge. A
  hand mutation of `pg_snapshot_xmin` into `pg_snapshot_xmax` fails the first part.
- **Success criterion 040-004** A cross-tenant test: tenant A probes a namespace of tenant B; no event, count or position
  of A's sink, of `GET /audit/ocsf` or of the sink API shows it, and nothing of B appears in A's events, as the
  application role and as the runtime role.
- **Success criterion 040-005** The generated reference equals the table; an encoder that omits one required object
  fails the validation test.
- **Success criterion 040-006** Each sink state change races an in-flight batch in both orders (pause, grant revoked,
  authority lost, delete), held by barriers: the batch's commit changes nothing when the sink moved, and no batch leaves
  after the change committed except the one already in flight.

### User stories

- **User story 040-01** As a tenant's security engineer, I point the platform at my SIEM's HTTPS collector and see who
  did what to which resource, classified and timed, without writing a parser.
- **User story 040-02** As the operator of a deployment, I send every tenant's audit and detection events to my own
  security data lake, including the cross-tenant probes the tenants cannot see.
- **User story 040-03** As a developer of the platform, I cannot add an audit action that the SIEMs will not understand.

## Goals

- Integrating a SIEM is configuration: an endpoint, a credential, a ceiling.
- A mapping that cannot drift: one table, total by construction, validated against the pinned schema and rendered into
  the reference.
- A stream that loses nothing and adds no statement to the audited transactions: at-least-once, ordered, resumable.

## Non-Goals

- **Replacing the stored audit record.** PostgreSQL keeps the native record as the authority; OCSF is a projection.
- **Telemetry in OCSF.** Traces, metrics and logs stay OpenTelemetry (design 015). Log lines are not OCSF events
  (Owner question 040-007 below).
- **Operational alerts as OCSF findings.** `operational_alerts` are operations, not security; they stay webhook events
  of feature 008 and metrics.
- **A connector per vendor.** One HTTPS sink speaks to every collector that takes NDJSON over HTTPS with a header.
- **An object-store sink and Parquet** for Amazon Security Lake in 0.3.0: a follow-up (Gap 040-003).
- **Ingesting OCSF from other systems.**
- **A registered OCSF extension** in 0.3.0 (Owner question 040-005 below).

## Open Questions

None for the owner; the seven are decided below. Technical gaps, registered in `specs/040-ocsf-event-standard/research.md`:

- **Gap 040-001** The audit record does not store the caller's network address, so `src_endpoint` (required by API
  Activity and Datastore Activity) carries the platform surface that served the act (`svc_name` `rest`, `grpc` or
  `engine`). Storing the address is a privacy decision of its own, not taken here.
- **Gap 040-002** Authentication failures are logged and counted, not audited, so OCSF Authentication (3002) events
  come only when design 013 (federated login) audits sign-ins.
- **Gap 040-003** Amazon Security Lake takes OCSF as Parquet in a bucket; the object-store sink is a follow-up.
- **Gap 040-004** The records written before the migration have no transaction id and are not streamed; they stay
  readable through `GET /audit/ocsf`, and a sink's `startFrom` reaches only records that have one.
- **Gap 040-005** After an unplanned failover in an asynchronous replication mode (feature 032), a SIEM may hold events
  whose records the promoted standby lost; the failover report lists the window, and no tombstone is sent.
- **Gap 040-006** `trace.uid` may be chosen by the caller (`traceparent`), so unrelated events can share it; the
  reference says so.

## Owner decisions needed

None open. The seven questions of the first draft are decided under Owner decision 2026-11-24-4 ("Quero que os agents
continuem por mim. Tomem a melhor decisão"), each recorded as a delegate row (Owner decisions 2026-11-26-100 to
2026-11-26-106 in `docs/owner-decisions.md`) and in `specs/040-ocsf-event-standard/research.md`:

| Question | Decision | Why this and not the others |
|---|---|---|
| O040-001 Scope | The whole audit trail, with violations and denials as Detection Findings (Requirement 040-007), in 0.3.0. Operational alerts stay out | The audit trail is what a SIEM needs; the three detection actions are the platform's own security controls firing, which a SIEM must see as findings. Alerts are operations: in a SIEM they would be noise with a security label |
| O040-002 Delivery | Both: read on request (`GET /audit/ocsf`) and push to sinks | A read alone makes every customer write a poller and a cursor, which is the parser problem again; push is what SIEMs are built to take |
| O040-003 Who configures a sink | Tenant administrators for their tenant (API, `audit:sink:manage`); the operator for every tenant (deployment configuration only) | A tenant owns its trail; the operator already holds the database and needs the cross-tenant probes no tenant may see. A cross-tenant stream configurable through the API would be a new way out of the tenant boundary |
| O040-004 Version and profiles | OCSF 1.10.0, the stable version that `https://schema.ocsf.io/api/version` reported on 2026-11-26; the profile `trace` on events that carry a trace id and `security_control` on findings; no `cloud`, `host` or `container` profile | 1.10.0 is current; the 1.9.0 of the first draft is older and deprecates User Access Management (3005). A profile adds attributes the platform must then fill or explain; the platform has trace ids and controls, not hosts or cloud accounts |
| O040-005 Extension | No registered extension in 0.3.0; platform facts in `unmapped` with closed keys | Registering an extension takes an identifier from the OCSF project and a review cycle outside this repository; receivers keep `unmapped`. A follow-up can register one without changing a mapping |
| O040-006 The native shape | Stays the default and is never removed; OCSF is a second operation, `GET /audit/ocsf` | A separate operation has a typed OpenAPI schema and generated SDK methods; content negotiation on one path would give one operation two response schemas |
| O040-007 Logs | Only durable records become OCSF; log lines stay OpenTelemetry logs | A log line is lossy by design (ADR-0084); a SIEM fed with lossy security lines would believe a gap is a quiet period |

Architecture decision 0150 is accepted with this design (delegate, Owner decision 2026-11-26-107).

## Critical User Journeys (CUJs)

### CUJ 1: A security team connects its SIEM

*   **Given**: a tenant administrator with `audit:sink:manage` and `audit:read`, a namespace `security` whose egress
    policy allows the SIEM collector at `https://siem.example.net/ingest`, and the collector's token.
*   **When**: they create a sink `{kind: https, homeNamespace: security, url, credential: {header: Authorization, value:
    "Bearer …"}, format: ndjson, gzip: true}` and store the signing key the answer returns once; a minute later a
    developer is granted a role.
*   **Then**: the SIEM receives a User Management event (3007, activity Assign Roles 16, status Success) with the actor's
    subject, the developer's subject as `user.uid`, the trace id, and `metadata.uid` the audit record's id, within
    seconds; the sink shows the id and time of the last record it delivered and the age of its oldest undelivered one;
    the creation of the sink is itself an audit record and an event.

### CUJ 2: The collector is down for an hour

*   **Given**: the sink of CUJ 1.
*   **When**: the collector answers `503` for an hour while 50,000 records are written, and the engine is restarted in
    the middle.
*   **Then**: the sink is `STALLED` after its fifth failure and shows the age of its oldest undelivered record; nothing
    is dropped; when the collector answers again the sink sends the backlog in order, in batches of at most 500 events
    and 1 MiB, from the cursor it had committed; the batch in flight at the restart is sent again, under a new delivery
    id, with the same `metadata.uid` values.

### CUJ 3: A developer adds an audit action

*   **Given**: a change that adds `AuditAction::FooChanged`.
*   **When**: it is built.
*   **Then**: the build fails at the `match` of `audit_ocsf.rs` until the action has a mapping; then the reference test
    fails until `docs/reference/ocsf-mapping.md` is regenerated; then the validation test checks the new action's
    events against the vendored schema.

### CUJ 4: A script tries to escape its sandbox

*   **Given**: the operator sink and a tenant sink.
*   **When**: a script's attempt is stopped by the sandbox and `script.violation` is recorded.
*   **Then**: both sinks receive a Detection Finding (2004, Create) with severity Medium, `status_id` New, `action_id`
    Denied, `finding_info.title` "Sandbox violation", `finding_info.uid` the audit record's id, and the script as
    resource; no value of the script's input or output is in it.

## Technical Plan (Architecture)

### 1. The mapping in the domain

`crates/domain/src/audit_ocsf.rs` holds pure data and no serialization (Principle II):

```rust
pub enum OcsfClass { ApiActivity, UserManagement, EntityManagement, DatastoreActivity, DetectionFinding }
pub struct OcsfMapping { pub class: OcsfClass, pub activity: u8, pub severity: OcsfSeverity, pub status: OcsfStatus }
pub const fn mapping(action: AuditAction) -> OcsfMapping { match action { /* one arm per action, no wildcard */ } }
pub const UNKNOWN: OcsfMapping = /* API Activity 99, severity Unknown, status Unknown */;
```

`OcsfClass` gives `class_uid` and `category_uid` (6003/6, 3007/3, 3004/3, 6005/6, 2004/2); `type_uid` is
`class_uid * 100 + activity_id`, as OCSF defines it. A clippy lint (`clippy::wildcard_enum_match_arm`, denied in this
module) keeps the arms total. `AuditAction::ALL` and `AuditAction::parse` come from the macro that declares the enum.

### 2. The projection and its encoder

`crates/application/src/audit_ocsf.rs` turns an audit entry and its mapping into an `OcsfEvent` (an application type
holding the attributes the five classes use) and applies the disclosure rules of Requirements 040-003 and 040-005.
`crates/application` defines a port `OcsfEncoder`; its one implementation is a new module `crates/telemetry/src/ocsf.rs`
(that crate already depends on `serde_json`), used by the read in `crates/api-http` and by the export pass in the
engine. Per class, the required objects are filled from structured fields only, never from the reason:

| Class | Required object | Filled from |
|---|---|---|
| API Activity 6003 | `api`, `actor`, `src_endpoint` | `api.operation` the dotted action, `api.service.name` "sagat-octopus-orchestrator"; `actor.user.uid` the subject (`type_id` 3 Service for a `system:` subject); `src_endpoint.svc_name` the surface (Gap 040-001) |
| User Management 3007 | `user` | the subject in the resource id: the whole id of a `membership` resource; the part after the namespace of a `payload_reveal_grant` resource (`<namespace>/<subject>`) |
| Entity Management 3004 | `entity` | `type` the resource type, `uid` the resource id |
| Datastore Activity 6005 | `actor`, `src_endpoint`, one of `database` | `database.name` the connection's resource id, `type_id` 99 |
| Detection Finding 2004 | `finding_info` and the `security_control` attributes | `uid` the record id, `title` from a closed table per action, `analytic` `{type_id: 1, name: sandbox, egress or tenant-boundary}`; `evidences[0].actor` the subject; `status_id` 1, `action_id` 2, `disposition_id` 2 |

The role of a membership and the grantee of a secret access exist only in the reason, so they are not filled: the
membership event names the user and not the role, and `secret.access_granted` and `secret.access_revoked` map to API
Activity Update with the secret as resource (appendix). `unmapped` holds `sagat.action` (always) and, per action, the
closed keys the appendix lists. `metadata.profiles` lists `trace` when the event has a trace id, and `security_control`
on findings.

### 3. The read

`GET /audit/ocsf` reuses the query of `listAuditRecords` (same filters, cursor and limit; `audit:read`; withheld actions
excluded; `audit.read` recorded) and answers `{items: [<OCSF event>], nextCursor}` with `Cache-Control: no-store`. The
OpenAPI document gains the operation and a schema `OcsfEvent` that names the base attributes and leaves the class objects
open (`additionalProperties: true`), because the vendored OCSF schema, not OpenAPI, is the contract of their shape.

### 4. The stream: a cursor safe against commit order

Two migrations. The first adds `audit_records.tx_id xid8`, nullable, then `SET DEFAULT pg_current_xact_id()`; neither
rewrites a row (rows written before stay `NULL`, Gap 040-004), and it replaces the runtime role's table-wide `INSERT`
on `audit_records` by a column grant that leaves `tx_id` out, so no writer can choose its own position. The second runs
outside a transaction (sqlx's `-- no-transaction` marker; Task 040-9 verifies the repository's migrator honours it, and
otherwise the two indexes are built by a checked start-up step) and builds, `CONCURRENTLY`, two partial indexes where
`tx_id` is not null: `(tenant_id, tx_id, id)` for tenant sinks and `(tx_id, id)` for the operator sink; it then checks
`pg_index.indisvalid` and fails if either is invalid. Every insert gets the id of its transaction at no extra statement;
each insert adds two index entries (Non-functional requirement 040-001).

A transaction id below the snapshot's `xmin` belongs to a transaction that has ended, so a tenant sink reads:

```sql
SELECT ... FROM audit_records
 WHERE tenant_id = $tenant
   AND tx_id < pg_snapshot_xmin(pg_current_snapshot())
   AND (tx_id, id) > ($cursor_tx, $cursor_id)
   AND NOT (action = ANY($withheld))           -- WITHHELD_FROM_TENANT and audit.sink_delivered
 ORDER BY tx_id, id LIMIT 500
```

and the operator sink reads the same without the tenant predicate and with only `audit.sink_delivered` withheld. No
record can later appear below a position this query passed: every transaction below `xmin` has committed or aborted,
and an aborted one's rows are invisible. A long transaction anywhere in the cluster holds `xmin` back; that delays every
stream and loses nothing (section 6 shows it). `xid8` is 64 bits and does not wrap around within one cluster.

**Where the cursor moves.** To the `(tx_id, id)` of the last record placed in the body sent; when every record read was
withheld, to the last record read. The byte bound (1 MiB) counts the uncompressed body.

**Where a sink starts.** At creation, the cursor is `(pg_snapshot_xmin(pg_current_snapshot()), 00000000-…-0000)` read
then: the sink receives every record committed after its creation, and also those of transactions still running at that
moment, which at most the longest open transaction of the cluster bounds. With `startFrom: {createdAt: t}` on creation
(it needs `audit:read`, which the creator holds, and is audited), the cursor is set below the smallest `tx_id` of the
tenant's records whose `created_at` is at or after `t`: the sink also receives some records created slightly before `t`
(at least once, never fewer).

**A cluster whose transaction ids went back.** After a logical restore or a move by logical replication, new transactions
can have ids below every stored cursor, so new records would sort below them and never be sent. Before each batch, the
pass checks `pg_current_xact_id() < cursor_tx`; when it holds, the sink becomes `CURSOR_INVALID` (audited, alerted), and
resumes from the `created_at` of the last record it delivered minus 10 minutes, mapped as `startFrom` is. The restore
runbook says so, and a test restores into a fresh container.

### 5. Sinks, their states and the export pass

`audit_sinks` (tenant, id, `tenant_id NOT NULL`, home namespace, kind `https`, the sealed URL, the sealed collector
credential, the sealed signing keys (current and, for 24 hours after a rotation, previous), the URL's `scheme://host` and
path fingerprint, format, gzip, ceiling, state, revision, cursor, the time and id of the last delivered record, failures,
`next_attempt_at`, the authorizing subject, the claim token and its expiry) holds tenant sinks. The operator sink's
destination and keys live only in the engine's configuration; its cursor is one row of `audit_operator_export`, keyed by a
constant, inserted if absent and never reset by a start; a change of its configured URL keeps the cursor, and removing
its configuration stops it.

**States and who moves them.**

| State | Entered by | Left by |
|---|---|---|
| `ACTIVE` | creation; a resume; the first success of a `STALLED` sink | any of the others |
| `STALLED` | the pass, after 5 consecutive retryable failures | the pass, at a success |
| `PAUSED` | an administrator (`:pause`) | an administrator (`:resume`, which re-establishes the authorizing subject) |
| `PAUSED_GRANT_REVOKED` | the pass, when the grant check of a `pii` sink fails | `:resume` by a subject whose grant passes, or a change of the ceiling to `internal` |
| `PAUSED_AUTHORITY_LOST` | the pass, when Requirement 040-013's check fails | `:resume` by a subject whose authority passes |
| `CURSOR_INVALID` | the pass, section 4 | the pass, after it resumes from time |
| deleted | an administrator (`DELETE`) | none |

**The pass.** `audit_export` runs in the execution engine on each scheduler tick, on the shards the replica holds
(ADR-0020): a tenant sink belongs to the shard of its tenant, the operator sink to shard 0. Per replica it serves at most
8 sinks at once. For each due sink (`ACTIVE` or `STALLED` and `next_attempt_at` passed):

1. **Claim.** One short transaction takes the sink `FOR UPDATE SKIP LOCKED`, checks the fence, sets a fresh claim token
   with an expiry of 30 seconds, and reads the revision and the cursor. A sink another task holds is skipped.
2. **Check.** The egress rule of the home namespace (or the operator's allowed hosts), the authorizing subject's
   authority (Requirement 040-013) and, for `pii`, the grant (Requirement 040-010). A failure moves the state, audited,
   and ends.
3. **Read and send.** Up to 500 records after the cursor (section 4); for the operator sink, split into one message per
   tenant in cursor order. Encode NDJSON up to 1 MiB uncompressed, gzip when asked; sign the bytes sent (compressed)
   with the platform's delivery scheme (`crates/application/src/signing.rs`, a new direction `AuditExport` whose label
   is `octopus-audit-export-v1`), with the current and, during a rotation, the previous signing key, as webhook
   deliveries sign with two secret versions (`crates/scheduler/src/deliveries.rs`); send through the egress port of the
   home namespace (`crates/infra-egress`) with the headers `x-octopus-signature`, `x-octopus-timestamp`,
   `x-octopus-event-id` (a new UUIDv7 per attempt), `x-octopus-event-type` (`ocsf.batch`) and `x-octopus-delivery-id`
   (the same new UUIDv7), a 10-second timeout, redirects and proxies disabled. A receiver drops duplicates by
   `metadata.uid` only; the batch and delivery ids are new on every attempt, because a retried batch can hold more
   records than the first try.
4. **Commit.** On a `2xx`, one transaction, rechecking the `pii` grant, runs
   `UPDATE audit_sinks SET cursor = $new, last_delivered = … WHERE id = $id AND revision = $r AND claim_token = $t
   AND cursor = $old AND state IN ('ACTIVE','STALLED')`; when no row is updated (the sink was paused, changed or deleted
   meanwhile, or the claim was lost) the batch is discarded and nothing is written. Each batch counts in
   `audit_export_events_total`; the collapsed record `audit.sink_delivered` (no count, one per sink and minute) is
   written in this transaction and is never itself exported.
5. **Failures.** `408`, `429`, `5xx` and transport errors are retryable: exponential back-off from 1 second to 5
   minutes with jitter, `STALLED` after 5 in a row (audited once). Any other `4xx`, and `413`, is permanent for the
   batch: the pass halves the batch and retries at once, down to one record; one record refused permanently is written
   as `audit.sink_record_refused` (the record id), listed by `GET /audit/sinks/{sinkId}/refused`, and passed.

The pass holds no lock across the HTTP call. A crash between the receiver's `2xx` and the commit sends the batch again
on a later tick, under a new delivery id.

### 6. Surfaces

- REST, under `audit:sink:manage` together with `audit:read` (new permission `audit:sink:manage`, granted to the `admin`
  role by the role migration): `GET/POST /audit/sinks`, `GET/PATCH/DELETE /audit/sinks/{sinkId}`,
  `POST /audit/sinks/{sinkId}:pause`, `:resume`, `:rotateSigningKey` (answers the new key once), `:test` (sends one
  synthetic event marked `metadata.labels: ["test"]` and answers a closed outcome, `ok`, `denied`, `timeout` or
  `http_<class>`, never the remote body; at most one per sink per minute; audited), and
  `GET /audit/sinks/{sinkId}/refused`. A sink of another tenant answers `404` exactly as a missing one, audited as a
  cross-tenant denial with a new `ResourceType::AuditSink`. A sink shows `scheme://host`, the path fingerprint, the
  state, the id and time of the last delivered record and the age of the oldest undelivered one; never the cursor, the
  URL path, the credential or a key. Typed errors with `code`, `retryable`, `traceId`.
- Audit actions (seven, each with its mapping in the appendix): `audit.sink_created`, `audit.sink_changed`,
  `audit.sink_deleted`, `audit.sink_paused` (an administrator's pause and every pause by the pass, with the state as a
  closed reason), `audit.sink_resumed`, `audit.sink_delivered`, `audit.sink_record_refused`. The URL never appears in an
  audit reason, a log line or a span.
- Metrics: `audit_export_events_total{kind, outcome}`, `audit_export_batches_total{kind, outcome}`,
  `audit_sinks{state}`, `audit_export_oldest_undelivered_seconds{kind}` (a gauge, the largest over the replica's sinks
  of the age of the oldest record above the cursor), `audit_export_lag_seconds{kind}` (acknowledgement minus
  `created_at`), and `postgres_oldest_xid_holder_seconds` (the age of the oldest backend that holds a transaction id).
  Alert rules: a sink not `ACTIVE` for 15 minutes; the oldest undelivered record older than 15 minutes; the oldest
  transaction-id holder older than 5 minutes.
- Spans: `audit.export.batch` per batch with the sink kind and the count, never the URL or a value.
- Studio: a page under tenant settings to list, create, test, pause, resume and rotate sinks, and to list refused
  records.
- Runbook: `docs/runbooks/audit-export.md` (create a sink, rotate its key and credential, read the oldest undelivered
  record, what each state means, `startFrom`, a restore into another cluster, the operator sink's configuration).

### 7. Rollout

The column, the indexes and the tables are additive. A binary older than 0.3.0 inserts audit rows that get `tx_id`
from the default, so a mixed fleet streams correctly, and a newer action string read by an older binary projects as
unknown (Requirement 040-001). The export pass is always present in a 0.3.0 engine. `POST /audit/sinks` answers
`422 AUDIT_EXPORT_NOT_ACTIVATED` until the operator records the requirement `audit-export-v1` in the instance registry
(ADR-0082), which the registry refuses while a live engine or control plane does not list the capability.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A lossy mapping hides what happened | The native record stays the authority; the dotted action is always in `unmapped.sagat.action`; the mapping is reviewed per action in the appendix |
| The stream misses a record that commits late | The cursor is ordered by transaction id and reads only below the snapshot's `xmin`; Success criterion 040-003 forces the order |
| A long transaction of one tenant, or outside the platform, delays every sink | It delays, never loses; the oldest-undelivered and oldest-transaction gauges and their alerts; idle-in-transaction timeouts on the platform's pools |
| Transaction ids go back after a restore | `CURSOR_INVALID`, resumed from time (section 4) |
| A sink leaks across tenants or shows the cluster's activity | Tenant predicate and withheld actions in the query; the cursor is never shown; Success criterion 040-004 |
| Personal data leaves without authority | The reason is left out below `pii`; `pii` needs the grant in every namespace, checked per batch and at commit; authority rechecked per batch |
| Someone who can read a namespace secret forges or reads the stream | Sink credentials and keys are not namespace secrets and are never delivered |
| A receiver is flooded by duplicates | A batch is resent once per failed commit or lost claim; `metadata.uid` dedupes |
| One bad record blocks a sink | Halving isolates it; it is recorded as refused and passed |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | The audit record is written in its transaction as today; a sink's cursor advances in one transaction that compares revision, claim, cursor and state (section 5) |
| II. Modular domain, ports and adapters | Pass | The mapping is pure data in `crates/domain`; the projection in `crates/application`; the encoder and the HTTP client in adapters |
| III. External effects are at-least-once; sagas compensate | Pass | Delivery to a sink is at-least-once with a stable `metadata.uid` |
| IV. Compatibility is versioned and pinned | Pass | OCSF 1.10.0 pinned and vendored; an additive OpenAPI operation; the native read unchanged; a mixed fleet streams (section 7) |
| V. Code, tenant boundary and secrets are isolated | Pass | Tenant-scoped queries and sinks; the operator sink only from configuration, one tenant per message; sink credentials sealed and never delivered; authority and grant rechecked per batch; no tenant or sink id as a metric label |
| VI. Observability and audit are part of the feature | Pass | Every sink act audited; metrics with closed labels; spans per batch; alerts on states, backlog and held transactions |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass with conditions | The stream is read from the audit table by a commit-safe cursor instead of an outbox row per record; the audit table is the backlog and the dead-letter store of a refused record (Complexity Tracking) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Validation of every action against the vendored schema and by the OCSF server; the forced commit-order test, the crash hook, the state races, the cross-tenant battery and the restore test |
| Delivery Constraints | Pass | PostgreSQL as authority, no new component; the latency target is measured on a stated environment; the failover window is reported (Gap 040-005) |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| `tx_id xid8` and a cursor below `xmin` instead of an outbox row per audit record | A stream must not miss a late commit, and an outbox adds a write to every audited transaction | An outbox row per record: a second write in every audited transaction for the same guarantee. Paging by `created_at`: misses late commits |
| The audit table as the dead-letter store | A refused record must stay visible and replayable without a second copy | A dead-letter table: a copy of a record that is already kept for ever |
| A sink table, a pass and a state machine | Push is what SIEMs take; a pass racing an administrator must not undo the administrator | Read only: every customer writes a poller with the same commit-order problem |
| Sealed sink-owned credentials | A namespace secret can be delivered to tasks and webhooks, which would let others forge or read the stream | Namespace secret references |
| A separate read operation | A typed schema per operation | Content negotiation on `GET /audit`: one operation, two schemas |

## Alternatives Considered

- **Storing audit records natively in OCSF.** It ties the authority to an external schema's versions and rewrites every
  writer; a projection gives the same events.
- **A vendor format (CEF, LEEF, ECS).** OCSF is the one the owner named, is vendor-neutral and is what the security data
  lakes read.
- **An outbox row per audit record.** Rejected above.
- **Logical decoding (a replication slot) of `audit_records`.** Ordered by commit, but a slot that a stalled sink does not
  consume holds write-ahead log on the primary without bound, a risk to the authority itself.
- **OCSF Detection Findings for every denial.** A SIEM already alerts on failed API activity; only the three platform
  controls are findings.

## Non-Functional Requirements

See Requirements, Non-functional (Non-functional requirements 040-001 to 040-004).

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Three slices, each a pull request, in this order. Slice 1 touches `crates/domain`, `crates/application`,
`crates/telemetry` and `crates/api-http`; slice 2 touches `crates/infra-postgres`, `crates/scheduler`,
`apps/execution-engine` and `crates/application/src/signing.rs`; slice 3 touches the Studio and the documents.

- [ ] T040-1 Decide Owner questions 040-001 to 040-007 and write the design out; record the delegate rows.
  Refs: Requirement 040-001
- [ ] T040-2 Vendor the JSON Schema the OCSF 1.10.0 server exports for each class used with the declared profiles under
  `crates/telemetry/ocsf/1.10.0/` with a `SHA256SUMS` file and a test that the files match it; pin the OCSF server image
  by digest for the contract job.
  Refs: Requirement 040-004
- [ ] T040-3 Add `audit_ocsf.rs` to the domain with the mapping of every action (the appendix) and no wildcard arm;
  `AuditAction::ALL` and `parse` from the enum's macro with a round-trip test; the unknown mapping.
  Refs: Requirement 040-001, Requirement 040-007
- [ ] T040-4 Add the projection in the application with the disclosure rules (scrubbed reason; reason left out below
  `pii`; caps and the truncation marker; required objects from structured fields only; `unmapped` keys closed per
  action) and unit tests per class.
  Refs: Requirement 040-003, Requirement 040-005, Requirement 040-006
- [ ] T040-5 Add the encoder in `crates/telemetry/src/ocsf.rs` and a test that validates the events of every action
  (the fixture variants of Success criterion 040-001) against the vendored schema, with the `at_least_one` constraints
  checked in code; a hand mutation that drops one required object fails it.
  Refs: Requirement 040-003, Success criterion 040-001, Success criterion 040-005
- [ ] T040-6 Validate one sample per class with the pinned OCSF server's validation API in the contract job.
  Refs: Success criterion 040-002
- [ ] T040-7 Add `GET /audit/ocsf` with the OpenAPI operation and schema, the same permission, filters, withheld actions
  and self-audit as `GET /audit`, and the contract baseline.
  Refs: Requirement 040-002
- [ ] T040-8 Generate `docs/reference/ocsf-mapping.md` from the table and a test that fails when it is behind.
  Refs: Requirement 040-012, Success criterion 040-005
- [ ] T040-9 Write the two migrations: `audit_records.tx_id` (nullable, then the default, no rewrite) with the column
  grant that leaves it out; the two partial indexes built concurrently outside a transaction with the `indisvalid`
  check (verify the migrator's `-- no-transaction` support first); `audit_sinks` with `tenant_id NOT NULL`,
  `audit_operator_export`, grants, classification entries (the sealed columns as `secret`), a down migration that
  refuses while a sink exists, and the permission `audit:sink:manage` for `admin`.
  Refs: Requirement 040-008, Requirement 040-009, Non-functional requirement 040-001
- [ ] T040-10 Add the sink resource to the REST API (create with `startFrom`, change, pause, resume, rotate, test,
  delete, refused records) with the egress check, the sealed credentials, the signing key returned once, the `pii` and
  authority checks, the limit of 4 under the tenant's lock, the `404` of another tenant's sink, and the audit actions.
  Refs: Requirement 040-008, Requirement 040-010, Requirement 040-013
- [ ] T040-11 Add the `audit_export` pass: claim, checks, the cursor query below `xmin` with the withheld actions, the
  batch and byte bounds, the direction `AuditExport` in `signing.rs` with known test vectors, two signing keys during a
  rotation, new delivery ids per attempt, the guarded commit, the back-off, halving and refused records, `STALLED`,
  `PAUSED_GRANT_REVOKED`, `PAUSED_AUTHORITY_LOST`, `CURSOR_INVALID`.
  Refs: Requirement 040-009, Requirement 040-010, Requirement 040-013
- [ ] T040-12 Add the operator sink from the engine's configuration (allowed hosts, one tenant per message, its cursor
  row, its evidence in logs and metrics) and document its configuration.
  Refs: Requirement 040-011
- [ ] T040-13 Fault and race tests: the forced commit order and the crash hook of Success criterion 040-003; the state
  races of Success criterion 040-006; a restore into a fresh container ends `CURSOR_INVALID` and resumes.
  Refs: Success criterion 040-003, Success criterion 040-006, Requirement 040-009
- [ ] T040-14 Budget: an audited transaction issues the same statements as before the migration; write-ahead log bytes
  and insert latency of 10,000 audit inserts before and after, reported.
  Refs: Non-functional requirement 040-001
- [ ] T040-15 Cross-tenant and privilege tests for the read, the sink resource and the pass, also as the runtime role,
  including the probe of Success criterion 040-004 and an insert that names `tx_id` refused by the column grant.
  Refs: Success criterion 040-004
- [ ] T040-16 Metrics, spans, the three alert rules with their rule tests in `infra/observability/tests/`, and the
  capability `audit-export-v1` in the instance registry with the `422` before activation.
  Refs: Requirement 040-009, Non-functional requirement 040-004
- [ ] T040-17 Measure Non-functional requirement 040-003 on a stated environment and write the dated report in
  `specs/040-ocsf-event-standard/validation-report.md`.
  Refs: Non-functional requirement 040-003
- [ ] T040-18 Studio: the sinks page (list, create, test, pause, resume, rotate, refused records, state and backlog).
  Refs: Requirement 040-008
- [ ] T040-19 Write the runbook, the quickstart of CUJ 1 with the failure scenario of CUJ 2, and update
  `docs/traceability.md`, the roadmap row and the release notes.
  Refs: Requirement 040-012

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the three
checkers, the unit tests of the crates it changes and the suites it touches, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`
where it adds a statement. A test is seen failing for the right reason before the code that passes it; races use
barriers and fault hooks, never sleeps.

- Slice 1: the domain and projection unit tests; the validation of every action against the vendored schema (Success
  criterion 040-001); the OCSF server samples; the OpenAPI baseline; the reference drift test; the hand mutation of Task
  040-5.
- Slice 2: a new target `integration_ocsf_export` on real PostgreSQL with a local HTTPS receiver; the tests of Task
  040-13; the budget of Task 040-14; the cross-tenant battery as both roles; the migrations' up, down and privilege
  audit; the hand mutation `pg_snapshot_xmin` into `pg_snapshot_xmax`.
- Slice 3: the Studio's `npm run api && npm run check:api` and its unit tests; the quickstart run end to end including
  the outage of CUJ 2.

## Appendix: the mapping of every audit action (2026-11-26)

Classes: **API** = API Activity 6003 (activities 1 Create, 2 Read, 3 Update, 4 Delete, 99 Other; 5 List is not used);
**UM** = User Management 3007 (14 Assign Privileges, 15 Remove Privileges, 16 Assign Roles); **EM** = Entity Management
3004 (1 Create, 3 Update, 4 Delete, 6 Enroll, 7 Unenroll, 8 Enable, 9 Disable, 99 Other); **DS** = Datastore Activity
6005 (1 Read, 5 Write, 8 List, 99 Other); **DF** = Detection Finding 2004 (1 Create, with the attributes of Requirement
040-007). Severity is Informational (1) unless the row says Low (2) or Medium (3). Status is Success (1) unless the row
says Failure (2); a finding's status is New (1). Every event carries `unmapped.sagat.action`.

| Actions | Class, activity | Severity, status |
|---|---|---|
| `tenant.provisioned`, `workspace.created`, `namespace.created` | EM 1 | |
| `membership.granted` | UM 16 | |
| `payload.reveal_granted` | UM 14 | Low |
| `secret.access_granted`, `secret.access_revoked` (the grantee is only in the reason, so the secret is the resource) | API 3 | Low |
| `payload.reveal_revoked` | UM 15 | Low |
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
| `script.violation` | DF 1 (`analytic.name` `sandbox`, title "Sandbox violation") | Medium, New |
| `egress.denied` | DF 1 (`analytic.name` `egress`, title "Egress denied") | Medium, New |
| `access.denied.cross_tenant` | DF 1 (`analytic.name` `tenant-boundary`, title "Cross-tenant access denied"); operator sink only | Medium, New |
| `audit.sink_created` | EM 1 | |
| `audit.sink_changed` | EM 3 | |
| `audit.sink_deleted` | EM 4 | |
| `audit.sink_paused` | EM 9 | Low |
| `audit.sink_resumed` | EM 8 | |
| `audit.sink_delivered` (never exported) | API 99 | |
| `audit.sink_record_refused` | API 99 | Low, Failure |

The table covers the 127 values of `AuditAction::as_str` on 2026-11-26 and the seven actions this design adds, 134
in all; Task 040-3 turns it into the `match`, which the lint holds total. A reviewer who finds an action missing here finds it missing in the code too.
