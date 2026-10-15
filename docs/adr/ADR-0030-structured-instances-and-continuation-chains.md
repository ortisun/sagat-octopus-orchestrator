# ADR-0030 — Structured instances share the reducer and continuation creates an explicit chain

Status: **Accepted architecture; implementation and release evidence pending** | Date: 2026-10-14 | Feature: 014 | Refines ADR-0003, ADR-0005, ADR-0010 and ADR-0015; supersedes none

**Partial supersession:** [ADR-0032](ADR-0032-additive-workflow-ir.md) replaces the separate v2 IR
allocation and all version-based rollout/dispatch wording below with additive v1 capabilities.
The historical text is retained; identity, limits, compensation, transaction and chain decisions
remain accepted. Worker protocol minor 1.5 remains independent of the workflow IR.

## Context

Feature 014 needs bounded collections, sequential loops, authorized human approvals, multiway
routing, declared state operations and same-version continuation. Its current examples already run
on v1. The runtime additions require an approved dependency on the existing execution, saga,
event, sandbox identity, SDK, security, observability, scheduling and versioning features.

The current domain keys steps and sagas by `NodeId` (`execution/state.rs`), application data keys
outputs by `NodeId`, and PostgreSQL keys steps by `(tenant_id,run_id,node)`. Reusing an ordinary
node for successive iterations would merge leases, results, effects and compensation obligations.
The worker task reference is currently `run/node/generation`. None of those identities can be
silently reinterpreted for a running v1 workflow.

The repository also has operational constraints a new primitive must preserve: cancellation-safe
transaction start (ADR-0014), run-shard scheduler fencing (ADR-0020), exact worker-session admission
and finite task-stream credit (ADR-0029), durable byte/retention quotas (ADR-0027), and the
structure-only payload policy (ADR-0028). ADR-0031 separately decides value disclosure for declared
state operations; accepting this ADR does not authorize a general payload viewer.

## Decision

### Version and identity

1. Allocate IR `sagat-octopus-orchestrator.io/v2` and worker protocol **1.5**. The checked baseline
   is v1 and worker 1.4 (`application/worker.rs::PROTOCOL_MINOR`). Keep v1 validation, event meaning,
   root identities and every accepted worker minor form. Unsupported or disabled v2 capabilities
   fail before publication/run creation with the existing error envelope, without new `Error.code`
   or run-status enum values. Rollout enables each primitive only after its own evidence passes.
2. Keep the top-level graph, each body and the body-reference graph acyclic. A named body is a
   closed graph with an entry and logical return; it cannot contain workflow `end` or
   `continueAsNew`, escape into another body or recurse. All authored nodes, including bodies,
   share the 10,000-node definition ceiling.
3. Keep the existing `WorkflowDefinition`/`Node`/`NodeKind` definition model and `StepState` inside
   `RunState`; “workflow primitives” names capabilities, not a replacement Rust `Primitive` type.
   `execution/instance.rs` owns `ExecutionInstanceId {path,node}`, a run-local key because RunState
   already owns the run id. Its complete persisted identity is
   `(tenant,namespace,run,path,definitionNode)`, supplied by application/store context. The typed path
   contains `(scopeNode,iterationIndex)` pairs, indices 0..9999, depth at most 8; root is empty.
   Attempt/generation is separate. All reducer maps, events, effects, child links, timers,
   subscriptions, capabilities, task reports and projections use that identity. Definition node
   names remain local definition names, not an encoding of the path.
4. Existing root step UUIDs, child IDs and effect keys remain byte-for-byte stable. Scoped IDs use
   a domain-separated, length-delimited encoding of the typed identity. Compensation and
   reconciliation retain the forward instance path and derived-node origin (ADR-0010). Worker 1.5
   adds explicit scoped identity and checks it against the durable step identity. Scoped tasks
   never reach an older worker, through polling or streaming. Secret/blob capabilities and report
   deduplication receive the same identity; changing only task delivery is insufficient.

### Primitive semantics and bounded evaluation

5. `forEach` captures a bounded array once, retains duplicate values at distinct indices, admits
   iterations incrementally under scope and tenant capacity, and returns results in input order.
   SDK `map` emits the same node. Permanent failure stops admission and drains accepted work;
   it does not collect all failures or claim that cancellation undid an effect.
6. `while` is pre-test and `repeatUntil` is post-test. Both are sequential, persist carry/result/
   condition checkpoints and require finite iteration/deadline limits. A delay is a durable timer
   only between needed iterations; exhaustion is checked before creating it. Slots are released
   only when the iteration is terminal or safely drained, including unknown-effect reconciliation.
7. Compensation is reverse topological within an iteration; collection iterations are independent
   branches, and sequential loop iterations compensate in reverse iteration order. Completion
   sequence is evidence, never the compensation order. Forward and derived work use the same
   reducer, lease/fence, retry and manual-review machinery. A failed compensation or irreversible
   effect preserves MANUAL_REVIEW/PARTIAL rather than being overwritten by a simple scope failure.
8. One eligible decision settles an approval. Snapshot its subject/role policy and initiating
   identity; check current authorization and eligibility on every decision/replay. Initiator
   exclusion defaults true. Subjects and built-in `admin`/`developer`/`operator` roles are the only
   eligibility vocabulary; no custom roles/groups, quorum, delegation or escalation are introduced.
   `reasonCode` is bounded ASCII, never free text. Decision/expiry/cancellation use first commit,
   including a response after the nominal deadline when expiry has not committed. Signals cannot
   decide an approval. Grant `approval:read` to admin/developer/operator/viewer and `approval:decide`
   to admin/developer/operator; worker/runner receive neither.
9. Switch evaluates ordered boolean cases until the first true result, otherwise its mandatory
   default. Predicate error fails without fallthrough; replay uses the recorded choice.
10. Query/Update interpret the existing bounded expression subset, with no callback, I/O or
    arbitrary SDK code. Only Updates change declared state; no state assignment node or
    state-predicate wait is added. A successful Update advances state revision once and schedules
    no work. ADR-0031 governs the result disclosure and grants `run:query`/`run:update`.
11. The effective expression source ceiling is **1024 bytes** (`infra-expr/src/limits.rs`), although
    the existing domain string wrapper accepts 2048. Preserve the evaluator's 256-token and
    depth-16 caps, 64-element list literals, 4096-element constructed-list/concatenation cap and
    64 KiB generated-string cap. There is no object/map literal or mutation operator. State/carry/
    result expressions may select an existing object or compute values supported by this subset;
    examples must not imply an unimplemented object-construction syntax. A referenced input/blob
    array may contain up to 10,000 elements, but bounded materialization, digest/schema checks and
    byte-budget evidence are required before advertising that collection ceiling. The engine's
    indexed aggregate is not a CEL list concatenation. No evaluator cap is silently raised.

### Budgets and transactions

12. V2 has optional top-level `limits: {maxInstances,maxDepth}`; omitted fields default to 100000
    and 8 respectively. Allowed ranges are 1..100000 and 1..8. Tenant policy may lower them.
    Capture effective limits on run creation and honor a later reduction for new admissions.
    Required node-specific limits remain explicit: items/iterations 1..10000, concurrency 1..256,
    timeout/delay 1..31622400000 ms, cases 1..32, operations 0..32 per kind, approval principals
    1..32, segments 1..10000 including the initial run. These are design ceilings, not measured
    capacity claims.
13. `maxInstances` is a shared lifetime admission budget, not a count of currently active work.
    Activating a forward instance charges one unit and reserves one unit for each declared derived
    compensation/reconciliation identity it may require. Reserved units count against the ceiling
    immediately and are not reused later; activating that derived instance consumes its reservation
    without a second charge. Retries reuse identity and reservation. This conservative admission
    keeps actual activated instances within the ceiling and leaves accepted obligations drainable
    after exhaustion or policy reduction. A scope/control/return activation also costs one unit.
    Mandatory drain/terminal metadata must remain writable when forward payload admission stops;
    this does not exempt payload values from the byte limits.
14. Preserve 256 KiB inline values, 4 MiB retained/run and 8 MiB task context, and charge snapshots,
    state, carry, ordered results, receipts, transfers and compensation inputs. Blob references
    require catalog provenance/authorization and do not bypass quotas. Scope slots and tenant
    execution capacity are distinct checks; the scheduler must not deadlock nested scopes by
    consuming every descendant's admission capacity in suspended ancestors. The chosen descendant
    admission/reservation mechanism must pass nested low-capacity and policy-reduction tests before
    nesting is enabled; no progress guarantee is inferred from a quota alone.
15. Each transition atomically persists its journal, state, next tasks, timer/subscription changes,
    applicable quota deltas, receipts, audit and outbox. Admission is bounded, never whole-input
    unbounded fanout. Use `BeginSafely::begin_safely`; no response acknowledges before commit.
    Scheduler actions check owner/generation/expiry in that transaction; worker reports check their
    attempt/lease fence; authenticated commands use the run/chain serialization and current policy.
    An HTTP command does not invent a scheduler lease. Evaluate only bounded trusted expressions;
    external blob reads happen outside the transaction with immutable digest/schema verification
    and version recheck. If computation uses an unlocked snapshot, CAS the observed run/state
    revision and retry computation only against a fresh snapshot. Replay never reevaluates a
    committed choice.

### Chains and lock order

16. Continue-As-New is top-level, outside active saga bodies, and requires quiescence: no live
    sibling work, child, lease, timer, subscription, approval, unknown effect or compensation
    obligation. It atomically closes SUCCEEDED/CONTINUED, inserts a same-version successor with
    explicitly selected input/state, and writes head, lineage, charges, audit and outbox. Other
    terminal outcomes are FINAL. Preserve original deadline/initiator and finite segment count;
    a new run never resets tenant accounting. A root successor starts its own tree after quiescence;
    a child successor retains its parent's root/depth and consumes the lifetime 1,024-run tree
    budget. Cleanup cannot replenish tree or chain admission counts.
17. A signal is accepted only by an already waiting step and consumed in that transaction; there
    is no pending signal to transfer. Run commands never forward. Chain commands explicitly
    resolve/serialize the head, use chain-scoped keys and optional expected head, and reauthorize
    receipt replay. Chain cancellation sets a durable latch checked by every handoff. RERUN_AS_NEW
    starts an unrelated singleton chain. Existing waits remain segment waits; a new explicit chain
    wait ends only at FINAL, and client timeout stops waiting rather than the workflow.
18. For v2, lock chain before its current run; preserve applicable scheduler/drain/session/activity
    admission ordering. Take the tree-admission advisory lock for creation before ordered
    subordinate resources. Handoff inserts its successor and does not lock two preexisting runs.
    Direct-run mutations of a chain member follow the same chain-before-run order. Never impose
    arbitrary sorted run IDs on the old parent-first creation path.
19. Persist a parent child-instance link with original parent instance, child chain, cancellation
    latch, final outcome and settlement marker. Parent transitions lock parent chain/run then link;
    child handoff/finalization lock child chain/run then link. Neither obtains the other existing
    chain/run row. Parent cancellation writes the link latch; every handoff checks it under lock,
    and cancellation reconciliation follows the current child segment. Final chain outcome is
    applied once to the original parent instance under the parent's scheduler fence. Intermediate
    continued segments do not emit child completion. Keep legacy v1 parent/child behavior until
    explicitly migrated; its child-row UPDATE locks must not be mixed into the new lock protocol.

## Approved dependencies and delivery sequence

Features 001/002/003/004/005/006/008/009/010/011 are approved dependencies for their scoped contracts,
identity/persistence, compensation, inbox/outbox, scoped sandbox-attempt identity, SDK negotiation, projection/telemetry, RBAC/quotas,
scheduler ownership and pinned-version behavior. Feature 007 permits only safe display of unknown
revisions and scoped runtime projections in this delivery, not full new-node authoring. No separate
service, procedural workflow engine or sandbox capability is added.

The accepted product scope is all six runtime capabilities, delivered in tested slices: foundation
and old-root replay; bounded collections; loops; approval/switch; declared operations; continuation.
The first vertical collection slice has an activity and body return at two indices with concurrency
one, then expands to the required 17-item/concurrency-three case and full failure/parity evidence.
Design acceptance closes T014-002 only. Canonical contracts, code, migrations and all runtime tasks
remain pending until their named evidence exists.

## Alternatives and consequences

Graph back-edges and procedural replay would change the execution model. Flattening scoped names
into `NodeId` hides independent identity and collides at module boundaries. A separate loop engine
or compensation service duplicates durable ownership. Completion-order compensation violates
dependency semantics. Sorted cross-run locking conflicts with existing parent-first writers;
durable child links avoid requiring parent and child transactions to acquire each other's run.

The cost is wider identity, additive tables/indexes, more events, explicit descendant admission and
chain-link retention. The conservative derived-instance reserve can reject forward work before
100,000 actual activations; the exposed budget reports admitted/reserved consumption. The benefit
is one reducer/recovery model and a drain path that is not defeated by its own forward ceiling.

## Review criteria and pending evidence

Before release, prove old-root UUID/event replay and all accepted worker minors; two-sided commit
barriers, lost broker delivery, stale scheduler/worker fences; nested admission/policy reduction;
unknown effects and failed compensation; authorization/revocation/redaction; chain/parent races and
retention; and reviewed five-SDK conformance. Measure transaction/lock time, memory, journal/storage
growth and scheduler delay for the plan's workloads, including 10,000-item materialization.
Record failures/skips and operational recovery cost. None of these results is supplied by this ADR.

Disable new publications/starts to roll back capability, but retain compatible readers and workers
until v2 runs drain. Changing scope identity, compensation dependencies, lock order, evaluator caps,
value policy or in-flight compatibility requires a new decision with the constitution's evidence.
