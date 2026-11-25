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
| `SCIM` | an identity provider through design 013's SCIM 2.0 endpoint of one connection, into rows of `access_groups` and `access_group_members` with source `SCIM` (design 013, Decision 013-017) | a full member set (`PUT`) or member additions and removals (`PATCH`) of one group | a SCIM group does not expire by silence: the provider pushes every change, deprovisioning included; design 013 raises an informational alert after 7 days without any request on the connection |
| `ASSERTION` | a sign-in through one connection whose groups claim or attribute is mapped, into the same tables with source `ASSERTION` | the full set of that person's groups for that connection | ends when the sign-in session that wrote it ends (revoked or expired), and at most 24 hours after the sign-in; past it the memberships confer nothing until the next sign-in |

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
   one changes nothing), so a retried request has no second effect. An assertion replaces the person's set for that
   connection as a whole.
4. **Idempotent, bounded deliveries.** At most 1,000 member operations per SCIM request and 256 group values per
   assertion; a larger delivery is refused before any change. A SCIM request is one transaction.
5. **Freshness.** As the table above. A stale membership confers nothing (fail closed) and is not deleted.
6. **Role changes.** When a member's built-in role changes, by an administrator or by design 013's group-to-role
   mapping, feature 029's revocation of custom grants applies (FR-029-024); external memberships stay, and the
   assignments that name an external group are unaffected.

## What the platform side must keep

- A SCIM delivery is one transaction that takes the affected memberships, then the exclusive tenant access fence (Architecture decision 0050, slot 4a), with a writer `lock_timeout`. An assertion is written at the sign-in's redemption without the exclusive fence (design 013, Technical Plan 4, which argues why the fence's guarantee still holds); a narrowing it brings governs every request authorized after its commit.
- External groups and their members are read-only through feature 029's REST routes (`EXTERNAL_GROUP_READ_ONLY`).
- Membership in a fresh external group is evaluated like a local one; it never bypasses the human-role,
  reserved-permission or non-escalation rules. Linking an external group to roles or folders is an ordinary
  assignment or entry by an administrator, and a group that appears in a claim grants nothing until one names it.
