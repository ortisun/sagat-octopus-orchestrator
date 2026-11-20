# OCSF as the standard schema for security and audit events

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-13

--------------------------------------------------------------------------------

## Overview

The platform adopts the Open Cybersecurity Schema Framework (OCSF, <https://schema.ocsf.io/>) as the standard shape of
the security-relevant events it hands to the outside: the audit trail first, then the security incidents and the
operational alerts. An operator can then send them to a SIEM or a security data lake that reads OCSF without writing a
mapping per tool. The owner asked for it on 2026-11-13: "Adicione mais uma especificacao de ter o
https://schema.ocsf.io/ como padrao". This draft states the scope the assistant read into that sentence and the
decisions it leaves to the owner; it is to be written out with `egm-draft` and reviewed before any code.

## The Problem

The audit trail has a shape of its own. A record (`crates/domain/src/audit.rs`, `AuditRecord`) holds a tenant, an
actor, one of about 156 actions (`AuditAction`, for example `membership.granted` or `business_id.conflict`), a resource
type and id, an optional closed reason, a trace id and a time. `GET /audit` returns the records of a tenant to an
administrator, newest first, with filters and a cursor (`docs/runbooks/observability.md`). Sandbox denials are durable
incidents of their own, and the monitor raises alerts with its own kinds.

A security team that wants these in its SIEM has to learn that shape, write a parser, and keep it current as actions
are added. Every product they run asks the same of them, which is the problem OCSF exists to remove: one vocabulary of
event classes (Authentication 3002, API Activity 6003, Entity Management 3004 and so on, in OCSF 1.9.0), one base
event, one set of objects for the actor, the resource and the outcome.

## Requirements

- **FR-040-001** Every audit action has exactly one documented OCSF mapping: a class (`class_uid`), an activity
  (`activity_id`), a severity and a status, kept in one table in the code. An action without a mapping does not compile
  or fails a test, so a new action cannot ship unmapped.
- **FR-040-002** An administrator can read the audit trail of their tenant as OCSF events, with the filters and paging
  `GET /audit` has today. The native shape stays available and unchanged.
- **FR-040-003** Each event carries the base attributes OCSF requires (class, category, activity, type, time,
  severity, metadata with the product and the schema version) and the platform's facts in OCSF objects: the actor as
  `actor.user`, the tenant, the affected resource, the outcome and the reason, and the trace id as the correlation
  identifier.
- **FR-040-004** The OCSF schema version is pinned and stated in every event (`metadata.version`). Moving to a newer
  version is a recorded decision, never a side effect of a dependency update.
- **FR-040-005** An OCSF event discloses nothing the native record does not: no business id, no payload, no secret, no
  field of another tenant. The classification rules of feature 016 apply to it as they do to the native read.
- **FR-040-006** What OCSF has no class for (a workflow published, a run reset, a schedule paused) maps to the nearest
  class with the platform's own detail in a declared extension or in `unmapped`, as OCSF prescribes. The choice per
  action is in the mapping table.
- **FR-040-007** Sandbox-denial incidents and monitor alerts are available as OCSF findings (Detection Finding 2004),
  in a later slice than the audit trail.
- **SC-040-001** Every event the platform emits validates against the pinned OCSF schema in an automated test, for
  every audit action.
- **SC-040-002** A reference SIEM or validator that reads OCSF accepts a sample of each class with no custom parser.
- **SC-040-003** The mapping is published as reference documentation generated from the table, and a test fails when
  the page is behind it.

## Goals

- One standard, documented shape for the security events of the platform, so that integrating a SIEM is configuration.
- A mapping that cannot drift: the table is the single source, checked by tests and rendered into the reference.

## Non-Goals

- Replacing the stored audit record. PostgreSQL keeps the native record as the authority; OCSF is a projection of it.
- Replacing OpenTelemetry traces, metrics and logs, which stay as they are. OCSF covers security events, not telemetry.
- Shipping a connector per SIEM vendor.
- Ingesting OCSF events from other systems.

## Open Questions

- **O040-001 Scope.** The audit trail only, or also incidents and alerts (FR-040-007), and in which order.
- **O040-002 Delivery.** Read on request only (`GET /audit` with an OCSF representation), or also pushed: a stream to
  a sink the tenant or the operator configures (a webhook, a broker topic, object storage in JSON or Parquet). A push
  goes through the outbox and is at-least-once, so events carry a stable `metadata.uid` for deduplication.
- **O040-003 Who configures a sink.** The operator for the whole platform, a tenant administrator for their tenant, or
  both; and whether a platform-wide stream may leave the tenant boundary at all.
- **O040-004 Schema version.** Pin OCSF 1.9.0 (the stable version on 2026-11-13) and which profiles to declare
  (for example `cloud`, `datetime`).
- **O040-005 Extension.** Register a platform extension for workflow facts (run, workflow, schedule, namespace), or
  keep them in `unmapped` until a need is shown.
- **O040-006 The native shape.** Whether OCSF becomes the default representation of `GET /audit` in a later major
  version, with the native one kept behind a parameter, or stays an alternative representation for good.
- **O040-007 Logs.** Whether security-relevant log lines (authentication failures, denied requests) also take the OCSF
  shape, or only durable records do.

## Owner decisions needed

- Confirm the reading of the request: OCSF is the standard for the security and audit events the platform hands out,
  as a projection of the native record (Overview, Non-Goals).
- Answer O040-001 to O040-007.
- Accept the ADR this feature needs once drafted (the mapping table as the single source, the pinned schema version,
  the projection instead of native storage).
- Accept this design for implementation once it is written out and reviewed.

## Critical User Journeys (CUJs)

### CUJ 1: A security team connects its SIEM

*   **Given**: a tenant whose administrator has a SIEM that reads OCSF.
*   **When**: they read the audit trail in the OCSF representation, or configure the stream if O040-002 adds one.
*   **Then**: the SIEM shows who did what to which resource, with class, activity, outcome and time, without a custom
    parser, and a role granted yesterday appears as an Identity and Access Management event.

### CUJ 2: A developer adds an audit action

*   **Given**: a change that adds an `AuditAction`.
*   **When**: it is built and tested.
*   **Then**: the build or a test fails until the action has an OCSF mapping, and the reference page shows it.

## Technical Plan (Architecture)

To be written with `egm-draft`. The outline:

1. A mapping table in the domain (`AuditAction` to class, activity, severity, status), total by construction, with no
   serialization in the domain (principle II): the OCSF JSON is built by an adapter.
2. An OCSF projection in the application layer and its JSON codec in an adapter crate, with the pinned schema files
   vendored for the validation test.
3. An OCSF representation of `GET /audit` (content negotiation or a parameter, decided with the contract), in the
   OpenAPI contract, with the same authorization, filters, paging and classification as the native read.
4. If O040-002 adds a push: a sink resource, delivery through the outbox with retries and a dead-letter state, egress
   under the existing allowlist and secret handling, metrics and an alert for a stalled stream.
5. Incidents and alerts as findings (FR-040-007).
6. Generated reference documentation and the contract tests.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A lossy mapping hides what happened | The native record stays the authority; platform detail goes in the extension or `unmapped`; the mapping is reviewed per action |
| A stream leaks across tenants | Events are built from a tenant's own records; a platform-wide stream is an owner decision (O040-003) with its own authorization |
| OCSF changes under us | The version is pinned and stated in every event; an upgrade is a recorded decision with the validation suite rerun |
| A push duplicates events | At-least-once through the outbox, with a stable `metadata.uid` per event |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | n/a | Not yet designed |
| II. Modular domain, ports and adapters | n/a | Not yet designed |
| III. External effects are at-least-once; sagas compensate | n/a | Not yet designed |
| IV. Compatibility is versioned and pinned | n/a | Not yet designed |
| V. Code, tenant boundary and secrets are isolated | n/a | Not yet designed |
| VI. Observability and audit are part of the feature | n/a | Not yet designed |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | Not yet designed |
| VIII. Failure is tested and contracts are verified, honestly | n/a | Not yet designed |
| Delivery Constraints | n/a | Not yet designed |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|

## Alternatives Considered

- Storing audit records natively in OCSF: rejected for now. It ties the authority to an external schema's versions and
  would rewrite every writer of the trail; a projection gives the same events to the reader.
- A vendor format (CEF, LEEF, ECS): OCSF is the one the owner named, is vendor-neutral and is what the security data
  lakes read.
- Documentation of the native shape only: leaves every customer to write and maintain a parser.

## Non-Functional Requirements

- Reading the OCSF representation costs no more than a small constant factor over the native read (no extra query per
  record).
- A stream, if built, does not slow the transaction that writes the audit record: it reads the outbox.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T040-1 Write this design out with `egm-draft` after the owner answers O040-001 to O040-007, with the mapping of
  every existing audit action as an appendix, and run `egm-review`.
  Refs: FR-040-001

#### Follow-up tasks

None yet.

### Verification Plan

To be written with the design. It will include validation of every emitted event against the pinned OCSF schema
(SC-040-001) and a tenant-boundary test of every read and stream.
