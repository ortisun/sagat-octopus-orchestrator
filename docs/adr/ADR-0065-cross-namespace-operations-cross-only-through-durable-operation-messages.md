# ADR-0065: Cross-namespace service operations cross the boundary only through durable operation messages, under a pinned contract, an endpoint allow-list and a declared result disclosure

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26)
**Refines / supersedes:** refines ADR-0002, ADR-0012, ADR-0020, ADR-0030 (§18–19), ADR-0031 and
ADR-0033 for calls between namespaces; follows the admission order of ADR-0050 (proposed) and the
registries of features 018, 020, 028 and 031 (drafts); supersedes none.
**Enforced by:** planned tests of feature 030 — the commit-point crash matrix and duplicate/lost-hint
suites in `tests/fault/service-operations/`, the isolation battery in
`tests/security/service-operations/`, the disclosure canaries, and a lock-order review of every
transaction in `specs/030-cross-namespace-services/plan.md`. None exists yet (feature 030, T030-*).

## Context

A namespace is the environment and RBAC scope of definitions and runs. A `child` node resolves its
workflow only in the parent's namespace (`crates/domain/src/workflow.rs`, `NodeKind::Child`; D36 of
feature 002), so there is no typed, authorized way for `orders-prod` to use a capability owned by
`payments-prod`. Feature 030 adds service contracts, an endpoint registry and a caller node in the
manner of Temporal Nexus (<https://docs.temporal.io/nexus>), inside one tenant.

The forces:

- Constitution I and ADR-0002: a transition, its next tasks, timers and outbox commit together. Two
  namespaces have independent runs, quotas and operators; one transaction that completes the caller
  step from the handler's terminal transaction would hold two run mutation locks, which ADR-0030
  §18–19 and D36 avoid for parents and children.
- Constitution VII and ADR-0012/0025: every message can arrive twice, late or out of order; the inbox
  deduplicates before acting, NATS carries hints.
- Constitution V: the tenant is a mandatory dimension and another tenant's resource is
  indistinguishable from a missing one; authorization fails closed. Memberships are tenant-wide
  (`memberships` primary key `(tenant_id, subject)`), so roles alone cannot separate `orders-dev` from
  `payments-prod`.
- ADR-0028/0031/0033: values are disclosed only by an explicit declaration and per-field
  classification; a schema alone is not consent.
- ADR-0032: IR changes are additive capabilities in v1.

## Decision

1. **One tenant.** An endpoint, its allow-list entries, both namespaces, the caller pins, the operation
   records and the messages MUST carry one tenant id with composite foreign keys, so a cross-tenant
   call is not representable. A foreign reference MUST be reported as `NOT_FOUND` and the attempt
   audited for the caller's tenant.
2. **Two owners, shared messages.** Caller-side records MUST change only in transactions that hold the
   caller run's lock (chain before run where ADR-0030 applies) and the caller run's shard fence.
   Handler-side records MUST change only in the admission transaction that creates the handler run or
   in transactions that hold the handler run's lock and shard fence. No transaction MAY hold mutation
   locks on runs of both sides. The only shared rows are operation messages: a sender inserts one in
   its causing transaction; the receiver consumes it in its own transaction under a unique
   `(tenant, operation id, kind)` key and a state check, which is the inbox deduplication.
3. **Lock order (ADR-0050 decision 7).** Handler admission MUST take its locks in decision 7's slots:
   shard fence and `START` row (3); the handler version's namespace fences and publisher memberships
   (4a); the handler version's pins, then the service-endpoint head and the allow-list entries, last in
   4b (the binding is resolved from an unlocked read and rechecked under these share locks); secrets
   (5); the new run, the execution record and the `STARTED` row (6); rate, quota and usage (8b). Caller
   publication takes memberships (4a) before the endpoint head and allow-list (4b). A transition on an
   existing run starts at slot 3R (chain before run), then takes its operation record and message
   (and, for the handler's terminal transaction, the endpoint head and allow-list share locks), and
   ends with usage. One exception, for decision 7 to list: the caller's deadline or cancellation locks
   its own `START` row (slot 3) after its run; this cannot cycle because admission, the only order-(a)
   transaction on a `START` row, never locks an existing run. Endpoint changes lock only the endpoint
   head, its allow-list rows and the registry quota fence, never a run or a message.
4. **Identity.** The operation id MUST be derived from (tenant, caller run, caller node instance, call
   attempt) with a domain-separated, length-delimited encoding, and the handler run id from the operation id; the
   database MUST reject a second handler run or execution record for one operation.
5. **Pinned contract, resolved routing.** Contract revisions are immutable. A caller version MUST pin
   the endpoint identity and the contract revision and digest at publication; routing (handler
   workflow and version under the binding policy) MUST be resolved at admission. An endpoint serves at
   most four revisions; it never retargets a pinned caller to another endpoint identity.
6. **Authorization.** The endpoint's explicit allow-list of caller namespaces is the authority for
   crossing. It MUST be checked at caller publication (advisory, under a share lock), at admission
   (authoritative) and at completion (disclosure); an empty list admits nobody and there is no wildcard.
   The caller publisher's `service:call` is checked at publication and again when the call is scheduled.
7. **Declared disclosure.** A result MUST cross only for an operation that declares
   `disclosure: "declared-service-result-v1"`, only as a value that validates against the closed,
   bounded output schema, which is a classification carrier (ADR-0033) without `secret` levels and
   canonically equal to the handler workflow's output schema. A failure code MUST cross verbatim only
   if the operation declares it. The input MUST cross only under the caller node's
   `inputDisclosure: "declared-service-input-v1"`; for a direct source path an operation input field
   MUST NOT be looser than its caller source. Ordinary projections of both namespaces stay
   payload-free.
8. **Races and order.** First commit wins on the caller run between a notification and a deadline or
   cancellation; first commit wins on the `START` row between admission and withdrawal. The loser is a
   recorded, audited no-op. No order between notifications of one operation MAY be assumed: a
   `COMPLETED` MUST be applicable before its `STARTED`, and a later `STARTED` is a no-op.
9. **Effects.** At most one handler run per operation is a platform guarantee. Effects inside the
   handler run remain at-least-once (ADR-0004); no exactly-once effect is promised across namespaces.
   Every settlement MUST record an effect class (`none`, `failed`, `applied`, `partial`, `unknown`); a
   caller deadline after admission MUST look the outcome up for a bounded window before settling as
   `unknown`; a failure whose effect is possible MUST go to saga compensation or manual review, never
   to an ordinary error route (constitution III).
10. **Trace.** The handler run MUST continue the caller's trace under a computed service-call span id,
    and both spans MUST carry OTel links to each other; durable journal links are the record.
11. **Encoded payloads (feature 031).** Encoded values MUST NOT cross namespaces in v1 (031 D1667): a v1
    envelope binds tenant, namespace and location (031 D1556) and no platform component may decode or
    re-encode. The operation schemas refuse `x-sagat-encoding`, no handler version declaring encoding
    can be bound, caller publication refuses an input from an encoded source, and admission and result
    return refuse or withhold an envelope found at runtime, with 031's codes.

## Considered options

1. **Two-sided records and durable messages (chosen).** Each side keeps its own transactional
   invariants, quotas and shard ownership; duplicates and crashes are handled by the same inbox and
   fence patterns already proven for events and children; the boundary is visible and auditable.
   Costs: two extra short transactions per call and message rows to retain and purge.
2. **Complete the caller from the handler's terminal transaction.** One fewer hop, but it locks two
   runs across namespaces, couples the handler's terminal commit to the caller's quotas and shard
   owner, and reopens the two-aggregate protocol ADR-0030 rejects.
3. **Cross-namespace `child` (a `namespace` field on the child node).** Reuses the child protocol, but
   binds callers to another team's workflow names and versions, offers no contract, allow-list or
   declared disclosure, and puts the handler run in the caller's tree and budget.
4. **HTTP callbacks between namespaces over the outbound delivery pipeline (ADR-0025).** Matches
   Nexus' transport between cells, but adds signing, egress and a network hop inside one database for
   no gain; kept as a possible transport if namespaces ever live in different cells or regions (G1484).
5. **Pin the endpoint revision at run admission (017 style).** Rejected for the contract: the caller's
   downstream expressions are typed by the contract at publication. Routing still follows the
   endpoint at admission, as connection credentials follow current authority in feature 017.

## Consequences

- Good: the caller and handler teams evolve independently behind a pinned contract; every crossing
  is authorized, classified, audited and explainable from both journals.
- Bad: latency of two dispatcher hops per notification; message rows and an extra retained record per
  call; schema compatibility is canonical equality in the first increment (no subtyping, G1485).
- Harder: handler chains (`continueAsNew`), calls inside structured bodies and attachment of several
  callers to one handler run need their own decisions (G1480–G1482).
- Resets (feature 018): each new journal event declares its rule in 018's registry; an open call
  refuses a reset point and a finished call is copied as provenance without re-creating rows.
- Operators get manual-review items for effect-possible failures outside sagas (timeouts that outlast
  the lookup window, invalid results, revocations): the price of not treating them as "not done".
- Rollback: disabling the capability stops new publications and schedules; message application for
  calls in flight continues until they drain; no stored record is erased. The additive migrations'
  down steps refuse while rows exist.

## Evidence

None yet. Required before acceptance as implemented: the crash matrix over the eight runtime commit
points, duplicate and lost-hint suites, 100-round race suites, the isolation battery, disclosure
canaries, five-SDK conformance and a benchmark report on a declared environment
([spec](../../specs/030-cross-namespace-services/spec.md),
[plan](../../specs/030-cross-namespace-services/plan.md),
[research](../../specs/030-cross-namespace-services/research.md)).
