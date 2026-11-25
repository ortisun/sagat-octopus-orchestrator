# ADR-0123: Hub integrations are signed data that specialise closed operation kinds, and are imported as pinned versions

**Status:** Proposed
**Decided by:** agent proposal (design 047, 2026-11-24), to be accepted or rejected under Owner decision 2026-11-24-4 when design 047 completes its review
**Refines / supersedes:** refines the curated template hub of feature 028 (Requirements 028-039 to 028-042: the signed index, its verification, import as a draft) and ADR-0055 (the closed resource-type catalog of feature 021) as admitted by constitution 5.0.0 (Owner decision 2026-11-24-2); supersedes none
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
2. A hub resource type MUST specialise one of feature 021's closed operation kinds (`http_api` and `graphql` in 0.3.0):
   it fixes the hosts, the auth scheme and its provider, narrows the settings schema, and may declare named request
   presets (method, path template, typed parameters, a body template) that the integration runner evaluates with its
   existing `http.request` or `graphql.execute` under the existing evaluator. A new kind of operation is code and enters
   only through a release of the platform.
3. Items live in this repository under `hub/`, each with a review record, are validated in CI by the platform's own
   codecs, and are signed into the release's index by the release key (feature 028's format: Ed25519 over the RFC 8785
   canonical form, a version that never decreases, an expiry, a SHA-256 per file). An operator may add indexes signed by
   keys the operator pins. There is no tenant-configured source, no public submission, no rating and no payment.
4. Importing a resource type or a provider creates an immutable, tenant-scoped version that records its provenance
   (index, item, version, digest, signer); resources pin it as they pin a platform type. A newer hub version is a new
   tenant version, adopted by a resource only through an explicit new revision. Importing a template creates a draft
   (feature 028). An item revoked by a later index cannot be imported; versions already imported get a finding, and an
   operator may block their new admissions.

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
