# ADR-0137: Standalone steps and draft test runs are ordinary runs of private definitions; a test tree is isolated by a run flag and its own gate pool, and a pin is a journaled completion

**Status:** Proposed
**Decided by:** agent proposal (feature 022, design of the rest of release 0.3.0, 2026-11-24) under the owner's delegation (OD-2026-11-24-4, "Tomem a melhor decisão"); the coordinating session accepts or rejects it after the independent architecture, correctness and security reviews (task T022-200)
**Refines / supersedes:** extends ADR-0035 decision 9 (ad hoc work is ordinary durable execution of an internally retained definition) from database nodes to standalone activity, script and database steps and to draft test runs; refines ADR-0050 decision 7 (where the draft workflow row and the pins sit in the merged admission order, amended 2026-11-24) and ADR-0056 decisions 1 and 3 (the draft-test pool is derived from a run flag; the new creators admit through the one port); builds on ADR-0089 decision 1 and ADR-0094 (invariant 11 admits a mock) and ADR-0050 decision 9 (fleet activation). Supersedes none.
**Enforced by:** proposed checks, built by the slices of US05 and US06 in `specs/022-flow-control-step-policies/tasks.md`: `run_start_errors` of `scripts/check-architecture.py` (a domain start call outside `Engine::start` is refused, already built) and a new rule that refuses a literal gate pool outside `GatePool::of_run` (task T022-204); `integration_flow_control_step_policies::{standalone, test_runs, run_creators}` (one case per new creator, task T022-221); invariant 7 of `data-model.md` in `support::flow_control::key_consistency`; the exhaustive classification test of effect node kinds (task T022-218); `fault_flow_control_step_policies::{standalone, test_runs}`; the tenant battery for both routes (task T022-058).

## Context

Feature 022 US05 (FR-022-023 to FR-022-025) starts one activity, script or managed database step durably without a
published workflow; US06 (FR-022-026 to FR-022-030) runs a workflow's draft as a test, from a chosen node, with upstream
outputs pinned and downstream steps mocked. Feature 017 proved the pattern for database nodes: a private, immutable
definition retained per tenant, namespace, actor and digest, then an ordinary run (`crates/application/src/runs/ad_hoc.rs`,
ADR-0035 decision 9). What the tree adds since the approval:

- Every run creator admits through `Engine::start` (D022-049), which evaluates the start's gate keys; a domain start call
  elsewhere is refused by `scripts/check-architecture.py`. The approved texts still say "the existing admission
  transaction" (017's `RunStore::create`).
- The gate rows carry a pool, `ORDINARY` or `DRAFT_TEST` (`GatePool`, `crates/domain/src/execution/flow_control.rs`), so
  that test trees never block or are blocked by ordinary runs; about thirty sites pass the ordinary pool as a literal.
- Feature 018's business identifier (FR-018-018 to FR-018-020) is on the tree; its contract lists the refusal
  `RESET_SOURCE_TEST_RUN` as feature 022's, which no code raises yet.
- Invariant 11 requires a gated step leaving towards readiness to commit with its grants or its `*.queued`; a mocked
  node completes without either.
- Features 021, 024, 025 and 030, whose effect node kinds a test must never run unmocked, are not on the tree.

## Decision

1. **Both are ordinary runs of a private definition, admitted through the one port.** A standalone step and a draft test
   run MUST be admitted through `Engine::start` as an ordinary run of a private retained definition (017's retention,
   shared), with origin `STANDALONE_STEP` or `DRAFT_TEST`, a required idempotency key and its receipt: the same key and
   body replay the same run after current authorization, a changed body is `CONFLICT`. Listing, inspection,
   cancellation, retry and result disclosure use the existing run routes.
2. **A standalone step.** Exactly one `activity`, `script` or `database` node with its own retry, timeout, secrets,
   idempotency template, `reconcileWith`, `cache` and node-level gates; `compensateWith`, `next` and `onError` are refused.
   The private one-node definition's result is the node's output, its `outputSchema` the node's. The start MAY carry 018's
   `businessId` with its conflict and reuse policies, claimed at slot 3. The default group of the node's gate MUST be
   derived from the executable identity (the activity name or the script artifact's digest), never from the private
   definition, and MUST NOT equal a group any published workflow can derive.
3. **A draft test run.** The draft at the named revision is validated with the publication rules and the capabilities in
   force, its workflow row read `FOR SHARE` at slot 4b (a moved draft is `CONFLICT`); the private definition's publisher
   is the actor; nothing is published. A namespace whose environment is `prod` MUST refuse before any row exists
   (`FORBIDDEN`, `TEST_RUN_PRODUCTION_NAMESPACE`).
4. **A test tree is isolated by a run flag.** Admission MUST set `runs.test_tree = true` for a `DRAFT_TEST` run, every
   child admission MUST copy its parent's flag, and the flag never changes. Every gate decision, grant, timeout, listing
   and journal event MUST take its pool from one function of the run (`GatePool::of_run`: `DRAFT_TEST` for a test tree),
   never from a literal. A test tree MUST NOT create outbound deliveries, fire triggers, dispatch a namespace handler, or
   read or write the `workflow` and `namespace` cache scopes; it counts against the test-run quotas and the ordinary run
   quotas; 018's reset, rerun and batch admissions MUST refuse it as a source (`RESET_SOURCE_TEST_RUN`).
5. **A pin is a journaled completion.** An upstream pin (a node before `startAt`) MUST be committed in the admission
   transaction (slot 6, with the run: its payload row, its `test_run_pins` row and its `step.mocked`); a downstream pin
   MUST complete when the reducer releases the node, in that commit, with `step.mocked` and no lease, attempt, lookup,
   gate, child run or wait. Pinned nodes are excluded from compensation and reconciliation planning. Invariant 11 admits
   `step.mocked`. A pin never enters a definition (D1186).
6. **Effects are refused unless pinned.** Admission MUST refuse a draft whose graph reachable from `startAt` holds an
   unpinned node of an effect kind (`VALIDATION_FAILED`, `TEST_RUN_UNMOCKED_EFFECT`, naming the node). The classification
   is an exhaustive `match` over the node kinds; no kind on the tree is an effect kind today, and a new kind does not
   compile until it is classified.
7. **Secrets of an unmocked step** resolve under the actor's current `secret:use` and access lists at each resolution,
   each resolution audited (`test_run.secret_resolved`, never a value); `prod` is excluded by decision 3.
8. **Fleet activation.** `step-executions-v1` and `draft-test-runs-v1` are registry capabilities required of control-plane
   and engine processes; until activation the routes refuse (`FLOW_CONTROL_ACTIVATION_PENDING`).

## Considered options

1. **Ordinary runs of private definitions, a run flag, a derived pool, pins as completions** — chosen. One state machine,
   one admission port, one set of run routes; the flag is a single fact every rule reads; a pin is a fact the journal
   replays.
2. **A separate execution model for standalone steps**, as Temporal's standalone activities (own identifier space,
   describe, list, cancel) — rejected: a second state machine and a second set of routes for what an ordinary run of a
   one-node definition already is (ADR-0035 decision 9 proved it), and a second identifier registry beside 018's.
3. **Test runs in a separate namespace or schema** — rejected: it would copy definitions, secrets and connections, and
   the environment check would then be about the copy, not about where the effects land.
4. **The pool chosen per call site** — rejected (decision D022-212 of the feature's research): thirty places to be wrong;
   one derivation and a fitness rule cannot disagree.
5. **Pins written into the private definition** (pinned nodes replaced by constants) — rejected (D1186): pin values
   would enter definition digests and retention, and the journal would lose which output was pinned and from where.
6. **Mocked nodes leased to a mock worker** — rejected: leases and attempts for work that never happens, and compensation
   planning would see an attempt to undo.

## Consequences

- Good: a one-off job and a test have every guarantee of a run (idempotent start, retries, cancellation, audit, gates);
  a test tree cannot block, be blocked by, poison the cache of, or notify on behalf of production work.
- Bad: a draft test run creates a retained private definition per actor and draft digest (bounded by the workflow quota);
  the pool threading touches every gate site once.
- Harder: every future creator copies the test-tree flag to its children; every future node kind is classified for test
  runs before it compiles; every future reset or rerun path refuses a test-tree source.
- Compatibility and rollback: additive origins (`STANDALONE_STEP`, `DRAFT_TEST`), columns and routes;
  `database-executions` stays and shares the path. Disabling a capability refuses new starts; existing runs finish.

## Evidence

None yet: the tests named under **Enforced by** are built by tasks T022-039 to T022-047, T022-204, T022-216 to T022-219
and T022-221. Spec: FR-022-023 to FR-022-030 and their amendments of 2026-11-24; research: D1183 to D1189, D1601,
D022-200, D022-202, D022-209, D022-211, D022-212, D022-214, D022-218, D022-219; data model: `test_run_pins`, the `runs`
columns and invariant 7.
