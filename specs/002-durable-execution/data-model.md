# Data Model: Durable execution engine and scheduler

## Context

Ensure DAG progress, checkpoints, suspension, retries and failure recovery without re-executing commits.

Data ownership follows the bounded contexts of `docs/architecture/overview.md`, with the reference physical schema in `specs/001-platform-foundation/data-model.md` and illustrative SQL in `contracts/schema.sql`.

## Entities and observable state

- **Definition/Version/Artifact:** immutable after publication; tenant and namespace required.
- **Run/Step/Attempt:** stable identifiers, version, status, timestamps, deadlines, parent/correlation.
- **JournalEvent:** append-only seq per Run, event_type, payload_ref, causation/correlation and actor.
- **Inbox/Outbox:** uniqueness for dedupe and delivery; next_attempt_at; last_error without PII.
- **Lease/Subscription/Timer:** fencing generation, validity, deadline and transition status.
- **SagaEntry/Compensation:** state, dependency order, idempotency key, reason and outcome.

## Invariants

- Every tenant-scoped record validates the security context.
- No in-place update of a published version.
- Every operational state change records a sequenced event and information for reconstruction.
- Large payloads go to the blob store with digest and non-sensitive references.

## Migrations

Additive migrations first; expand/contract for existing schemas; `CONCURRENTLY` indexes where applicable on large databases; idempotent and measured backfill. See the feature plan and ADRs.
