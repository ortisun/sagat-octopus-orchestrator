# ADR-0051: Deployment routing, task-token completion and push invocation are decided on the canonical attempt lease

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26)
**Refines / supersedes:** refines ADR-0015 (additive minors, one per wire slice), ADR-0020 (fairness algorithm reused,
scheduler unchanged) and ADR-0029 (same claim path for streams); preserves ADR-0002, ADR-0004, ADR-0012 and the
"Final V1 decision" of `docs/architecture/worker-runtime-protocol.md`; supersedes nothing.
**Enforced by:** planned tests of feature 019 — `fault_worker_deployments_dispatch` (routing-change-vs-claim and
pattern-change-vs-routing-change races, forged-token oracle and gRPC token-replay battery,
crash at every pin/detach/push commit point), `integration_worker_deployments_dispatch` (bucket function equal in
domain and SQL, claim eligibility, token fences), the `deployment-versioning`, `async-completion` and
`push-envelope` conformance scenarios, and `scripts/check-architecture.py` (SigV4/JWT/HTTP only in adapters).
Until those exist: review only.

## Context

Feature 019 ([spec](../../specs/019-worker-deployments-dispatch/spec.md)) adds four ways to decide who executes an
attempt and who may report it: worker deployment versions with pinned and auto-upgrade routing, a ramp and
retirement; priority and fairness ordering; asynchronous completion by an external system holding a task token;
and push invocation of serverless functions by the engine. Each has a tempting design that creates a second
authority next to the run row:

1. **Queues per version** (or NATS subjects per version) that workers consume. A message would be a grant; the
   worker protocol already forbids a lease granted anywhere but the Worker API's canonical transaction.
2. **Routing decided when a step becomes ready** and stored on the step. Many writers make steps ready (admission,
   completion, timers, signals, approvals, chains); a routing change would have to retarget ready rows, and a
   writer that forgets the rule routes wrongly.
3. **Task tokens as signed JWTs or stored random secrets.** A JWT needs a new signing key and cannot be revoked
   when the attempt is reclaimed; a stored secret is another credential table to protect, rotate and purge, and a
   lost response loses it.
4. **A push sender modelled on outbound deliveries** that records its own attempts. It would duplicate retry,
   timeout, cancellation and fencing semantics that the step already has.

The engine interprets a declarative IR (ADR-0003); Temporal's versioning exists largely for workflow-code replay,
which does not apply. What must be protected here is the contract of activity implementations across builds, the
fence, and the tenant boundary.

## Decision

1. **The claim transaction is the only place routing is decided.** A step routed through a deployment MUST be
   leased only if, under a shared deployment fence and the routing revision re-read inside the claim transaction,
   the claiming session's version equals the run's pin (pinned steps) or the route of the run's bucket (auto-upgrade
   steps and first pins). The only order between the namespace dispatch-catalog fence and deployment fences MUST be
   catalog → deployment: changes of deployment patterns or push-endpoint activities take the catalog fence
   exclusively and then the affected deployments' fences exclusively; routing changes take the catalog fence shared
   and then their deployment's fence exclusively with a CAS on the revision; claims and registrations take both
   shared. Every committed lease was therefore eligible under the catalog and routing at its commit, and a pattern
   change cannot race a routing change into a state neither checked. These fences take slot 4 of ADR-0050's merged
   admission lock order (catalog before deployment; coordinator decision C4). The bucket MUST be one deterministic function
   (eight hash bytes) of run id and deployment id, identical in domain and SQL. Discovery outside the transaction MUST remain a hint.
2. **Pins are journaled facts.** A pin MUST be created in the claim (or admission, or chain-handoff) transaction that
   needs it and journaled there; a repin MUST be journaled and audited. Pins MUST NOT change the workflow version a
   run references.
3. **A version grants no authority.** Registration with a deployment identity MUST still require `worker:execute`
   and a binding; drains, secret pools, activity quotas and breakers MUST apply unchanged. Sessions without a
   deployment, including every session of an older protocol minor, MUST be served as unversioned.
4. **Priority and fairness only reorder.** They MUST select among candidates the session is already entitled to,
   inside one namespace, using strict priority and the weighted max-min share of ADR-0020 between fairness keys.
5. **A task token is an encoding of the lease.** It MUST encode namespace, run, instance digest, attempt,
   generation and the existing lease token; the server MUST NOT store a new secret for it, and any reclaim (new
   generation) MUST invalidate it. The server MUST verify the token's secret first, in constant time, against the
   current lease hash and the retained hashes of the step's ended detached or push-held leases; any mismatch MUST be
   the opaque `NOT_FOUND`, and `STALE_ATTEMPT` or `NOT_DETACHED` MAY be answered only after a match, so the API is no
   oracle for runs, attempts or generations. Using it MUST also require an authenticated principal holding
   `task:complete` in the namespace and, for a machine identity, a binding covering the step's activity (owner
   question O1041 asks whether a token alone may ever suffice). External heartbeats and reports MUST run the same
   fenced transactions as worker reports, with idempotency receipts keyed by principal and read only after the
   secret check, and the existing two-phase secret scan. Every worker-protocol call that takes a lease token MUST
   refuse a lease whose holder is `detached` or `push`, so a decoded token cannot be replayed on gRPC.
6. **Push invocation is a claim by an engine-side invoker.** The invoker MUST claim through the canonical claim
   (run lock, then the shard fence, the shared catalog fence and tenant → namespace → endpoint push fences with exact
   counts), MUST perform HTTP with no transaction or lock held, MUST renew and report under that lease with a stable
   idempotency key, and MUST use the guarded egress client. The envelope MUST carry no credential except, for an
   attempt allowed to complete asynchronously, its task token — which, by point 5, the function can use only with its
   own platform identity; such an attempt MUST accept that external report even while the invoker still holds it
   (before the `202` is recorded), first commit wins. A push invocation is case (b) of the placement rule held by
   ADR-0055 (a fixed, platform-defined envelope to a tenant-configured destination, sent by the engine through the
   guarded egress client with only that destination's credential; coordinator decision C3), and its signing key is
   listed in 032's key inventory (C11). Because one platform key signs every tenant's envelopes,
   verifiers MUST require the expected tenant, namespace and endpoint and reject passed deadlines. The engine MUST
   resolve provider credentials as the endpoint revision's secret binding, rechecked at every invocation against the
   revision's named binding principal (the ADR-0025 broker-credential precedent), and MUST only sign requests or mint
   short-lived ID tokens whose audience is derived from the target's origin, through the pinned token endpoint. A
   timeout after sending MUST be `OUTCOME_UNKNOWN`.
7. **One additive protocol minor per wire slice** (deployment identity, checkpoints, detach), each a superset of the
   previous and advertised only when its slice's evidence passes; the server MUST keep serving every older minor.

## Considered options

1. **Every dispatch mode is a variation of the attempt lease (chosen).** One authority (run row, fence, lease
   token); every race is decided by the first commit; recovery is the existing lease reaper. Cost: the claim query
   gains predicates whose discovery cost must be measured (G1040), and a push replica holds a lease while it waits.
2. **Per-version queues or broker subjects.** Rejected: a delivered message would grant work, contradicting
   ADR-0002 and the worker-protocol decision; a lost or duplicated message would need a second reconciliation.
3. **Routing assigned at readiness.** Rejected: spreads the rule to every writer and requires retargeting ready
   rows on each routing change.
4. **Stored or signed task tokens.** Rejected: new key or secret lifecycle, irrecoverable after a lost response,
   and revocation that does not follow the fence.
5. **A separate push delivery pipeline (ADR-0025 style) with its own attempts.** Rejected: duplicates step retry,
   timeout and cancellation semantics and would let a delivery succeed for an attempt the run has already
   reclaimed.

## Consequences

- Operators can roll out builds gradually, keep incompatible runs pinned and retire builds only when nothing needs
  them; nothing changes for activities outside deployments or for workers of older minors.
- The claim becomes more expensive for ramping and pinned versions; ceilings are provisional until the benchmark.
- A detached attempt is still a lease: it expires, is reclaimed with a new generation and can be raced by a timeout
  or a cancellation, all decided by the first commit.
- The engine process makes outbound calls with provider credentials it resolves per invocation, like outbound
  webhook delivery today; private networks would need a separate executor (G1044).
- Rollback: disable the capabilities; keep readers until detached attempts and pins drain; an older engine is not a
  safe reader of runs with detached attempts (ADR-0032 point 8).

## Evidence

None yet: this ADR is a proposal with planned tests (Enforced by). Design sources:
[plan](../../specs/019-worker-deployments-dispatch/plan.md#transaction-boundaries),
[research](../../specs/019-worker-deployments-dispatch/research.md) D1042–D1064,
[contracts](../../specs/019-worker-deployments-dispatch/contracts/).
