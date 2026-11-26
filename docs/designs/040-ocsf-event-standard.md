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
  status, kept in one table in the application (`crates/application/src/audit_ocsf.rs`; OCSF's numeric codes belong to an
  external schema, not to the domain) whose `match` over `AuditAction` has no wildcard arm. `AuditAction` (in the domain)
  gains `ALL` and `parse` generated by one macro with the enum, with a round-trip test, so
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
  is capped: an identifier at 1,024 characters, ending with `…[truncated]` when cut; the reason by `scrub` (2,048
  characters, with its own marker `…[N characters cut]`).
- **Requirement 040-004** The OCSF version MUST be pinned (1.10.0) and stated in every event. The JSON Schema that the
  OCSF server of that version exports for each class used, with the declared profiles, MUST be vendored in the
  repository with its SHA-256 recorded; moving to another version is a new architecture decision with the validation
  suite run again, never a side effect of a dependency update.
- **Requirement 040-005** An OCSF event MUST disclose nothing that the native record of the same reader does not: no
  payload, no business identifier, no secret value, no field of another tenant, no value parsed out of the free-text
  reason. The reason MUST be scrubbed as `GET /audit` scrubs it (`sagat_telemetry::scrub`), and MUST be left out of an
  event sent to a sink whose ceiling is below `pii` (Requirement 040-010).
- **Requirement 040-006** The platform facts that OCSF has no attribute for MUST go in `unmapped`, as OCSF prescribes
  for a producer without a registered extension, as one nested object `{"sagat": {"action": <dotted action>,
  "region_epoch": <integer>}}`, with a third key `tombstone` only on the class-0 tombstone event; the resource type is
  `resources[0].type`.
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
  path. At most 4 sinks per tenant, held by four slots (Technical Plan 6). Every act is audited.
- **Requirement 040-009** A sink MUST receive every record of its scope (its tenant's records less the withheld
  actions and the `audit.sink_delivered` records; for the operator sink every tenant's records less
  `audit.sink_delivered`) that commits after the sink's start, at least once, in the order of the export cursor, with
  `metadata.uid` stable across redeliveries so that a receiver drops a duplicate by it. A sink that keeps failing MUST
  NOT skip records: it backs off and becomes `STALLED`, and a refusal by the destination as a whole (`401`, `403`,
  `404`, `405`, `410`, an egress refusal) pauses it with no record passed. A record the receiver refuses permanently,
  alone in its batch while the receiver accepted a sibling half, is the one exception: it is recorded as refused
  (`audit.sink_record_refused`, the record id and the sink), listed by the API, and passed; it stays in the audit table,
  which is its dead-letter store (Principle VII).
- **Requirement 040-010** The uniform rule of Owner question 1081 (decided 2026-11-24, delegate Owner decision
  2026-11-24-4, Decision 1617 of feature 020) MUST govern what leaves through a tenant sink: the ceiling defaults to
  `internal`; a `pii` ceiling is set only by an authorizing subject who holds the effective reveal grant in every
  namespace of the tenant (`has_effective_payload_reveal_grant`, as the step-log quota rule reads it for data that spans
  the tenant), checked when it is set, before each batch and again in the transaction that advances the cursor; when it
  fails the sink is `PAUSED_GRANT_REVOKED` and at most the batch already in flight has left; every batch is counted in
  metrics and every pause audited; `secret` never leaves. The operator sink's ceiling is `internal`, fixed.
- **Requirement 040-011** The operator of the deployment MAY configure one operator sink through the execution
  engine's configuration (never through the API): its URL, collector credential, signing key and allowed hosts are held
  in memory from that configuration; it persists a fingerprint of that configuration and its cursors, never the
  destination or a credential. Each message it sends holds the records of one
  tenant, naming it in `metadata.tenant_uid`, so that messages keep tenant scope (Principle V). Its own evidence
  (deliveries, stalls, refusals) goes to the engine's logs and metrics, never into a tenant's trail.
- **Requirement 040-012** The mapping MUST be published as reference documentation generated from the table
  (`docs/reference/ocsf-mapping.md`), and a test MUST fail when the page is behind the table.
- **Requirement 040-013** Before each batch, the pass MUST check the authorizing subject's current authority: membership
  of the tenant, `audit:sink:manage` and `audit:read`. When it fails, the sink is `PAUSED_AUTHORITY_LOST`, audited.
  Any change of the URL, the credential, the home namespace or the ceiling, any rotation of the signing key, and any
  resume makes the person who does it the authorizing subject. On a `pii` sink such a request is refused with `403 PII_CEILING_NOT_PERMITTED` unless the
  person holds the grant of Requirement 040-010 or the same request sets `ceiling: internal`. A change of the URL or the
  credential leaves `PAUSED_DESTINATION_REFUSED`; a change of the ceiling to `internal` leaves `PAUSED_GRANT_REVOKED`;
  a resume by a subject whose checks pass leaves any paused state.

### Non-functional

- **Non-functional requirement 040-001** Writing an audit record MUST add no statement, row, lock or trigger: one 8-byte
  column filled by a default and two entries in partial indexes. The write-ahead log bytes and the insert latency of
  10,000 audit inserts are measured before and after the migration and reported (Task 040-14).
- **Non-functional requirement 040-002** `GET /audit/ocsf` MUST run the same single query as `GET /audit` for the same
  filters and page size, and build each event in memory in bounded time (no query per record).
- **Non-functional requirement 040-003** With one sink and an endpoint that answers within 100 ms, the lag of a record
  (its acknowledgement time minus its `created_at`, which the application sets before the commit; the clocks of the
  engine and the control plane are assumed within 1 second) MUST be at most 10 seconds at the 99th percentile, at 200
  audit records per second per tenant, on a stated environment (a target, measured by Task 040-17); for the operator sink, with 50 tenants writing
  interleaved at 4 records per second each, the same bound.
- **Non-functional requirement 040-004** No tenant, namespace or sink identifier is a metric label (Principle V); the
  export metrics carry the sink kind, the state and the outcome only.

### Success criteria

- **Success criterion 040-001** For every audit action, records built by fixtures (with and without a trace id, with
  and without a reason, under both ceilings, with a `system:` actor) are projected and validated against the vendored
  JSON Schema, and pass: 134 of 134 when this design is built, and every action added later; an unknown action string
  projects and validates too.
- **Success criterion 040-002** A sample of each class used (API Activity, User Management, Entity Management, Detection
  Finding) is accepted by the validation API of the pinned OCSF server image, run by digest in the
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
  after the change committed except the one already in flight. An endpoint held longer than the claim's expiry never
  lets a second replica send the same range while the first still renews (the claim made to lapse by a fault hook
  between the renewal and the send); a `401` on 1,000 records passes none, and nor does a `400` on 1,000 records after an
  earlier `2xx`; a failure write and a renewal racing an administrator's pause leave the sink paused.
- **Success criterion 040-007** A restore into a fresh container with the epoch step of the runbook delivers a record
  written after the sink's last delivery and before the dump; without the step, every sink reports `CURSOR_INVALID`.
  Idle sinks consume no transaction id: N idle sinks over M ticks, measured.

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

- **Gap 040-001** The audit record stores neither the caller's network address nor the surface that served the act, so
  `src_endpoint` (required by API Activity) carries the constant `svc_name: "sagat"`. Storing the address is a privacy
  decision of its own, not taken here.
- **Gap 040-002** Authentication failures are logged and counted, not audited, so OCSF Authentication (3002) events
  come only when design 013 (federated login) audits sign-ins.
- **Gap 040-003** Amazon Security Lake takes OCSF as Parquet in a bucket; the object-store sink is a follow-up.
- **Gap 040-004** The records written before the migration have no transaction id and are not streamed; they stay
  readable through `GET /audit/ocsf`, and a sink's `startFrom` reaches only records that have one.
- **Gap 040-005** The tombstone after a failover that lost work (section 5) needs feature 032's region epoch and the
  window of its failover report; until feature 032 is built there is one region, epoch 1, and nothing to tombstone.
- **Gap 040-006** `trace.uid` may be chosen by the caller (`traceparent`), so unrelated events can share it; the
  reference says so.
- **Gap 040-007** On a managed service where `pg_resetwal` is not available, only a physical restore keeps the export;
  after a logical restore there, sinks are recreated with `startFrom`.
- **Gap 040-008** `startFrom` and `rebase` read a 10-minute window; a record of a transaction that lived longer than 10
  minutes can fall below the cursor. The platform's pools bound statements, not transactions; the oldest
  transaction-id holder is alerted at 5 minutes. A stated residual risk.

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

### 1. The mapping and the projection, in the application

The domain gains only `AuditAction::ALL`, `AuditAction::parse` and the seven new actions. Everything OCSF lives in
`crates/application/src/audit_ocsf.rs`, which already may use `serde` and `serde_json` (`crates/application/Cargo.toml`):

```rust
enum OcsfClass { ApiActivity, UserManagement, EntityManagement, DetectionFinding }
enum EventStatus { Unknown = 0, Success = 1, Failure = 2 }   // `status_id` of an activity class
enum FindingStatus { New = 1 }                                // `status_id` of a finding, a different OCSF enum
enum Status { Event(EventStatus), Finding(FindingStatus) }
enum OcsfSeverity { Unknown = 0, Informational = 1, Low = 2, Medium = 3 }
struct OcsfMapping { class: OcsfClass, activity: u8, severity: OcsfSeverity, status: Status }
const fn mapping(action: AuditAction) -> OcsfMapping { match action { /* one arm per action, no wildcard */ } }
const UNKNOWN: OcsfMapping = /* API Activity 99, severity Unknown, status Unknown */;
#[derive(Serialize)] struct OcsfEvent { /* the attributes the four classes use; class 0 for the tombstone follows */ }
```

`OcsfClass` gives `class_uid` and `category_uid` (6003/6, 3007/3, 3004/3, 2004/2); `type_uid` is
`class_uid * 100 + activity_id`, as OCSF defines it. A clippy lint (`clippy::wildcard_enum_match_arm`, denied in this
module) keeps the arms total. `OcsfEvent` serializes itself; there is no encoder port and no adapter mapping, because
the event has no meaning apart from its JSON. The vendored schema and the validation test sit in
`crates/application/tests/` and `crates/application/ocsf/1.10.0/`.

### 2. Filling the classes

The projection applies the disclosure rules of Requirements 040-003 and 040-005. Per class, the required objects are
filled from structured fields only, never from the reason:

| Class | Required object | Filled from |
|---|---|---|
| API Activity 6003 | `api`, `actor`, `src_endpoint` | `api.operation` the dotted action, `api.service.name` "sagat-octopus-orchestrator"; `actor.user.uid` the subject (`type_id` 3 Service for a `system:` subject); `src_endpoint.svc_name` the constant `sagat` (Gap 040-001) |
| User Management 3007 | `user` | the subject in the resource id: the whole id of a `membership` resource; the part after the first `/` of a `payload_reveal_grant` resource (`<namespace uuid>/<subject>`; a UUID has no `/`) |
| Entity Management 3004 | `entity` | `type` the resource type, `uid` the resource id |
| Detection Finding 2004 | `finding_info` and the `security_control` attributes | `uid` the record id, `title` from a closed table per action, `analytic` `{type_id: 1, name: sandbox, egress or tenant-boundary}`; `evidences[0].actor` the subject; `status_id` 1, `action_id` 2, `disposition_id` 2 |

The role of a membership and the grantee of a secret access exist only in the reason, so they are not filled: the
membership event names the user and not the role, and `secret.access_granted` and `secret.access_revoked` map to API
Activity Update with the secret as resource (appendix). `unmapped` holds `sagat.action` and `sagat.region_epoch`
and nothing else. `metadata.profiles` lists `trace` when the event has a trace id, and `security_control` on findings.
The projection is `project(row: &AuditRow, ceiling: Ceiling, epoch: u64) -> OcsfEvent`, where `AuditRow` is the stored
record with its tenant id; the read passes the `pii` ceiling (it shows the scrubbed reason, as `GET /audit` does), and
the product version is the binary's `CARGO_PKG_VERSION` cut to major and minor. The reason is scrubbed by `scrub`,
whose own cut and marker (`…[N characters cut]`) apply. One event, as sent:

```json
{"class_uid": 3007, "category_uid": 3, "activity_id": 16, "type_uid": 300716, "severity_id": 1, "status_id": 1,
 "time": 1795651200000, "metadata": {"uid": "<record id>", "tenant_uid": "<tenant id>", "version": "1.10.0",
 "product": {"name": "Sagat Octopus Orchestrator", "vendor_name": "Sagat", "version": "0.3"}, "profiles": ["trace"]},
 "actor": {"user": {"uid": "<subject>"}}, "user": {"uid": "<member subject>"},
 "resources": [{"type": "membership", "uid": "<member subject>"}], "trace": {"uid": "<trace id>"},
 "unmapped": {"sagat": {"action": "membership.granted", "region_epoch": 1}}}
```

### 3. The read

`GET /audit/ocsf` reuses the query of `listAuditRecords` (same filters, cursor and limit; `audit:read`; withheld actions
excluded; `audit.read` recorded) and answers `{items: [<OCSF event>], nextCursor}` with `Cache-Control: no-store`. The
OpenAPI document gains the operation and a schema `OcsfEvent` that names the base attributes and leaves the class objects
open (`additionalProperties: true`), because the vendored OCSF schema, not OpenAPI, is the contract of their shape.

### 4. The stream: a cursor safe against commit order

Two migrations. The first adds `audit_records.tx_id xid8`, nullable, then `SET DEFAULT pg_current_xact_id()`; neither
rewrites a row (rows written before stay `NULL`, Gap 040-004), and it replaces the runtime role's table-wide `INSERT`
on `audit_records` by a column grant that leaves `tx_id` out, so no writer can choose its own position. Three more
migration files follow, each one statement marked `-- no-transaction` (sqlx sends a file as one query, and PostgreSQL
refuses `CREATE INDEX CONCURRENTLY` in a query of several statements): the partial index `(tenant_id, tx_id, id)` where
`tx_id` is not null, for tenant sinks; the partial index `(tx_id, id)`, for the operator sink; and a `DO` block that
fails unless both are valid in `pg_index.indisvalid`. Every insert gets the id of its transaction at no extra statement;
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

and each of the operator sink's per-tenant streams reads the same with only `audit.sink_delivered` withheld (section 5);
its discovery of tenants reads the same query without the tenant predicate. No
record can later appear below a position this query passed: every transaction below `xmin` has committed or aborted,
and an aborted one's rows are invisible. A long transaction anywhere in the cluster holds `xmin` back; that delays every
stream and loses nothing (section 6 shows it). `xid8` is 64 bits and does not wrap around within one cluster.

**Where the cursor moves.** To the `(tx_id, id)` of the last record placed in the body sent (withheld records are not
read, so they never hold it back). The byte bound (1 MiB) counts the uncompressed body.

**Where a sink starts.** At creation, the cursor is `(pg_snapshot_xmin(pg_current_snapshot()), 00000000-…-0000)` read
then: the sink receives every record committed after its creation, and also those of transactions still running at that
moment, which at most the longest open transaction of the cluster bounds. With `startFrom: {createdAt: t}` on creation
(it needs `audit:read`, which the creator holds, and is audited; `t` is at most 90 days back), the cursor is set below
the smallest `tx_id` of the tenant's records whose `created_at` lies in `[t, t + 10 minutes]`, a window longer than any
transaction the platform lets live (the oldest transaction-id holder is alerted at 5 minutes), read through the
`(tenant_id, created_at)` index over that window only, or below the first record after `t` when the window is empty:
the sink also receives some records created slightly before `t` (at least once, never fewer).

**A cluster whose transaction ids went back.** A logical restore (`pg_dump` and `pg_restore`, the procedure of
`docs/runbooks/control-plane.md`) or a move by logical replication copies `tx_id` and the cursors as data, while the new
cluster's counter starts lower: new records would sort below every cursor. Every cursor therefore records the cluster it
was read on (`cursor_cluster`, the cluster's `system_identifier`, read through a `SECURITY DEFINER` function of the
owner role that returns only that number, so that the runtime role needs no grant on `pg_control_system()`). When the pass finds a cursor of another
cluster, the sink becomes `CURSOR_INVALID` (audited, alerted; for the operator sink, logged), and stays so, whatever the
counter does, until the operator runs `control-plane audit-export rebase`. A logical restore MUST advance the new
cluster's transaction-id epoch above the largest `tx_id` before loading (`pg_resetwal --epoch`, on the stopped, empty
cluster), so that every new record sorts above every restored one; `rebase` refuses while
`max(tx_id) >= pg_snapshot_xmax(pg_current_snapshot())`, which catches a restore without the step. Then `rebase` sets
each invalid cursor, the sinks', the operator streams' and the operator's discovery cursor, from the time of its last
delivered record minus 10 minutes (or the sink's creation time when it has delivered nothing), with `startFrom`'s rule,
and records the new cluster: nothing is skipped and some records are sent twice. On a managed service where
`pg_resetwal` is not available, only a physical restore (which keeps the counter) keeps the export (Gap 040-007). A test
restores into a fresh container with the epoch step, sees `CURSOR_INVALID` until `rebase`, runs it, and sees a record
written after the last delivery and before the dump arrive, and a record written after the restore arrive; without the
step, `rebase` refuses.

**Who serves which sink, and what makes a commit correct.** A tenant sink is served by the replica that holds the shard
of its tenant (`Shard::of` keyed by the tenant: ADR-0020 keys run work by run, and tenant keying is acceptable here
because a tenant has at most 4 sinks); the operator sink by the replica that holds shard 0. The shard only chooses the
replica; the correctness of a commit comes from the claim token and the compare-and-set of section 5, not from the
fence, because a pass that lost its shard still holds a claim that only expiry can take from it.

### 5. Sinks, their states and the export pass

`audit_sinks` (tenant, id, `tenant_id NOT NULL`, home namespace, kind `https`, the sealed URL, the sealed collector
credential, the sealed signing keys (current and, for 24 hours after a rotation, previous), the URL's `scheme://host` and
path fingerprint, format, gzip, ceiling, state, revision, cursor, the time and id of the last delivered record, failures,
`next_attempt_at`, the authorizing subject, the claim token and its expiry) holds tenant sinks. The operator sink's
destination and keys live only in the engine's configuration; its fingerprint and discovery cursor live in
`audit_operator_config` and its per-tenant cursors in `audit_operator_export` (section 5, "The operator sink"); removing
its configuration stops it.

**States and who moves them.**

| State | Entered by | Left by |
|---|---|---|
| `ACTIVE` | creation; a resume; the first success of a `STALLED` sink | any of the others |
| `STALLED` | the pass, after 5 consecutive retryable failures | the pass, at a success |
| `PAUSED` | an administrator (`/pause`) | an administrator (`/resume`, which re-establishes the authorizing subject) |
| `PAUSED_GRANT_REVOKED` | the pass, when the grant check of a `pii` sink fails | `/resume` by a subject whose grant passes, or a change of the ceiling to `internal` |
| `PAUSED_AUTHORITY_LOST` | the pass, when Requirement 040-013's check fails | `/resume` by a subject whose authority passes |
| `PAUSED_DESTINATION_REFUSED` | the pass, at a `401`, `403`, `404`, `405`, `410` or an egress refusal | `/resume` by a subject whose authority passes, or a change of the destination |
| `CURSOR_INVALID` | the pass, at a cursor read on another cluster (section 4) | `control-plane audit-export rebase` |
| deleted | an administrator (`DELETE`) | none |

**The pass.** `audit_export` runs in its own loop of the execution engine (`run_audit_export` beside `run_deliveries`,
`run_schedules` and `run_batches` in `apps/execution-engine/src/loops.rs`), every second, gated by `while_serving`; the
scheduler's tick never waits on it, so a slow collector cannot delay a timer or fail liveness. Per replica it serves at
most 8 sinks at once. A sink is due when it is `ACTIVE` or `STALLED`, its `next_attempt_at` has passed and its claim, if
any, has expired. For each due sink:

1. **Look.** A read-only statement (no transaction id assigned) checks whether a record of its scope exists above the
   cursor and below `xmin`. When none does, the replica remembers in memory not to look again at that sink for 5
   seconds, and nothing is written.
2. **Claim.** One short transaction takes the sink `FOR UPDATE SKIP LOCKED`, sets a fresh claim token with an expiry of
   30 seconds, and reads the revision and the cursor. A sink another task holds is skipped.
3. **Check.** The egress rule of the home namespace (or the operator's allowed hosts), the authorizing subject's
   authority (Requirement 040-013) and, for `pii`, the grant (Requirement 040-010). A failure moves the state, audited,
   and ends.
4. **Read and send.** Up to 500 records after the cursor (section 4). Encode NDJSON up to 1 MiB uncompressed, gzip when
   asked; sign the bytes sent (compressed) with the platform's delivery scheme (`crates/application/src/signing.rs`, a new
   direction `AuditExport` whose label is `octopus-audit-export-v1`), with the current and, during a rotation, the
   previous signing key, as webhook deliveries sign with two secret versions (`crates/scheduler/src/deliveries.rs`). The
   covered fields of `signing::Covered` are: the event id, the attempt's delivery id (a new UUIDv7); the event type,
   `ocsf.batch`; the subject, the tenant id; the correlation key, empty. A tenant sink sends through the existing port
   `WebhookSender` (`crates/application/src/outbound.rs`, `OutboundRequest` with the tenant and the home namespace,
   implemented by `sagat-infra-connectors` over the egress client); the operator sink through a new port
   `AuditExportSender` that takes an egress policy built from the operator's allowed hosts, implemented in the same
   adapter; both map the answer onto `SendFailure` as section 5's step 6 classes it. The headers are those of the wire
   contract (`specs/040-ocsf-event-standard/contracts/audit-export.asyncapi.yaml`, written before the code, Task
   040-20): `x-octopus-signature`, `x-octopus-timestamp`, `x-octopus-event-id` and `x-octopus-delivery-id` (both the
   attempt's delivery id) and `x-octopus-event-type` (`ocsf.batch`); a 10-second timeout, redirects and proxies
   disabled. A receiver drops
   duplicates by `metadata.uid` only; the batch and delivery ids are new on every attempt, because a retried batch can
   hold more records than the first try. Before each request the claim is renewed (its expiry moved 30 seconds ahead,
   `WHERE claim_token = $t AND revision = $r AND state IN ('ACTIVE','STALLED')`); a failed renewal ends the work and
   sends nothing more. Every write of the pass (renewal, cursor commit, failure count, stall, the paused states,
   `record_refused`, the release) carries the same predicate on the claim token, the revision and the state; every act
   of an administrator raises the revision and clears the claim, so that no write of a pass begun earlier can undo it;
   at the end of its work the pass releases its claim (`claim_token = NULL WHERE claim_token = $t`).
5. **Commit after each accepted request.** On a `2xx`, one transaction, rechecking the `pii` grant, runs
   `UPDATE audit_sinks SET cursor = $new, last_delivered = … WHERE id = $id AND revision = $r AND claim_token = $t
   AND cursor = $old AND state IN ('ACTIVE','STALLED')`; when no row is updated (the sink was paused, changed or deleted
   meanwhile, or the claim was lost) the work is discarded and nothing is written. The `pii` recheck is a new store
   function, `holds_reveal_grant_in_every_namespace(tx, tenant, subject)`, one statement on one snapshot with no fence,
   as the step-log quota's `step_log_usage_if_revealed` reads; when it fails, the same transaction still advances the
   cursor (the batch has left), sets `PAUSED_GRANT_REVOKED` and writes `audit.sink_paused`. The collapsed record
   `audit.sink_delivered` (no count, one per sink and minute) is written in this transaction and is never itself
   exported, written only for a `pii` sink, where the rule of Requirement 040-010 asks for the audit of its deliveries;
   counts go to metrics for every sink.
6. **Failures.**
   - `408`, `429`, `5xx`, a transport error and a key-authority error (a sealed credential that cannot be opened now) are
     retryable: exponential back-off from 1 second to 5 minutes with jitter, `STALLED` after 5 in a row (audited once).
   - `401`, `403`, `404`, `405`, `410` and an egress refusal are failures of the destination, not of a record: the sink
     becomes `PAUSED_DESTINATION_REFUSED`, audited and alerted, and no record is passed.
   - `400`, `413` and `422` are failures of the batch: the pass splits it, left half first, depth first, and commits the
     cursor only over a contiguous accepted prefix. When the leftmost unresolved part is one record that the receiver
     refuses alone, it is passed as refused only if a sibling part of the same split was accepted (Requirement 040-009):
     `audit.sink_record_refused` (the record id) commits in the transaction that moves the cursor past it, and
     `GET /audit/sinks/{sinkId}/refused` lists it; the pass then goes on with the rest. When no part of a split is
     accepted, or a sink has refused 10 records within an hour, the sink becomes `PAUSED_DESTINATION_REFUSED`. A sink's
     own `audit.sink_*` records (resource the sink itself) are left out of that sink's scope, so a refusal never feeds
     itself.
   - `SendFailure::Invalid` (the request could not be built for the destination) is a failure of the destination; a
     `Retry-After` on `429` or `503` is honoured within the 5-minute cap.

**The operator sink.** It is one destination served as one stream per tenant. `audit_operator_config`, one row, holds a
fingerprint of the configured destination (SHA-256 of the URL and of the credential, never either itself), a configuration revision and a global
discovery cursor; `audit_operator_export` holds one row per tenant, each with the claim token and expiry, state,
`next_attempt_at`, revision and cursor of a tenant sink and the same compare-and-set, halving and refusal. A message
holds one tenant's records, so messages keep tenant scope, and order is promised per tenant; up to 8 tenants' streams
are sent at once per replica. Discovery reads the cursor query of section 4 without the tenant predicate from the
discovery cursor; for a tenant that has no row yet it creates one whose cursor stands just below the first record found,
so no record of a new tenant (a newly provisioned attacker's probes included) is skipped, and the writers of audit
records gain nothing. At start, an engine compares its configured destination's fingerprint with the stored one under
a lock of the configuration row: only a different fingerprint is a change, which raises the revision once, keeps the
cursors and lets every stream paused for its destination resume; the configured `start` (`now`, or a time) is stored
too and applies once, only when its own value changes; a restart or a rolling deploy with the same configuration changes
nothing. The discovery cursor records its cluster like every other cursor.

The pass holds no lock across an HTTP call. A crash between a receiver's `2xx` and the commit sends that request's
records again on a later pass, under a new delivery id.

**Credentials.** The URL, the collector credential and the signing keys of a tenant sink are sealed through today's
`SecretCipher::seal` and `open` (`crates/domain/src/secrets.rs`), each with its own key id column and a column holding
its nonce and ciphertext, and the associated data `"sagat/audit-sink/v1"`, then the tenant id, the sink id and the column name, each
preceded by its length in four bytes, so a sealed value cannot be moved to another sink or column. Slice 2 needs nothing
of design 015; the sealed columns of `audit_sinks` join design 015's re-seal inventory, and when design 015's `transit`
authority is built they move to it with its re-seal (Task 040-22, after Task 015-11). The operator sink's URL,
credential and signing key come from the engine's configuration; under design 015's `transit` they come from the KV path
`secret/data/sagat/audit/operator-sink` through the OpenBao Agent (Task 040-22).

**Storage, as the builders share it.**

```sql
CREATE TABLE audit_sinks (
  tenant_id uuid NOT NULL, id uuid NOT NULL, slot smallint NOT NULL CHECK (slot BETWEEN 1 AND 4),
  home_namespace_id uuid NOT NULL, kind text NOT NULL CHECK (kind = 'https'),
  url_host text NOT NULL, url_path_fingerprint text NOT NULL,          -- first 16 hex of SHA-256 of the path
  url_sealed bytea NOT NULL, credential_sealed bytea NOT NULL, signing_key_sealed bytea NOT NULL,
  previous_signing_key_sealed bytea, previous_signing_key_expires_at timestamptz,
  url_key_id text NOT NULL, credential_key_id text NOT NULL, signing_key_key_id text NOT NULL,
  previous_signing_key_key_id text,                                   -- the ring key id of each sealed column
  format text NOT NULL CHECK (format = 'ndjson'), gzip boolean NOT NULL, ceiling text NOT NULL CHECK (ceiling IN ('internal','pii')),
  state text NOT NULL CHECK (state IN ('ACTIVE','STALLED','PAUSED','PAUSED_GRANT_REVOKED','PAUSED_AUTHORITY_LOST',
                                       'PAUSED_DESTINATION_REFUSED','CURSOR_INVALID')),
  revision bigint NOT NULL, cursor_tx xid8 NOT NULL, cursor_id uuid NOT NULL,
  last_delivered_id uuid, last_delivered_at timestamptz, failures integer NOT NULL, next_attempt_at timestamptz NOT NULL,
  authorizing_subject text NOT NULL, claim_token uuid, claim_expires_at timestamptz, last_test_at timestamptz,
  PRIMARY KEY (tenant_id, id), UNIQUE (tenant_id, slot),
  FOREIGN KEY (tenant_id, home_namespace_id) REFERENCES namespaces (tenant_id, id));
-- audit_operator_config: one row with destination_fingerprint, revision, discovery_cursor_tx, discovery_cursor_id.
-- audit_operator_export: one row per tenant (tenant_id primary key) with state, revision, cursor_cluster, cursor_tx,
-- cursor_id, failures, next_attempt_at, claim_token, claim_expires_at, last_delivered_id, last_delivered_at; no
-- destination column. audit_sinks also has cursor_cluster bigint NOT NULL beside its cursor.
```

Each sealed column holds the nonce and the ciphertext of `SealedSecret`, framed as `nonce || ciphertext`. The ports:
`AuditSinkStore` (`create`, `change`, `set_state`, `list`, `get`, `refused`) for the API and `AuditExportStore`
(`look`, `claim`, `renew`, `read_batch`, `commit_cursor`, `record_refused`, `pause`) for the loop, in
`crates/application/src/audit_export.rs`, implemented in `crates/infra-postgres/src/audit_sinks.rs`; the loop is
`crates/scheduler/src/audit_export.rs`, run by `apps/execution-engine/src/loops.rs`.

**After a failover that lost work.** Every event carries the region epoch in force when it is sent
(`unmapped.sagat.region_epoch`; feature 032's epoch, 1 until feature 032 is built). After an activation whose failover
report states a window of possibly lost work (feature 032, Delivery Constraints of the constitution), each sink first
sends a tombstone message, an OCSF Base Event (class 0, activity 99) with `unmapped.sagat.tombstone: {epoch, lostFrom,
lostTo}`, then rewinds its cursor to the first record at or after `lostFrom` and sends again, under the new epoch, every
record of the window that survived; a receiver treats an event of an older epoch in that window that was not sent again
as lost.

### 6. Surfaces

- REST, under `audit:sink:manage` together with `audit:read` (a new `Permission::AuditSinkManage` in
  `crates/domain/src/identity.rs`, added to `Role::ADMIN_PERMISSIONS`): `GET/POST /audit/sinks`,
  `GET/PATCH/DELETE /audit/sinks/{sinkId}`, `POST /audit/sinks/{sinkId}/pause`, `/resume`, `/rotate-signing-key`
  (answers the new key once), `/test` (sends one synthetic event marked `metadata.labels: ["test"]` and answers a closed
  outcome, `ok`, `unreachable` (every egress refusal, name-resolution failure, transport error and timeout alike, so
  that the test cannot map internal names) or `http_<class>`, never the remote body; at most one per sink per minute;
  audited), and `GET /audit/sinks/{sinkId}/refused`. The answers that carry a signing key (`POST /audit/sinks` and
  `/rotate-signing-key`) are `Cache-Control: no-store`, and the key is never logged or put in an audit reason, as the
  one-time `leaseToken` of the worker protocol is. A sink of another tenant answers `404` exactly as a missing one,
  audited as a cross-tenant denial with a new `ResourceType::AuditSink`. Two creates that take the same free slot meet the
  unique constraint; the second retries once with the next free slot, then answers `409 AUDIT_SINK_LIMIT`. The `/test`
  event is an API Activity 99 marked `metadata.labels: ["test"]` with a random `metadata.uid`, sent in `ACTIVE`,
  `STALLED` and `PAUSED_DESTINATION_REFUSED` only, and moves no cursor. The limit of 4 sinks per tenant is held by a
  `slot smallint CHECK (slot BETWEEN 1 AND 4)` column with `UNIQUE (tenant_id, slot)`, so it needs no lock on the
  tenant's row. A sink shows `scheme://host`, the path fingerprint, the state, the id and time of the last delivered
  record and the age of the oldest undelivered one; never the cursor, the URL path, the credential or a key. Typed
  errors with `code`, `retryable`, `traceId`.
- Audit actions (seven, each with its mapping in the appendix; each record names the resource type `audit_sink` and the
  sink's id as its resource, its actor is the administrator or, for the pass, `system:engine`, and its reason is a closed
  value: the state, or for `audit.sink_record_refused` the refused record's id): `audit.sink_created`, `audit.sink_changed`,
  `audit.sink_deleted`, `audit.sink_paused` (an administrator's pause and every move of the pass to a state other than
  `ACTIVE`, with the state as a closed reason: `PAUSED`, `STALLED`, `PAUSED_GRANT_REVOKED`, `PAUSED_AUTHORITY_LOST`,
  `PAUSED_DESTINATION_REFUSED`, `CURSOR_INVALID`), `audit.sink_resumed` (an administrator's resume and the pass's
  return to `ACTIVE`, with the earlier state as reason), `audit.sink_delivered`, `audit.sink_record_refused`. The URL
  never appears in an audit reason, a log line or a span.
- Metrics: `audit_export_events_total{kind, outcome}`, `audit_export_batches_total{kind, outcome}`,
  `audit_sinks{state}`, `audit_export_oldest_undelivered_seconds{kind}` (a gauge, the largest over the replica's sinks
  of the age of the oldest record above the cursor), `audit_export_lag_seconds{kind}` (acknowledgement minus
  `created_at`), and `postgres_oldest_xid_holder_seconds` (the age of the oldest backend that holds a transaction id;
  read through a `SECURITY DEFINER` function owned by a monitoring role, with `search_path` pinned, that returns only
  that age, so the runtime role gains `EXECUTE` on it and no view of other sessions' text, as design 041's Gap 041-016
  recommends). Alert rules: a sink not `ACTIVE` for 15 minutes; the oldest undelivered record
  older than 15 minutes; the oldest transaction-id holder older than 5 minutes. Their rule tests join the CI step that
  runs `promtool test rules` (`.github/workflows/ci.yml`), which is changed to run every `infra/observability/tests/*_test.yaml`.
- Spans: `audit.export.batch` per batch with the sink kind and the count, never the URL or a value.
- Studio: a page under tenant settings to list, create, test, pause, resume and rotate sinks, and to list refused
  records.
- Runbook: `docs/runbooks/audit-export.md` (create a sink, rotate its key and credential, read the oldest undelivered
  record, what each state means, `startFrom`, the epoch step of a restore into another cluster, the operator sink's
  configuration, the grant of `pg_read_all_stats`).

### 7. Rollout

The column, the indexes and the tables are additive. The `ALTER TABLE` statements set a `lock_timeout` of 2 seconds, so
that they never queue behind a long transaction and block every audited write; the migration is retried when it times
out. A failed concurrent index build leaves the migration marked dirty: the runbook gives the recovery (drop the invalid
index, clear the mark, run again) and an operator command, run as the database owner, that builds the two indexes when
the migrator cannot run outside a transaction. A binary older than 0.3.0 inserts audit rows that get `tx_id` from the
default (its six insert statements name their columns, so the column grant does not break it), and a newer action
string read by an older binary projects as unknown (Requirement 040-001). The export loop is always present in a 0.3.0
engine. `POST /audit/sinks` answers `422 AUDIT_EXPORT_NOT_ACTIVATED` until the operator records the requirement
`audit-export-v1` in the instance registry (ADR-0082), which the registry refuses while a live engine or control plane
does not list the capability.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A lossy mapping hides what happened | The native record stays the authority; the dotted action is always in `unmapped.sagat.action`; the mapping is reviewed per action in the appendix |
| The stream misses a record that commits late | The cursor is ordered by transaction id and reads only below the snapshot's `xmin`; Success criterion 040-003 forces the order |
| A long transaction of one tenant, or outside the platform, delays every sink | It delays, never loses; the oldest-undelivered and oldest-transaction gauges and their alerts; idle-in-transaction timeouts on the platform's pools |
| Transaction ids go back after a restore | Cursors record their cluster; `CURSOR_INVALID` holds until `rebase` (section 4) |
| A sink leaks across tenants or shows the cluster's activity | Tenant predicate and withheld actions in the query; the cursor is never shown; Success criterion 040-004 |
| Personal data leaves without authority | The reason is left out below `pii`; `pii` needs the grant in every namespace, checked per batch and at commit; authority rechecked per batch |
| Someone who can read a namespace secret forges or reads the stream | Sink credentials and keys are not namespace secrets and are never delivered |
| A receiver is flooded by duplicates | A request's records are resent after a failed commit, a lost claim or a `rebase`; `metadata.uid` dedupes |
| One bad record blocks a sink | Halving isolates it; it is recorded as refused and passed |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | The audit record is written in its transaction as today; a sink's cursor advances in one transaction that compares revision, claim, cursor and state (section 5) |
| II. Modular domain, ports and adapters | Pass | The domain gains only `AuditAction::ALL` and `parse`; OCSF's codes, the projection and its serialization are in `crates/application`, with no port and no mapper; the HTTP client and the statements in adapters |
| III. External effects are at-least-once; sagas compensate | Pass | Delivery to a sink is at-least-once with a stable `metadata.uid` |
| IV. Compatibility is versioned and pinned | Pass | OCSF 1.10.0 pinned and vendored; an additive OpenAPI operation; the native read unchanged; a mixed fleet streams (section 7) |
| V. Code, tenant boundary and secrets are isolated | Pass | Tenant-scoped queries and sinks; the operator sink only from configuration, one tenant per message; sink credentials sealed and never delivered; authority and grant rechecked per batch; no tenant or sink id as a metric label |
| VI. Observability and audit are part of the feature | Pass | Every sink act audited; metrics with closed labels; spans per batch; alerts on states, backlog and held transactions |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass with conditions | The stream is read from the audit table by a commit-safe cursor instead of an outbox row per record; the audit table is the backlog and the dead-letter store of a refused record (Complexity Tracking) |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Validation of every action against the vendored schema and by the OCSF server; the forced commit-order test, the crash hook, the state races, the cross-tenant battery and the restore test |
| Delivery Constraints | Pass with conditions | PostgreSQL as authority, no new component; the latency target is measured on a stated environment; every event carries the region epoch and a sink sends a tombstone for a lost window. Condition: the tombstone is built with feature 032 (Gap 040-005, Task 040-21) |

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

Three slices, each a pull request, in this order.

- **Slice 1, the projection and the read:** `crates/domain` (`AuditAction::ALL`, `parse` and the seven new actions, so
  that 134 of 134 hold here), `crates/application` (the mapping, the projection, the vendored schema and its tests),
  `crates/api-http` (the read), and the OpenAPI document. Tasks 040-1 to 040-8.
- **Slice 2, the stream:** `crates/infra-postgres` (migrations and statements), `crates/application` (the sink service,
  `signing.rs`), `crates/api-http` (the sink resource), `crates/domain` (`Permission::AuditSinkManage`,
  `ResourceType::AuditSink`), `crates/scheduler` and `apps/execution-engine` (the export loop),
  `infra/observability`, `.github/workflows/ci.yml` (the rule-test step) and the wire contract. Tasks 040-25 and 040-20
  first (the contracts), then 040-9, 040-10, 040-11, 040-23, 040-24 and 040-12 to 040-17. It needs nothing of design
  015 (the credentials use today's `SecretCipher`).
- **Slice 3, the surfaces and the documents:** the Studio and the documents. Tasks 040-18 and 040-19. The follow-up
  tasks wait for feature 032 (Task 040-21) and for design 015's Task 015-11 (Task 040-22).

- [ ] T040-1 Decide Owner questions 040-001 to 040-007 and write the design out; record the delegate rows.
  Refs: Requirement 040-002, Requirement 040-009, Requirement 040-011
- [ ] T040-2 Vendor the JSON Schema the OCSF 1.10.0 server exports for each class used with the declared profiles under
  `crates/application/ocsf/1.10.0/` with a `SHA256SUMS` file and a test that the files match it.
  Refs: Requirement 040-004
- [ ] T040-3 Write a declarative macro that generates `AuditAction`, its doc comments, `as_str`, `ALL` and `parse` (with a
  round-trip test), add the seven new actions through it, grow `Permission::ALL` by `AuditSinkManage`, and give
  `audit.sink_delivered` a 60-second repeat window in `collapses_repeats`; add the mapping of every action (the appendix) with no wildcard arm and the unknown mapping in
  `crates/application/src/audit_ocsf.rs`.
  Refs: Requirement 040-001, Requirement 040-007
- [ ] T040-4 Add the projection with the disclosure rules (scrubbed reason; reason left out below `pii`; caps and the
  truncation marker; required objects from structured fields only; `unmapped` keys closed per action; the region epoch)
  and unit tests per class.
  Refs: Requirement 040-003, Requirement 040-005, Requirement 040-006
- [ ] T040-5 Serialize `OcsfEvent` and validate (with `jsonschema` as a dev-dependency of `sagat-application`) the events
  of every action (the fixture variants of Success criterion
  040-001) against the vendored schema, with the `at_least_one` constraints checked in code; a hand mutation that drops
  one required object fails it.
  Refs: Requirement 040-003, Success criterion 040-001, Success criterion 040-005
- [ ] T040-6 Add `infra/docker/ocsf-server/Dockerfile`, pinned to a released `ocsf-server` tag and to the `ocsf-schema`
  tag `v1.10.0` by digest, and a test target `contract_ocsf_event_standard` (declared in `tests/Cargo.toml`, run by the
  contract suites) that starts it as a Testcontainers fixture and validates one sample per class with its validation
  API; Task 040-2's files are exported from this same image.
  Refs: Success criterion 040-002
- [ ] T040-7 Add `GET /audit/ocsf` with the OpenAPI operation and schema, the same permission, filters, withheld actions
  and self-audit as `GET /audit`, the contract baseline, and a test that a page issues one query.
  Refs: Requirement 040-002, Non-functional requirement 040-002
- [ ] T040-8 Generate `docs/reference/ocsf-mapping.md` from the table, blessed as `contract_developer_experience` blesses
  the reference (`SAGAT_DOCS_BLESS=1`), and a test that fails when it is behind.
  Refs: Requirement 040-012, Success criterion 040-005
- [ ] T040-9 Write the two migrations: `audit_records.tx_id` (nullable, then the default, no rewrite, `lock_timeout`
  set) with the column grant that leaves it out; the two partial indexes built concurrently outside a transaction with
  the `indisvalid` check (verify the migrator's `-- no-transaction` support first; otherwise the owner's command of
  section 7); `audit_sinks` with `tenant_id NOT NULL` and the slot, `audit_operator_export`, grants, entries in
  `crates/infra-postgres/storage-classification.json` (the sealed columns as `secret`), and a down migration that refuses
  while a sink exists.
  Refs: Requirement 040-008, Requirement 040-009, Non-functional requirement 040-001
- [ ] T040-10 Add `Permission::AuditSinkManage` (admin) and `ResourceType::AuditSink`, and the sink resource to the REST
  API (create with `startFrom`, change, pause, resume, rotate, test, delete, refused records) with the egress check, the
  sealed credentials and their associated data, the signing key returned once with `no-store`, the `pii` and authority
  checks, the four slots, the `404` of another tenant's sink, and the audit actions.
  Refs: Requirement 040-008, Requirement 040-010, Requirement 040-013
- [ ] T040-11 The export store: `look`, `claim`, `renew`, `read_batch` (the cursor query below `xmin` with the withheld
  actions), `commit_cursor` with its compare-and-set and the `pii` recheck, `record_refused`, `pause`, and
  `holds_reveal_grant_in_every_namespace`, with tests of each guard.
  Refs: Requirement 040-009, Requirement 040-010
- [ ] T040-23 The export loop: the batch and byte bounds, the direction `AuditExport` in `signing.rs` with the covered
  fields of section 5, two signing keys during a rotation, a delivery id per attempt, a commit after each accepted
  request, the three failure classes with halving and refused records, the states of section 5, the in-memory idle
  back-off; Task 040-20 merged first.
  Refs: Requirement 040-009, Requirement 040-013
- [ ] T040-24 The operator sink's grouping by tenant with the longest acknowledged prefix, its row in
  `audit_operator_export`, and the port `AuditExportSender` with its egress policy from the allowed hosts.
  Refs: Requirement 040-011
- [ ] T040-12 Add the operator sink from the engine's configuration (or the KV path under `transit`), its allowed hosts,
  its cursor row and its evidence in logs and metrics, and document its configuration.
  Refs: Requirement 040-011
- [ ] T040-13 Fault and race tests: the forced commit order and the crash hook of Success criterion 040-003; the state
  races, the claim held past its expiry and the `401` case of Success criterion 040-006; the restore of Success
  criterion 040-007.
  Refs: Success criterion 040-003, Success criterion 040-006, Success criterion 040-007, Requirement 040-009
- [ ] T040-14 Budget: an audited transaction issues the same statements as before the migration; write-ahead log bytes
  and insert latency of 10,000 audit inserts before and after; transaction ids consumed by idle sinks; all reported.
  Refs: Non-functional requirement 040-001, Success criterion 040-007
- [ ] T040-15 Cross-tenant and privilege tests for the read, the sink resource and the pass, also as the runtime role,
  including the probe of Success criterion 040-004 and an insert that names `tx_id` refused by the column grant.
  Refs: Success criterion 040-004
- [ ] T040-16 Metrics, spans, the three alert rules with their rule tests, the CI step changed to run every rule test,
  and the capability `audit-export-v1` (kinds `EXECUTION_ENGINE` and `CONTROL_PLANE`) added to the closed checks of the
  registry's tables and to `Capability` in `crates/application/src/instances.rs`, with the `422` before activation.
  Refs: Requirement 040-009, Non-functional requirement 040-004
- [ ] T040-17 Measure Non-functional requirement 040-003 (tenant and operator sinks) on a stated environment and write
  the dated report in `specs/040-ocsf-event-standard/validation-report.md`.
  Refs: Non-functional requirement 040-003
- [ ] T040-18 Studio: the sinks page (list, create, test, pause, resume, rotate, refused records, state and backlog).
  Refs: Requirement 040-008
- [ ] T040-19 Write the runbook, the quickstart of CUJ 1 with the failure scenario of CUJ 2, and update
  `docs/traceability.md`, the roadmap row and the release notes.
  Refs: Requirement 040-012
- [ ] T040-20 Write the wire contract `specs/040-ocsf-event-standard/contracts/audit-export.asyncapi.yaml` (every header,
  the signed string, the body, the status codes and what each means, deduplication by `metadata.uid`) with known
  signature vectors, its baseline and a compatibility test that the slice-2 code reads.
  Refs: Requirement 040-009
- [ ] T040-25 The sink contract in OpenAPI, before Task 040-10: the `/audit/sinks*` operations and schemas, the fields
  `PATCH` may change (URL, credential, home namespace, ceiling, format, gzip), the `startFrom` body, the answers (host,
  path fingerprint, state, last delivered record, oldest undelivered age), the refused-records page, and the error codes
  `AUDIT_SINK_LIMIT` (`409`), `EGRESS_NOT_ALLOWED` (`422`), `PII_CEILING_NOT_PERMITTED` (`403`),
  `START_FROM_TOO_OLD` (`422`), `START_FROM_TIMEOUT` (`503`, retryable), `SINK_TEST_RATE_LIMITED` (`429`) and
  `AUDIT_EXPORT_NOT_ACTIVATED` (`422`), with the contract baseline.
  Refs: Requirement 040-008

- [ ] T040-26 Extend `control-plane reseal-secrets` to the sealed columns of `audit_sinks` (each with its key id), with a
  test that rotates the key ring, re-seals, removes the old key and still delivers.
  Refs: Requirement 040-008

#### Follow-up tasks

- [ ] T040-21 The tombstone after a failover that lost work, from feature 032's region epoch and failover report, with
  the OCSF Base Event (class 0) schema vendored, and a test on feature 032's failover drill. Depends on feature 032.
  Refs: Requirement 040-009
- [ ] T040-22 Move the sink's sealed columns and the operator sink's credentials to design 015's `transit` authority with
  its re-seal, after Task 015-11.
  Refs: Requirement 040-008, Requirement 040-011


### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the three
checkers, the unit tests of the crates it changes and the suites it touches, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`
where it adds a statement. A test is seen failing for the right reason before the code that passes it; races use
barriers and fault hooks, never sleeps.

- Slice 1: the domain and projection unit tests; the validation of every action against the vendored schema (Success
  criterion 040-001); the OCSF server samples; the OpenAPI baseline; the reference drift test; the hand mutation of Task
  040-5.
- Slice 2: a new target `integration_ocsf_export`, declared in `tests/Cargo.toml`, on real PostgreSQL with a local HTTPS receiver; the tests of Task
  040-13; the budget of Task 040-14; the cross-tenant battery as both roles; the migrations' up, down and privilege
  audit; the hand mutation `pg_snapshot_xmin` into `pg_snapshot_xmax`.
- Slice 3: the Studio's `npm run api && npm run check:api` and its unit tests; the quickstart run end to end including
  the outage of CUJ 2.

## Appendix: the mapping of every audit action (2026-11-26)

The column "Class, activity" uses short labels, each standing for one OCSF class: "API" for API Activity 6003
(activities 1 Create, 2 Read, 3 Update, 4 Delete, 99 Other; 5 List is not used); "UM" for User Management 3007 (14 Assign
Privileges, 15 Remove Privileges, 16 Assign Roles); "EM" for Entity Management 3004 (1 Create, 3 Update, 4 Delete, 6
Enroll, 7 Unenroll, 8 Enable, 9 Disable, 99 Other); "DF" for Detection Finding 2004 (1 Create, with the attributes of Requirement 040-007). Severity is Informational (1) unless the row says Low (2) or Medium (3). Status is Success (1) unless the row
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
| `database.result_read` | API 2 | |
| `database.catalog_browsed` | API 2 | |
| `iceberg.write_committed` | API 3 | |
| `destination.decided` | API 99 | |
| `database.denied` | API 99 | Low, Failure |
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
