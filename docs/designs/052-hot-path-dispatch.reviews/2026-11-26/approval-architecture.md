# Approval confirmation of design 052: architecture (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: d770bb0c159f12d11e62f45cc4f327cb95a920df
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`architecture-full.md`, at d734e026),
then `reread-1-architecture.md` and `reread-2-architecture.md`, whose findings are all confirmed resolved here. Not a
full review. The reviewer read files only, ran nothing, and could not list branches.

The wake's suites stay within the constitution's selections (the claim suite for `wake.rs` and `service.rs`; the new
target when it changes and at full runs); the chart checker runs where the other checkers run until 5.1.0 admits its
step on every pull request. The correctness fixes keep the layers (the capped contended map, rank 0, the per-round
decision, the hand-off under the registry's mutex, the gate reading the merge base). Nothing stops ADR-0133 or
ADR-0134 from being accepted: both state alternatives, consequences, rollback and enforcement, and match the design.

No Blocker, no Major.

## Nit

1. The hand-off's waking mechanism is unnamed: a parked stream registers a `std::task::Waker` or a waiter port that
   `api-grpc` implements, called under the registry's mutex, so ADR-0092's check stays as it is.
2. The Constitution Check's Delivery Constraints row names only the gate, not the chart checker's step.
3. A change of the NATS wake code selects no wake test on a pull request; the subject parse, the payload rule, the own
   message and the rate limit as unit tests in `crates/infra-nats`, named in ADR-0134's Enforced by.

Counts: Blocker 0, Major 0, Nit 3
