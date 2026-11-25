# ADR-0142: A federated person is a member linked to external identities; provisioning and groups arrive as durable rows, and a claim never authorizes by itself

**Status:** Proposed
**Decided by:** agent proposal (design 013, 2026-11-24), for acceptance under Owner decision 2026-11-24-4 once the review of design 013 is complete
**Refines / supersedes:** refines Architecture decision 0064 (feature 029: identity-provider groups join the evaluator through the group-source contract, amended by design 013 for two source kinds) and the feature 009 membership model (one built-in role per membership); supersedes none
**Enforced by:** planned in design 013 and not existing yet: `tests/integration/federated-login/identities.rs` (two connections asserting the same e-mail address create two members unless an administrator links them; a connection cannot assert a subject that another connection or the deployment issuer owns); `tests/integration/federated-login/groups.rs` (a group claim confers nothing until the assertion is stored and an assignment names the group; an assertion-sourced membership confers nothing after its freshness bound; a SCIM group change applies on the next request); the property test of the role mapping (order of groups and rules does not change the result; the highest role wins; a pinned role is never changed)

## Context

The platform's identity is a membership `(tenant, subject, role)`. Feature 029 adds groups, custom roles and folders,
and refused claim-based groups: "Token group or role claims MUST NOT confer any permission", because a claim cannot be
revoked before the token expires and is not visible to an administrator. It wrote a contract a future source must meet
(`specs/029-custom-roles-folders/contracts/group-source.md`). Feature 016 recorded that there is no supported way to
add a member (Gap 290).

Design 013 brings people from each organization's identity provider, by sign-in (OIDC, SAML) and by provisioning
(SCIM 2.0). It must decide what a person is, how a provider's identity maps to it, how groups and roles arrive, and how
they leave.

## Decision

1. A person in a tenant MUST be a membership. A federated identity MUST be an external identity row: (tenant,
   connection, the provider's stable identifier), linked to exactly one membership subject. The stable identifier is
   the OIDC subject claim (`sub`, or `oid` with `tid` for Entra ID) or the SAML persistent name identifier (or a
   configured immutable attribute), never an e-mail address or a display name. A connection's issuer or entity
   identifier is immutable, so that a subject is never re-read under another provider.
2. A membership created by federation MUST get the platform subject `federated:<uuid>`. An external identity MAY be linked
   to an existing subject (a member created before federation, or a person moving between providers) only inside a
   sign-in by that subject, who confirms it; the link is audited. An administrator MAY unlink and pre-provision, never
   attach an identity to a subject that already has one or to an `admin` subject. The platform MUST NOT link two
   identities because their e-mail addresses match.
3. Just-in-time provisioning, when a connection enables it, MUST create the membership, its external identity and its
   first role in the sign-in transaction, with a default role of `viewer`, `operator` or `developer` (never `admin`).
   SCIM provisioning MUST do the same for a user the provider pushes, before the person's first sign-in.
4. Groups from a provider MUST become 029 external groups with durable memberships, from two source kinds: **SCIM**
   (the provider pushes groups and their members; an accepted change applies on the next request and never expires by
   silence) and **sign-in assertion** (the groups claim or attribute of a sign-in, stored as that person's full set
   for that connection; it confers nothing once the sign-in session that wrote it ends, by revocation or expiry, and
   at most 24 hours after the sign-in). A group confers nothing until an administrator assigns it a role or a folder entry
   (feature 029); its appearance in a claim grants nothing.
5. A membership MUST have one owning connection (the one that provisioned it, or the one an administrator names);
   only its rules and its SCIM may change the member's role or status. Its rules map only groups sourced from that
   connection, by an ordered list; the highest role any rule gives MUST win; the result MUST be computed on stored
   groups only and MUST change the membership's role only when it differs and the member's role is not pinned by an
   administrator. A role mapped from assertion groups MUST be stored with its source and the expiry of those groups
   and fall back to the connection's default role when they stop conferring, so a claim never leaves a durable role
   behind. A change is audited and applies feature 029's revocation of custom grants (Requirement 029-024).
6. A SCIM deactivation or deletion by the owning connection MUST suspend or remove the membership, revoke its sign-in
   sessions and its app sessions in the same transaction, and keep the external identity for the audit trail.

## Considered options

1. **Membership plus linked external identities, durable groups from two source kinds (chosen).** One person per
   tenant whatever the provider; offboarding by SCIM; groups an administrator can see and revoke; claims are inputs
   stored on arrival, never authority.
2. **The provider's subject as the platform subject.** No link table. Rejected: two connections of one tenant could
   produce the same subject string for two people, and moving a tenant to a new provider would orphan every
   approval policy and grant that names a subject.
3. **Link by verified e-mail.** Convenient. Rejected: e-mail addresses are reassigned and some providers let a user
   set an unverified one; linking by e-mail is a known account-takeover path.
4. **Group claims evaluated from the token on each request.** No storage. Rejected for the reasons feature 029 gives:
   not revocable before expiry, not visible, not auditable.
5. **SCIM groups that expire by silence (feature 029's draft contract).** Fail closed on a silent source. Rejected for
   SCIM: some providers push only changes, so a stable organization would lose its access after the bound; the
   assertion source keeps the bound because a snapshot has no other way to end.

## Consequences

- Feature 029's group-source contract is amended (two source kinds, freshness per kind) in the same delivery.
- Feature 016's Gap 290 (no way to add a member) is closed by just-in-time provisioning, SCIM and the member routes of
  design 013.
- Approval policies keep naming subjects; a federated member's subject is `federated:<uuid>`, chosen in the Studio from the
  member list (design 050, Gap 050-004).
- Rollback: disabling a connection stops new sign-ins and SCIM writes; members and their links stay.

## Evidence

The membership model is read from `crates/infra-postgres/migrations/20261008000000_identity.up.sql` and
`crates/application/src/identity.rs`; the contract from `specs/029-custom-roles-folders/contracts/group-source.md`, on
`develop` at 2026-11-24. The planned tests are listed under **Enforced by**; none exists yet.
