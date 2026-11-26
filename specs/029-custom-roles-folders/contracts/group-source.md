# Identity-provider group source contract

Feature 029 builds no external group source. Design 013 (federated login, `docs/designs/013-federated-login.md`)
implements this contract (O1440 answered on 2026-11-24, Decision 029-002; Architecture decision 0142). This page states
what a source guarantees so that its groups join the evaluator of [evaluation.md](./evaluation.md) without changing it.
The source owns its tables, endpoints, credentials and tests. Amended on 2026-11-24 from the draft of 2026-10-26, which
assumed one source kind that pushed or heartbeated every group within a staleness bound: design 013 has two source
kinds, and some SCIM providers push only changes, so freshness is stated per kind.

## Source kinds

| Kind | Delivered by | What one delivery carries | Freshness |
|---|---|---|---|
| `SCIM` | an identity provider through design 013's SCIM 2.0 endpoint of one connection, into rows of `access_groups` and `access_group_members` with source `SCIM` (design 013, Decision 013-017) | a full member set (`PUT`) or member additions and removals (`PATCH`) of one group | a SCIM group does not expire by silence: the provider pushes every change, deprovisioning included; design 013 marks a connection `scimFailing` after ten consecutive refused requests (a quiet directory sends nothing, so silence is not a fault) |
| `ASSERTION` (deferred) | a sign-in through one connection whose groups claim or attribute is mapped | not built in 0.3.0 (design 013, Decision 013-007 and Gap 013-011); a later design that adds it decides how a snapshot stops conferring |

## Guarantees a source must provide

1. **Tenant binding.** A source is one connection of one tenant (design 013). The tenant comes from the connection or
   the SCIM token, never from a payload, as webhook ingress does (ADR-0026).
2. **Stable identity.** Each group has a stable external value (at most 256 bytes) unique per (tenant, connection).
   Members are named by the platform subject linked to the person's external identity (design 013, Requirement
   013-011: the provider's stable identifier, never an e-mail address). A member without a human membership is
   ignored and counted, never created by a group delivery.
3. **Generations.** Every accepted change to a group's members increments that group's `generation` in the same
   transaction as the change, for both kinds. A SCIM `PATCH` is a list of additions and removals applied in arrival
   order under the tenant access fence; each is idempotent by effect (adding a present member or removing an absent
   one changes nothing), so a retried request has no second effect.
4. **Idempotent, bounded deliveries.** At most 1,000 member operations per SCIM request; a larger delivery is refused before any change. A SCIM request is one transaction.
5. **Freshness.** As the table above. A stale membership confers nothing (fail closed) and is not deleted.
6. **Role changes.** When a member's stored built-in role changes, by an administrator or by design 013's group-to-role
   mapping, feature 029's revocation of custom grants applies (FR-029-024, amended to keep external memberships).

## What the platform side must keep

- A SCIM delivery is one transaction that takes the affected memberships, then the tenant access fence (Architecture decision 0050, slot 4a), with a writer `lock_timeout` of 2 seconds (a timeout answers a retryable `503`): exclusively when the change can alter authority (a member added to or removed from a group that an assignment or a folder entry names, or a built-in role or status change), shared otherwise (design 013, Technical Plan 4). An assignment or entry that would name the group takes the fence exclusively and so waits for the delivery to commit. Whether the group is named is read again after the shared fence is taken; if it now is, the delivery rolls back and restarts with the exclusive fence (design 013's barrier test `an_assignment_naming_the_group_mid_request_forces_the_exclusive_fence`). (The assertion kind, when a later design adds it, states its own fence rule.)
- External groups and their members are read-only through feature 029's REST routes (`EXTERNAL_GROUP_READ_ONLY`).
- Membership in a fresh external group is evaluated like a local one; it never bypasses the human-role,
  reserved-permission or non-escalation rules. Linking an external group to roles or folders is an ordinary
  assignment or entry by an administrator, and a group that appears in a claim grants nothing until one names it.
