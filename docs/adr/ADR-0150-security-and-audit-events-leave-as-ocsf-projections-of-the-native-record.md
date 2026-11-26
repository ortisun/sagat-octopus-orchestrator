# ADR-0150: Security and audit events leave the platform as OCSF 1.10.0 projections of the native record, mapped by one total table and streamed by a cursor below the snapshot's xmin

**Status:** Accepted (2026-11-26, delegate, Owner decision 2026-11-24-4)
**Decided by:** agent proposal with design 040, accepted by the agent preparing design 040 for build under the owner's
delegation "Quero que os agents continuem por mim. Tomem a melhor decisão" (Owner decision 2026-11-24-4); recorded as
Owner decision 2026-11-26-107. The owner may overrule it.
**Refines / supersedes:** applies Principle VI (the audit record stays the authority), ADR-0020 (the export loop is
placed by shard) and ADR-0082 (activation), and the `pii`-egress rule of Owner question 1081 (feature 020, Decision
1617) to a new destination; supersedes none.
**Enforced by:** the clippy lint `wildcard_enum_match_arm` denied in `crates/application/src/audit_ocsf.rs` (Task 040-3);
the validation of every action against the vendored schema (Task 040-5); the forced commit-order, crash, state-race and
restore tests (Task 040-13); the budget test (Task 040-14); the cross-tenant battery and the column-grant test (Task
040-15); the wire-contract test (Task 040-20). Until they exist: review only.

## Context

Security teams read OCSF; the platform's audit trail has a shape of its own (design 040, The Problem). The trail is
written from more than 300 places through six statements in four store modules, and `audit_records` has no sequence: `created_at` is taken
before a commit that can land after a later one's.

## Decision

1. The audit record in PostgreSQL MUST stay the authority. OCSF is a projection built on read or on export; nothing is
   stored in OCSF.
2. Every `AuditAction` MUST map to exactly one OCSF class, activity, severity and status through one `match` without a
   wildcard arm in `crates/application`; OCSF's codes stay out of the domain. A stored action string that no variant matches
   MUST project as an unknown API Activity, never fail.
3. The OCSF version MUST be pinned (1.10.0) and vendored with its digest; a move to another version is a new
   architecture decision.
4. A stream MUST read the audit table by a cursor `(tx_id, id)`, where `tx_id` is the writing transaction's `xid8`
   filled by a column default that the runtime role cannot set, and only below `pg_snapshot_xmin(pg_current_snapshot())`.
   The writers MUST NOT gain a statement. Delivery is at least once with `metadata.uid` the record id; a failing sink
   stalls, it never skips, except a single record its receiver refuses permanently, which is recorded as refused and
   stays in the audit table. The cursor is never shown outside the platform. A restore into another cluster MUST advance its
   transaction-id epoch above the largest stored `tx_id`; until then the export refuses (`CURSOR_INVALID`).
5. A tenant sink MUST carry only its tenant's records less the withheld actions, under the current authority of its
   authorizing subject, rechecked per batch; its URL and credentials are sealed in the sink and never delivered to
   anything else. Only the deployment's configuration may define the operator sink, which carries every tenant's records
   one tenant per message and persists only its cursor.
6. What leaves through a sink follows the uniform `pii`-egress rule: `internal` by default; `pii` only under the reveal
   grant held in every namespace of the tenant, checked per batch and at the cursor's commit; `secret` never. The
   operator sink is `internal`.

## Considered options

1. **A projection streamed below `xmin`** — no write added, no commit-order gap, no slot on the primary.
2. **An outbox row per audit record** — the same guarantee with a second write in every audited transaction.
3. **Logical decoding of `audit_records`** — ordered by commit, but an unconsumed slot holds write-ahead log without
   bound.
4. **Native OCSF storage** — ties the authority to an external schema's versions.

## Consequences

A long transaction delays every sink until it ends (the oldest-undelivered and oldest-transaction gauges show it). Records written before the migration are not
streamed (they stay readable). Moving to OCSF 1.11 or later is a decision, not an update. A new audit action cannot be
merged without its mapping.

## Evidence

Design 040 (`docs/designs/040-ocsf-event-standard.md`), its appendix, and `specs/040-ocsf-event-standard/research.md`.
The version was read from `https://schema.ocsf.io/api/version` on 2026-11-26 (1.10.0), and the classes and activities
from the same server's `/api/classes` pages.
