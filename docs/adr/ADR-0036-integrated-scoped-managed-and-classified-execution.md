# ADR-0036: Integrate scoped execution, managed databases and classified disclosure

Status: Accepted implementation reconciliation, 2026-10-17, within the user's authorization
to complete all open project tasks. Runtime guarantees remain gated by executed tests.

Partially superseded by [ADR-0040](./ADR-0040-release-checkpoints-preserve-incomplete-feature-boundaries.md) only for the final release-prerequisite
sentence: implemented-subset checkpoints do not close the complete feature-014 guarantees.

Refines ADR-0030/0032 scoped execution and ADR-0033 classified disclosure, and reconciles
feature 017's managed database decision, renumbered ADR-0035. ADR-0034 remains available for
the separately owned feature 015 draft, following the owner's numbering decision.

## Context

Features 014 and 017 were implemented from different baselines. Both allocated worker minor
1.5 and SQLx migration versions 20261027000000 through 20261102000000, and 017 allocated an
ADR-0033 already used by payload classification. Their protobuf fields are additive, but
014 replaced root-only step uniqueness with scoped instance identity. Keeping either whole
side of their shared worker/store modules would lose security or durable execution behavior.

## Decision

1. Preserve every 014 protobuf identity tag. Fix scoped support at minor 1.5 and allocate
   managed database capabilities to minor 1.6. Older ordinary/scoped workers retain their
   workloads; only a currently authorized DatabaseRunner can claim managed tasks. Protocol
   negotiation never substitutes for role, binding, path and lease checks.
2. Preserve the bytes and version identities of every migration already merged into develop,
   including `20261122000000_steps_open_index` (PR #37). Allocate the unreleased 017 migrations
   after it: `20261122000100_openbao_secrets` and `20261123000000` through `20261128000000`
   (feature 017 decisions D602 and D603). New classification and grant migrations use
   20261201000000 and 20261202000000 (a later one, 20261220000000, removes a subject's reveal grants when
   the role of its membership changes), and the 014 completion migrations keep 20261203000000
   through 20261213000000 (there is no 20261206000000 in this profile). The combined profile is
   tested from the released baseline. It does not silently rewrite a standalone 017 SQLx
   history, and no applied migration is renamed. A standalone database that applied the
   standalone 017 migrations under the earlier numbers needs an explicit proven bridge; the
   combined migrator must refuse its history.
3. Managed database execution remains root-only. Its credential pins reference the full
   scoped step key with explicitly empty path arrays, and every join/disclosure checks that
   root path. Reject database nodes in structured bodies and database/continuation combinations
   at publication until per-instance pins and connection-pin handoff have their own proof.
   Do not restore global uniqueness of a definition node across scoped instances.
4. Preserve async provider I/O outside run transactions, final authority/fence rechecks,
   existing secret-pool locks and scoped instance propagation together. No external effect
   gains an exactly-once promise or an automatic retry after an uncertain write.
5. The combined permission catalog has 48 entries after classification and seven built-in
   roles including the DatabaseRunner machine role. Only developer/operator/admin gain
   payload:read-internal. No role grants payload:reveal-pii; it requires the accepted current
   namespace grant. Admin's permission set excludes that reveal permission explicitly.
6. Preserve separate accepted disclosure policies: structural inspection (ADR-0028), declared
   Query/Update results (ADR-0031), classified carrier payloads (ADR-0033), and audited managed
   database results (ADR-0035). The database node is outside classification's closed carrier
   set; annotations there are rejected, and no classifier protection is implied for its
   separately authorized result endpoint. Canary tests state the policy of each surface.
   Values explicitly mapped into another supported carrier, including the workflow result,
   follow that receiving carrier under D283. This policy performs no taint tracking. A result
   selector reads only the retained run result; it never substitutes a database step output.
   G21's `child.finished` journal payload is judged by the exact parent `child` node's
   output schema in its pinned definition and runtime scope. The child run's own input and
   result selectors continue to use the child version's schemas. No duplicate completion
   event is introduced to make a child result eligible.

## Consequences and evidence

Existing immutable versions, tenant boundaries, journal/task/timer/outbox transactions and
external-effect semantics are retained. Unsupported cross-feature compositions are explicit
capability errors; this does not close any unimplemented acceptance criterion in feature 014
or 016. Each canonical task still needs its named tests and final integrated release evidence.

Standalone unreleased 017 installations must keep their own compatible binaries and coordinated
backups until an explicit migration bridge is implemented and proven. The combined migrator
must refuse conflicting SQLx history rather than treating different migrations as equivalent.
No production upgrade or rollout is authorized by this ADR alone.


### Current-instance charge projection

The combined scope frontier reads `run_instance_usage.charged_instances`, a transactionally
maintained projection of the domain's current step predicate. This keeps admission/reservation
accounting separate and replaces the per-transition historical count. The additive migration
`20261207000000` locks run/step/usage tables while backfilling and installing its trigger; rollback
removes only derived metadata. Existing migration identities and bytes are preserved.
This is an implementation of the accepted bounded-read design, not evidence that the complete
worker, nested scope or root-finalization resource criteria have passed.


### Bounded effect progress and global work

The combined path projection shares effect/review reducers and scoped attempt command policy
with full replay, including lease-expiry/attempt-timeout handling. Unknown effects retain lookup
or operator review, and stale fences mutate neither representation. Current global status combines local work with checked existential facts about
other paths and run-wide compensation, preserving the original status precedence. The facts are
read in the same repeatable-read snapshot; mutation still checks the global run revision. New
partial indexes in 20261208000000 support the probes. This changes no SDK/wire semantics and
adds no external-effect delivery guarantee. Full worker/provider/blob/control/finalizer wiring
and the accepted large-workload evidence remain prerequisites for the complete feature-014
guarantee. Implemented-subset release checkpoints follow [ADR-0040](./ADR-0040-release-checkpoints-preserve-incomplete-feature-boundaries.md).
