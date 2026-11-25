# ADR-0137: Standalone steps and draft test runs are ordinary runs of private definitions; a test tree is isolated by a run flag and its own gate pool, and a pin is a journaled completion

**Status:** Accepted (2026-11-25, delegate OD-2026-11-24-4, after three review rounds recorded in specs/022-flow-control-step-policies/reviews/2026-11-25/); revised after the reviews of `14b9e40b`, the re-reviews of `6d02632e` and the third reads of `c910345b` (decisions D022-222 to D022-271)
**Decided by:** agent proposal (feature 022, design of the rest of release 0.3.0, 2026-11-24) under the owner's delegation (OD-2026-11-24-4, "Tomem a melhor decisão"), revised with the coordinating session's rulings of 2026-11-25 (decisions D022-222 to D022-240, then D022-242 to D022-259); accepted once the three reviewers find no Blocker in a re-review (task T022-200)
**Refines / supersedes:** extends ADR-0035 decision 9 (ad hoc work is ordinary durable execution of an internally retained definition) from database nodes to standalone activity, script and database steps and to draft test runs; refines ADR-0050 decision 7 (the slots of the new creators and the retained definition as built, amended 2026-11-24 and 2026-11-25) and ADR-0056 decisions 1 and 3 (the draft-test pool is derived from a run flag; the new creators admit through the one port); builds on ADR-0089 decision 1 and ADR-0094 (invariant 11 admits a mock) and ADR-0050 decision 9 (fleet activation). Supersedes none.
**Enforced by:** proposed checks, built by the slices of US05 and US06 in `specs/022-flow-control-step-policies/tasks.md`: `run_start_errors` of `scripts/check-architecture.py` (a domain start call outside `Engine::start` is refused, already built) and a new rule that refuses a literal gate pool outside `GatePool::of_run` (task T022-204); `integration_flow_control_step_policies::{standalone, test_runs, run_creators}` (one case per new creator, task T022-221; the review refusal where approvals are required, task T022-229); the slot-order case of a publication of a workflow with a database node against a test admission of its draft (`tests/fault/schedules-bulk-operations/slot_order.rs`, task T022-224); `crates/domain/tests/test_runs.rs` (upstream pins route without activating, `A → X` and `A → B → C` with `startAt = C`, task T022-233); invariants 7 and 15 of `data-model.md` in `support::flow_control::key_consistency` and `verify_run`; the guards of `runs.test_tree` (a `BEFORE UPDATE OF test_tree` trigger, `ENABLE ALWAYS`, and the check at insert that a child's flag equals its parent's); the exhaustive classification test of effect node kinds (task T022-218); `fault_flow_control_step_policies::{standalone, test_runs}`; the tenant battery for both routes (task T022-058).

## Context

Feature 022 US05 (FR-022-023 to FR-022-025) starts one activity, script or managed database step durably without a
published workflow; US06 (FR-022-026 to FR-022-030) runs a workflow's draft as a test, from a chosen node, with upstream
outputs pinned and downstream steps mocked. Feature 017 proved the pattern for database nodes: a private, immutable
definition retained per tenant, namespace, actor and digest, then an ordinary run (`crates/application/src/runs/ad_hoc.rs`,
ADR-0035 decision 9). What the tree adds since the approval:

- Every run creator admits through `Engine::start` (D022-049); a domain start call elsewhere is refused by
  `scripts/check-architecture.py`.
- The gate rows carry a pool, `ORDINARY` or `DRAFT_TEST` (`GatePool`, `crates/domain/src/execution/flow_control.rs`);
  about thirty sites pass the ordinary pool as a literal.
- Publication locks `workflows FOR UPDATE` and then the namespace's secrets (`crates/infra-postgres/src/definitions.rs:618-642`,
  `quotas.rs:306-312`, `database/admission.rs:209`), and version activation locks the row first
  (`versioning.rs:534-537`): an admission that takes the secrets and then the workflow row would cycle with them.
- A namespace may require publication approvals (`publication_policies.required_approvals`, migration
  `20261016000000_versioning`); a draft test run or a standalone step would run code nobody reviewed.
- 017's `create` retains the private definition and takes the workflows quota lock before the slot-4a locks
  (`crates/infra-postgres/src/execution.rs:1531` against `1619-1640`).
- Feature 018's business identifier is on the tree; its contract lists `RESET_SOURCE_TEST_RUN` as feature 022's, which no
  code raises yet.
- A mocked node completes without grants; an upstream pin that routed like a completion would activate its successors.

## Decision

1. **Both are ordinary runs of a private definition, admitted through the one port.** A standalone step and a draft test
   run MUST be admitted through `Engine::start` as an ordinary run of a private retained definition (017's retention,
   shared, taken where `create` takes it today: `ad_hoc::retain` with its workflows quota lock before the slot-4a locks),
   with origin `STANDALONE_STEP` or `DRAFT_TEST`, a required idempotency key and its receipt. Listing, inspection,
   cancellation, retry and result disclosure use the existing run routes.
2. **Unreviewed code runs only where the namespace allows it.** A draft test run MUST be refused in a `prod` namespace,
   and in a namespace whose effective `required_approvals` is above zero unless its publication policy enables draft
   executions (a setting, off by default, changed through the publication-policy route with its permission and
   audited). Standalone activity and script steps follow the same rule; feature 017's database executions keep their
   accepted behaviour. In a `prod` namespace standalone activity and script steps are refused too unless the setting is
   enabled, while draft test runs stay refused in `prod` whatever the setting (O1160). The setting is read inside the
   admission transaction under an advisory key per namespace, held shared at slot 4a by the admissions that read it and
   exclusively by `set_policy` (`crates/infra-postgres/src/versioning.rs`), which locks nothing after it but its row
   and audit (decision D022-269); runs already admitted continue when it is
   turned off; creating the policy row to set the flag keeps the environment's approval default; a rerun of a
   standalone run is checked again. Refusals are `403 FORBIDDEN` with `TEST_RUN_PRODUCTION_NAMESPACE`,
   `TEST_RUN_REVIEW_REQUIRED`, `STANDALONE_STEP_REVIEW_REQUIRED` or `STANDALONE_STEP_PRODUCTION_NAMESPACE`, decided
   before any row exists, after the workflow's `404` (the check order:
   namespace from the authenticated context, `404` for the workflow, the `403`, then a `fromRun` `404` before
   `PIN_NOT_DISCLOSABLE`).
3. **A standalone step.** Exactly one `activity`, `script` or `database` node with its own retry, timeout, secrets,
   idempotency template, `reconcileWith`, `cache` and node-level gates; `compensateWith`, `next` and `onError` are
   refused; a script node accepts only the artifact of a published script version. The private one-node definition's
   result is the node's output, its `outputSchema` the node's. The start MAY carry 018's `businessId` with its conflict
   and reuse policies, claimed at slot 3. The default group of the node's gate MUST be derived from the executable
   identity (the activity name or the script artifact's digest), shared by every standalone execution of it in the
   namespace, and MUST NOT equal a group any published workflow can derive.
4. **A draft test run reads its draft without a lock.** The draft row is read without a lock, its `draft_revision`
   compared with the request's (a moved draft is `409 CONFLICT`, `DRAFT_REVISION_CHANGED`), and what the statement read
   is validated with the publication rules and the capabilities in force and copied into the immutable private
   definition; the publisher is the actor; nothing is published. Nothing waits on the workflow row, so no cycle with
   publication or activation can form.
5. **A test tree is isolated by a run flag.** Admission MUST set `runs.test_tree = true` for a `DRAFT_TEST` run, every
   child admission MUST copy its parent's flag (the database checks at insert that a child's flag equals its parent's),
   and the flag never changes (a `BEFORE UPDATE OF test_tree` trigger, `ENABLE ALWAYS`). Every gate decision, grant,
   timeout, listing and journal event MUST take its pool from one function of the run (`GatePool::of_run`: `DRAFT_TEST`
   for a test tree), never from a literal. A test tree MUST NOT create outbound deliveries, fire triggers, dispatch a
   namespace handler, or read or write any cache scope but the `version` scope of its own private version (a test-tree
   run of another version bypasses the cache); it counts against the test-run quotas and the ordinary run quotas; 018's
   reset, rerun and batch admissions MUST refuse it as a source (`RESET_SOURCE_TEST_RUN`); a continue-as-new successor
   of a test-tree run is refused at admission: a draft test of a definition that continues as new is refused `422
   VALIDATION_FAILED` (`CONTINUE_AS_NEW_REFUSED`), a child admission under a test tree fails its child step with that
   code, and `refuse_gated_successor` stays as a defence that ends the run through a typed finalization (decisions
   D022-249, D022-263).
6. **A pin is a journaled completion that routes without activating.** An upstream pin (a node outside what `startAt`
   reaches) MUST be committed in the admission transaction (slot 6, with the run: its payload row, its `test_run_pins`
   row, its `step.mocked` and a `step.routed` with no activation), so no successor of an upstream pin is released;
   `startAt` is recorded in `run.started` (`start_at`), so replay reaches the same entry. A downstream pin MUST complete
   when the reducer releases the node, in that commit, with `step.mocked` and no lease, attempt, lookup, gate, child run
   or wait. Pinned nodes are excluded from compensation and reconciliation planning. Invariant 11 admits `step.mocked`.
   A pin never enters a definition (D1186). The highest floor among a test run's pins applies to every disclosure of
   that test run.
7. **Effects are refused unless pinned.** Admission MUST refuse a draft whose graph reachable from `startAt` holds an
   unpinned node of an effect kind (`VALIDATION_FAILED`, `TEST_RUN_UNMOCKED_EFFECT`, naming the node). The classification
   is an exhaustive `match` over the node kinds; no kind on the tree is an effect kind today, and a new kind does not
   compile until it is classified.
8. **Secrets of an unmocked step** resolve under the actor's current `secret:use` and access lists at each resolution,
   each resolution audited (`test_run.secret_resolved`, never a value).
9. **Fleet activation.** `step-executions-v1` and `draft-test-runs-v1` are registry capabilities required of control-plane
   and engine processes, listed by a binary only once every slice of their story is in it; until activation the routes
   refuse (`FLOW_CONTROL_ACTIVATION_PENDING`).

## Considered options

1. **Ordinary runs of private definitions, a run flag, a derived pool, pins as completions that route without
   activating** — chosen. One state machine, one admission port, one set of run routes; the flag is a single fact every
   rule reads; a pin is a fact the journal replays.
2. **A separate execution model for standalone steps**, as Temporal's standalone activities — rejected: a second state
   machine and a second identifier registry beside 018's.
3. **Test runs in a separate namespace or schema** — rejected: it would copy definitions, secrets and connections, and
   the environment check would then be about the copy, not about where the effects land.
4. **The draft row `FOR SHARE` at slot 4b** (the first text) — rejected after the reviews: it cycles with publication
   and activation, which lock the row before the namespace's secrets.
5. **Refuse every test run where approvals are required** — rejected: it removes the review use of draft tests; and
   **allow them silently** — rejected: unreviewed code with real secrets.
6. **The pool chosen per call site** — rejected: thirty places to be wrong; one derivation and a fitness rule cannot
   disagree.
7. **Pins written into the private definition** — rejected (D1186).
8. **Mocked nodes leased to a mock worker** — rejected: leases and attempts for work that never happens.

## Consequences

- Good: a one-off job and a test have every guarantee of a run (idempotent start, retries, cancellation, audit, gates);
  a test tree cannot block, be blocked by, poison the cache of, or notify on behalf of production work; unreviewed code
  runs only where the namespace said it may.
- Bad: a draft test run creates a retained private definition per actor and draft digest (bounded by the workflow quota);
  the pool threading touches every gate site once; the separate draft-test pool adds its own load on a shared external
  system (a runbook section); the shared default group of a standalone activity is one key space for every actor.
- Harder: every future creator copies the test-tree flag to its children; every future node kind is classified for test
  runs before it compiles; every future reset or rerun path refuses a test-tree source.
- Compatibility and rollback: additive origins (the `origin` CHECK extended `NOT VALID`, then validated), columns and
  routes; `database-executions` stays and shares the path; every `down` refuses while rows of the new origins exist.
  Disabling a capability refuses new starts; existing runs finish; never a binary downgrade once journals hold the new
  events.

## Evidence

None yet: the tests named under **Enforced by** are built by tasks T022-039 to T022-047, T022-204, T022-216 to T022-219,
T022-221, T022-224, T022-229, T022-233 and T022-237. Spec: FR-022-023 to FR-022-030 and their amendments of 2026-11-24
and 2026-11-25; research: D1183 to D1189, D1601, D022-200, D022-202, D022-209, D022-211, D022-212, D022-214, D022-218,
D022-219, D022-222, D022-226, D022-227, D022-233, D022-237, D022-239, D022-249, D022-257; data model: `test_run_pins`, the `runs` columns and
invariants 7 and 15.
