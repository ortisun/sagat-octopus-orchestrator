# Identity-provider group source contract (draft; not built by feature 029)

Feature 013 (draft) excludes SCIM and the mapping of provider groups, and no approved feature delivers
them (AGENTS rule 2). Feature 029 therefore builds **no** external group schema or delivery path: only local
groups exist, token claims never authorize, and creating an external group answers
`ACCESS_CAPABILITY_DISABLED`. This page states what a future source feature (O1440: recommended, a 013
amendment pushing SCIM 2.0 Groups) must guarantee so that its groups can join the evaluator of
[evaluation.md](./evaluation.md) without changing it. The source feature owns its tables, endpoints,
credentials and tests.

## Guarantees a source must provide

1. **Tenant binding.** A source is configured for one tenant and authenticates as that tenant's connection
   (013's identity-provider connection or a successor). The tenant comes from the authenticated connection,
   never from a payload, as webhook ingress does (ADR-0026).
2. **Stable identity.** Each group has a stable external identifier (≤ 256 bytes) unique per (tenant,
   source). Members are named by the platform subject of the federated identity (013 FR-013-004: the
   provider's stable identifier, never an e-mail). A member without a human membership is ignored and
   counted, never created (013 FR-013-007).
3. **Generations.** Every delivery for a group carries a `generation`, strictly increasing per group for a
   change. A delivery with a lower generation is acknowledged and ignored. A delivery with the stored
   generation is accepted only as a heartbeat without member changes and renews freshness.
4. **Full sets and deltas.** A delivery carries either the full member set or a delta. A delta names its
   `baseGeneration` and applies only when it equals the stored generation; otherwise it is refused and the
   source must send a full set (no silent divergence after a lost delta).
5. **Idempotent, bounded deliveries.** Each delivery has an identifier recorded with a digest in the same
   transaction as its effect (inbox pattern, ADR-0012): the same digest replays the acknowledgement, a
   different digest is `IDEMPOTENCY_KEY_REUSED`. At most 1,000 member changes per delivery by default.
6. **Freshness.** The source delivers or heartbeats at least once per staleness bound (proposed default 24 h,
   hard 7 d) per group. Past the bound the group confers nothing (fail closed) and an operational alert
   opens.
7. **Role changes.** When a member's built-in role changes, their external memberships confer nothing until
   a delivery for that group is accepted after the change (consistent with FR-029-024 for local grants).

## What the platform side must keep

- One delivery is one transaction under the exclusive tenant access fence, with a writer `lock_timeout`.
- External members are read-only through the REST API (`EXTERNAL_GROUP_READ_ONLY` would be added then).
- Membership in an external group is evaluated like a local one while fresh; it never bypasses the human-role,
  reserved-permission or non-escalation rules. Linking an external group to roles or folders is an ordinary
  assignment or entry by an administrator.
