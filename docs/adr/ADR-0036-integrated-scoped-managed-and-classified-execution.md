# ADR-0036: Integrate scoped execution, managed databases and classified disclosure

Status: Accepted implementation reconciliation, 2026-10-17, within the user's authorization
to complete all open project tasks. Runtime guarantees remain gated by executed tests.

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
2. Preserve the bytes and version identities of 014 migrations. Allocate the seven unreleased
   017 migrations sequentially from 20261122000000 through 20261128000000. New classification
   and grant migrations use 20261201000000 and 20261202000000. The combined profile is tested
   from the released baseline. It does not silently rewrite a standalone 017 SQLx history.
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

## Consequences and evidence

Existing immutable versions, tenant boundaries, journal/task/timer/outbox transactions and
external-effect semantics are retained. Unsupported cross-feature compositions are explicit
capability errors; this does not close any unimplemented acceptance criterion in feature 014
or 016. Each canonical task still needs its named tests and final integrated release evidence.

Standalone unreleased 017 installations must keep their own compatible binaries and coordinated
backups until an explicit migration bridge is implemented and proven. The combined migrator
must refuse conflicting SQLx history rather than treating different migrations as equivalent.
No production upgrade or rollout is authorized by this ADR alone.
