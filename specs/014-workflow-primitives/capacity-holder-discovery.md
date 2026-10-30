# Bounded ChildRoot holder discovery for O705

Owner answer O705 selects a scheduler-tick sweeper with a separate transaction per native
operation. This increment prepares its read-only input. Branch `feature/014-capacity-sweeper`
was cut from `develop@888074e6` in the owned `sagat-octopus-capacity-sweeper` worktree while
PR66 validates worker lease possession independently. Dependency headffe54ab4 was integrated
with a local no-ff merge380e7014 after its corrected six-profile upgrade gate passed. PR66's
remaining full Rust/SDK jobs are still prerequisites to its merge. No dependency was merged
into develop here, and other sessions' worktrees remain untouched.

## Contract

Reuse `CapacityDiscoveryCursor`, `CapacityCandidate`, `CapacityDiscoveryPage` and the existing
ACTIVE CHILD_ROOT keyset index. A fixed input page (maximum256 plus one lookahead) MUST be
materialized before filtering group possession, selected member versus holder, requested shards
or tenant exclusions. The cursor MUST advance over skipped nonholders/tenants/shards, and a
reconstructed store MUST resume from it. A candidate requires HELD possession by that exact
member, its own tenant/namespace/chain and current head/revisions. Discovery takes no row/advisory
locks and emits no writes, audit, outbox, receipts or quota changes. Invalid windows and corrupt
metadata retain the existing typed errors. Closed span kind/counts identify the held scan.

No readiness, quiescence, scheduler authority or permission follows from a candidate. Terminal,
paused or nonrunnable heads remain observations for later revalidation. Neither this query nor
PR66 implements G715, native-command unknown-COMMIT recovery, group creation/join, quota/backfill
or a production sweeper caller. No canonical task closes here. The production activation fence
from PR66 remains required when the branches integrate. No wider publication is enabled.

## Ordered work and evidence

- [x] Check branch/worktree ownership and retain the O705 boundary.
- [ ] Observe native failing holder/pagination/lock/window tests against an explicit RED stub.
- [ ] Share the existing bounded query/decoder with separate selection, without copying SQL
  or changing vacant discovery semantics; review the exact selected-holder predicate.
- [ ] Verify runtime role, pagination/reconstruction, nonblocking held-G reads, metadata-only
  behavior, shards/tenant exclusions and the unchanged vacant regressions.
- [ ] Integrate the validated dependency and run all required gates before merge.

Heavy/native execution remains on CI per owner preference. Tests under preparation are not
claimed as executed; the production port MUST NOT be enabled with the RED scaffold.

The first bounded review found six actionable coverage gaps (shards, oversized exclusions,
all effect tables, holder-specific span/counts, actual Run revision/nonrunnable heads and
the named bounded stage helper). All were addressed before native execution. Nine native cases are
prepared. The row-lock barrier now holds all five joined tuple families under chain/run/G
order; it does not prove absence of arbitrary advisory locks. The GREEN implementation must
reuse the unchanged plain SELECT macro; direct SQL and helper calls require read-only review.

The final contract review also required actual continuation and a deterministic shard-routing
truth table. The native lifecycle is bounded and uses the dependency's600s Keeper/worker fixture.
Its conditional old-shard rejection does not stand alone: real ids can share one of64 shards.
Before GREEN, a separate selector unit table MUST observe RED against its stub, then prove BOTH
Vacant and HeldHolder modes with fixed UUID0/1 and an asserted disjoint-shard precondition:
head-only shard accepts; stable-chain-only shard rejects. No probabilistic retries or skipped
test may replace this proof. The close-span test MUST distinguish1/0 and1/1 counts, require
close/time.busy/time.idle, and exclude namespace along with all other resource ids.
