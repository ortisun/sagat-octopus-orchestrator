# ADR-0142: A federated person is a member linked to external identities; provisioning and groups arrive as durable rows, and a claim never authorizes by itself

**Status:** Accepted (2026-11-26, OD-2026-11-26-909)
**Decided by:** agent proposal (design 013, 2026-11-24), accepted with design 013's approval under Owner decision 2026-11-24-4 (OD-2026-11-26-909)
**Refines / supersedes:** refines Architecture decision 0064 (feature 029: identity-provider groups join the evaluator through the group-source contract, amended by design 013; SCIM is the source in 0.3.0) and the feature 009 membership model (one built-in role per membership); supersedes none
**Enforced by:** planned in design 013 and not existing yet: `tests/integration/federated-login/identities.rs` (two connections asserting the same e-mail address create two members unless the person links them in their own sign-in; a connection cannot assert a subject that another connection or the deployment issuer owns); `tests/integration/federated-login/groups.rs` (a group claim confers nothing; a SCIM group confers nothing until an assignment names it; a SCIM group change applies on the next request); the property test of the role mapping (order of groups and rules does not change the result; the highest role wins; a pinned role is never changed)

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
4. Groups from a provider MUST become 029 external groups with durable memberships, delivered by **SCIM**: the
   provider pushes groups and their members; an accepted change applies on the next request and never expires by
   silence. A group confers nothing until an administrator assigns it a role or a folder entry (feature 029); its
   appearance in a claim grants nothing. Groups taken from a sign-in assertion are not built in 0.3.0 (design 013,
   Decision 013-007); adding them needs a decision on how a snapshot stops conferring.
5. A membership MUST have one owning connection (the one that provisioned it, or the one an administrator names);
   only its rules and its SCIM may change the member's role or status. Its rules map only SCIM groups of that
   connection, by an ordered list; the highest role any rule gives MUST win; the result MUST be computed on stored
   groups only and MUST change the membership's role only when it differs and the member's role is not pinned by an
   administrator. A change is audited and applies feature 029's revocation of custom grants (Requirement 029-024,
   amended to keep external memberships).
6. A SCIM deactivation or deletion by the owning connection MUST suspend or remove the membership, revoke its sign-in
   sessions and the grants feature 029 revokes on a role change (and the reveal grants and secret access entries) in
   the same transaction, write no feature 027 app-session row (an app session checks the membership and its sign-in
   session on each call), and keep the external identity for the audit trail. The serving process writes memberships
   only through definer functions; its database role keeps `SELECT` only.

## Considered options

1. **Membership plus linked external identities, durable groups from SCIM (chosen).** One person per tenant whatever
   the provider; offboarding by SCIM; groups an administrator can see and revoke; no claim is ever authority.
2. **The provider's subject as the platform subject.** No link table. Rejected: two connections of one tenant could
   produce the same subject string for two people, and moving a tenant to a new provider would orphan every
   approval policy and grant that names a subject.
3. **Link by verified e-mail.** Convenient. Rejected: e-mail addresses are reassigned and some providers let a user
   set an unverified one; linking by e-mail is a known account-takeover path.
4. **Group claims evaluated from the token on each request.** No storage. Rejected for the reasons feature 029 gives:
   not revocable before expiry, not visible, not auditable.
5. **SCIM groups that expire by silence (feature 029's draft contract).** Fail closed on a silent source. Rejected for
   SCIM: some providers push only changes, so a stable organization would lose its access after the bound.
6. **Groups also from the sign-in assertion, stored per session and bounded** (an earlier draft of design 013).
   Rejected for 0.3.0 after the reviews of 2026-11-25: a role tied to session liveness in every membership read, a union
   across sessions, an expiry audit and an amendment of feature 029, for what SCIM gives durably; a later design may
   add it.

## Consequences

- Feature 029's group-source contract is amended (SCIM in 0.3.0; the assertion kind deferred) in the same delivery.
- Feature 016's Gap 290 (no way to add a member) is closed by just-in-time provisioning, SCIM and the member routes of
  design 013.
- Approval policies keep naming subjects; a federated member's subject is `federated:<uuid>`, chosen in the Studio from the
  member list (design 050, Gap 050-004).
- Rollback: disabling a connection stops new sign-ins and SCIM writes; members and their links stay.

## Evidence

The membership model is read from `crates/infra-postgres/migrations/20261008000000_identity.up.sql` and
`crates/application/src/identity.rs`; the contract from `specs/029-custom-roles-folders/contracts/group-source.md`, on
`develop` at 2026-11-24. The planned tests are listed under **Enforced by**; none exists yet.
