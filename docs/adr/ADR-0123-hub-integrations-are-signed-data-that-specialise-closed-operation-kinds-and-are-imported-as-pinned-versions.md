# ADR-0123: Hub integrations are signed data that specialise closed operation kinds, and are imported as pinned versions

**Status:** Proposed
**Decided by:** agent proposal (design 047, 2026-11-24, revised after review round 1), to be accepted or rejected under Owner decision 2026-11-24-4 when design 047 completes its review
**Refines / supersedes:** refines ADR-0074 (the operator's block and offers) and the curated template hub of feature 028 (Requirements 028-039 to 028-042: the signed index, its verification, import as a draft) and ADR-0055 (the closed resource-type catalog of feature 021) as admitted by constitution 5.0.0 (Owner decision 2026-11-24-2); supersedes none
**Enforced by:** planned tests of design 047: the contract suite `contract_integration_hub` that loads every item of `hub/` with the platform's own codecs and refuses an item that names an operation kind outside the closed set, a host outside https, a wildcard host or an undeclared scope; the index tests of feature 028 (tampered file, unpinned key, rollback, expiry); the import tests (a pinned type version, provenance, idempotency, nothing executed). Until they exist: review only.

## Context

Windmill's hub lets anyone publish scripts, flows and resource types, which a workspace imports. The constitution
(5.0.0) admits a curated hub "whose only sources are the index signed in the release and indexes the operator
configures with keys the operator pins, whose items are imported as drafts under the same publication, signature,
secret and isolation rules as authored content, and which runs nothing in the server". Feature 028 designs the signed
index and the import of workflow and script templates. Feature 021 has a closed set of resource types and operation
kinds executed by its integration runner, and refused namespace-defined types (its Owner question 1122).

## Decision

1. A hub item is data: an OAuth provider definition, a resource type, a script template, a workflow template, or an
   integration that groups them. No item carries code that the server, the engine or the integration runner executes.
   Script templates are source that becomes a draft and runs only in the sandbox after publication.
2. A hub resource type MUST narrow one of feature 021's operation kinds and authentication schemes (feature 021,
   Decision 021-004): fixed hosts, or one single-label host pattern bound once per credential from the token response or
   the registration; a narrowed settings schema; request presets evaluated by the existing operation, whose path
   variables are each one encoded segment and whose effect class is the kind's or stricter. A new kind of operation is
   code and enters only through a release of the platform.
3. Items live in this repository under `hub/`, each with a review record naming two reviewers, are validated in
   continuous integration by the platform's own codecs, and are signed into the release's index with feature 028's
   format. Operators may add indexes signed by keys they pin, and an optional revocation feed signed with the release key.
   There is no tenant-configured source, no public submission, no rating and no payment.
4. Importing creates drafts: a draft provider or resource-type version with provenance (index, item, version, digest,
   signer), or a workflow or script draft. A provider or resource type becomes usable only when a holder of
   `hub:publish` publishes it after the same checks as authored content. Resources pin a published version; a newer hub
   version is adopted only by an explicit new revision.
5. Revocations carry a class. A `security` revocation blocks new admissions of steps whose resources pin a published
   version of the item, until an operator lifts the block (audited); an `other` revocation records a finding. A revoked
   item cannot be imported or published.
## Considered options

1. **Signed data that specialises closed kinds, imported as pinned versions (chosen).** The hub adds integrations
   without adding code paths; every import is reproducible and attributable.
2. **An open marketplace (Windmill's hub).** Excluded by the constitution and a supply-chain path for every tenant.
3. **Hub items with executable connectors (a plugin per integration).** Each would be new code in the integration
   runner's trust base, reviewed per item. Rejected for 0.3.0; a richer integration is a script template instead.
4. **Namespace-defined resource types written by tenants.** Gives flexibility without review; a tenant can still write an
   `http_api` resource with its own hosts under its egress policy, which covers the need.

## Consequences

- The platform team curates and reviews `hub/`; each release refreshes the index and its expiry.
- Integrations that need more than a templated HTTP or GraphQL request are delivered as script templates (with packages
  from design 046) or as a platform release.
- Compatibility: additive item kinds in the index schema of feature 028; tenant versions are additive rows.

## Evidence

None yet: this is a proposal. The tests named under "Enforced by" are required before acceptance.
