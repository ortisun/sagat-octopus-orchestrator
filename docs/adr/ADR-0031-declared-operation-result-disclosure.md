# ADR-0031 — Declared operations disclose only their authorized business results

Status: **Accepted policy; implementation and release evidence pending** | Date: 2026-10-14 | Feature: 014 | Refines ADR-0028 for declared Query/Update results; preserves its structural inspection policy

## Context

ADR-0028 hides every payload scalar and object key in the general inspection API. A schema,
example, default or credential heuristic does not authorize value disclosure. Feature 014 Query
and Update would return business results, including idempotent receipt replays, so they require
an explicit separate policy decision before release.

This decision authorizes an intentionally published, bounded business operation response. It does
not establish customer-wide classification, permit raw payload export or change the existing
structure-only inspection endpoint. Its guarantees concern declared response boundaries and
capability isolation; it cannot infer whether an arbitrary business string is confidential.

## Decision

### Explicit operation policy and authorization

1. Each Query/Update declaration in the additive v1 IR (ADR-0032) must contain `disclosure: "declared-business-result-v1"` in
   addition to its argument/result schemas and expressions. There is no implicit disclosure value
   inferred from a schema annotation. The publisher deliberately selects that operation's output
   and authorizes its declared business values for callers with the operation permission. Existing
   workflow publication/approval/version rules pin this declaration for a run. No operation is
   available without the declaration and the server's enabled capability.
2. Add `run:query` and `run:update` to the built-in permission catalog; grant both to admin,
   developer and operator, neither to viewer, worker nor runner. A Query also requires `run:read`.
   These are namespace-scoped grants covering all declared operation names in that authorized
   namespace. The operation name selects an immutable declaration; it is not an unimplemented
   per-name ACL or a new custom-role facility. Permission to inspect structural payloads, signal,
   run a worker or approve publication grants neither operation implicitly.
3. Resolve the authenticated tenant/namespace and authorize every call, including stored receipt
   replay, before exposing data. Missing/foreign resources are opaque NOT_FOUND; restricted denial
   auditing must not become an existence oracle. Policy/membership revocation takes effect on the
   next admitted call. Rate admission uses the resolved canonical namespace and cannot debit a
   foreign namespace. Apply the same policy to explicit chain-addressed operations, returning the
   actual resolved run ID without silently moving run-addressed writes.

### What is and is not disclosed

4. Only the declared result is returned, with bounded operation/run/revision/receipt metadata.
   Result schemas must be closed and bounded: recursively declared object properties, no arbitrary
   additional properties, bounded arrays/strings/depth, and at most 256 KiB serialized result.
   Ordinary declared scalar values and declared object keys are intentionally visible to the
   authorized caller. Validate the result before persistence/response; failures are safe typed
   errors without excerpts. Returning a full input/state object is permitted only when that object
   itself conforms to this explicitly disclosed, closed result schema; no raw reveal/download,
   arbitrary path selection, external blob fetch or generic payload-reader operation is added.
5. The evaluator receives only the operation's ordinary workflow input/state/completed results/
   arguments, under the pinned declaration. It receives no credentials, secret resolver, capability
   token, network client, live lease or callback. Existing secret-bearing values remain forbidden
   from persisted workflow data and responses; preserve the existing secret-boundary checks and
   fail closed if known protected secret material is detected. Secret-marker tests must include
   input/output fields outside the declared projection, credentials and runtime secret capabilities.
   Heuristic detection is defense in depth, not proof that arbitrary customer data is nonsensitive.
   Authors must select business results appropriate for the granted callers; this policy does not
   create a customer classification engine or authorize confidential fields by schema alone.
6. The normal run resource, graph, SSE, journal listing, audit and telemetry remain payload-free.
   ADR-0028's inspection API still hides all values and keys and keeps its own permissions. Return
   operation responses with `Cache-Control: no-store`; no operation arguments/results go in URL
   query strings, logs, span attributes, audit reasons or persistent Studio caches. This delivery
   adds no Studio raw-value viewer.

### Snapshot, receipts and audit

7. Query observes one committed snapshot of input, business state and completed outputs, returns
   `runId`, `runVersion` and `stateRevision`, and changes no workflow journal/state/task/timer/
   subscription. A retained terminal run remains queryable. Audit access before returning the
   result; audit/commit failure returns no result. Bound evaluation to the accepted expression
   subset from ADR-0030 and validate its result. Query access auditing uses the existing access
   audit mechanism; repeated reads may collapse by actor/run/operation within 300 seconds without
   changing the workflow journal. Arguments/results are never part of the audit key or reason.
8. A new Update is admitted only in RUNNING/WAITING. Under the run/chain serialization it checks
   current authorization, key replay/hash, mutability, argument schema, optional expected state
   revision and validator, then computes/validates new state and result. Expressions use one
   pre-state; result may explicitly reference `newState`. Successful application increments state
   revision exactly once and commits state, result/receipt, journal, audit and outbox notification
   together, scheduling no work. Unrelated journal events do not change state revision.
9. An admitted business-validator or state-revision rejection persists a stable REJECTED receipt
   and audit, with no state event. Authorization/schema/rate failures before admission have no
   receipt. A transient failure leaves no committed mutation and can be retried with the same key.
   Receipt replay precedes later terminal/mutability refusal but follows current authorization.
   Same-key changed content, including a different operation, conflicts. A chain key remains bound
   to its original applied/rejected receipt across rollover. SDK transport retry never invents a
   new operation key, and client cancellation does not undo a committed update.
10. Read/decision evidence uses server-generated fixed reason codes and only actor/resource/
    operation/outcome/revision/time/correlation metadata. A successful value return or mutation
    requires its audit to succeed. Permission denials use the existing restricted denial path;
    failure to record a denial never authorizes access. Operation receipts/state/results count
    against durable content and retention quotas. After authorized resource removal return
    NOT_FOUND, never recreate an operation from an expired deduplication key.

## Alternatives and consequences

Reusing structural inspection permission would change ADR-0028's audience and purpose. Treating a
schema as disclosure consent repeats the explicitly rejected assumption. Generic raw-state reads
or field-selection APIs widen exposure beyond a reviewed operation; arbitrary asynchronous Update
handlers add a separate lifecycle and are outside this increment.

Explicit disclosure is more restrictive to publish but makes the newly authorized boundary visible
and immutable. The built-in role grants are coarse: all authorized named operations in a namespace
share them. A product requiring a narrower audience needs a future authorization decision; this
implementation must not pretend to provide per-operation roles. Results intentionally disclose
business values, so audit, response bounds, no-store behavior and revocation checks are release
requirements rather than optional observability.

## Review criteria and pending evidence

US06 remains disabled until real PostgreSQL/API tests prove snapshot consistency, atomic result/
state/audit/receipt, lost-response replay, stable rejection, current authorization on replay,
cross-tenant opacity, no-store responses, closed-schema bounds and secret/capability isolation.
Prove no Query workflow mutation and no Update scheduling; execute the shared public scenarios in
all five SDKs, including decimals/UTC and chain rollover. Measure evaluation/audit/receipt costs and
document finite retention. This ADR accepts the policy, not any untested implementation guarantee.

A broader audience, dynamic per-name ACL, raw value viewer/export, secret access, arbitrary handler
or revised classification rule requires another decision and corresponding security/failure tests.
