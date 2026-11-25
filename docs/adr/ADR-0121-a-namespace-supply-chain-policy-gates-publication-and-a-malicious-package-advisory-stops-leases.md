# ADR-0121: A namespace supply-chain policy gates publication, and a malicious-package advisory stops leases

**Status:** Proposed
**Decided by:** agent proposal (design 046, 2026-11-24), to be accepted or rejected under Owner decision 2026-11-24-4 when design 046 completes its review
**Refines / supersedes:** refines ADR-0057 decision 8 (the language policy checked at lease) by adding package rules to the same check; supersedes none
**Enforced by:** planned tests of design 046: the policy evaluation unit tests over fixtures (allow, deny, quarantine, severity threshold, waiver expiry, licence), the advisory matcher property tests against the version orderings of each ecosystem, the integration test in which a snapshot that marks a published package malicious makes the next lease of that version fail `SCRIPT_DENIED` in the lease transaction, and the test that an attempt already leased finishes. Until they exist: review only.

## Context

A lock pins what runs, but not whether it should. Windmill applies no policy to the packages a script imports. A
version that was clean at publication can be found vulnerable or malicious later. Blocking every version that gains a
vulnerability would stop production work on a disclosure the owner may judge harmless; blocking nothing leaves known
malware running.

## Decision

1. Each namespace has a supply-chain policy, part of the script language policy of feature 023 (User story 023-06): per
   ecosystem enabled or not, allow and deny rules by name and version range, a release-age quarantine, a vulnerability
   severity threshold, denied licences, time-limited waivers, a layer size ceiling. Every change is audited and the
   policy has a digest.
2. Publication evaluates the lock against the policy, the gateway's metadata (upload time, licence, install-script
   flag, yanked) and the platform's advisory snapshot (OSV records imported by the control plane, or by an operator
   when offline). A version that fails is refused with a typed reason before anything is built or signed. The result,
   with every finding, is stored with the version and its digest is in the signed bundle.
3. A package matched by an advisory of the malicious kind is always refused, at publication and at lease; no policy
   or waiver can allow it.
4. When a new snapshot is imported, the control plane re-evaluates the stored locks. A version that now contains a
   malicious package is marked blocked, and the engine MUST NOT lease its script steps: a step is failed in the lease
   transaction as non-retryable `SCRIPT_DENIED`, reason `PACKAGE_BLOCKED`, with its journal event, next work, outbox
   rows and audit record, exactly as ADR-0057 decision 8 does for a disabled language. An attempt already leased
   finishes. Other new findings are reported (finding list, audit, metric) and block leases only if the namespace
   policy says so.
5. The block is materialized in the row the lease path already reads for the language check of ADR-0057, so the claim
   path gains no statement.

## Considered options

1. **Gate at publication, report afterwards, block leases only for malware unless the policy asks for more (chosen).**
   Known malware never runs; a vulnerability disclosure becomes a visible finding with a clear owner.
2. **Gate at publication only.** Malware found after publication keeps running. Rejected.
3. **Block every new finding at lease.** One disclosure can stop a business process whose exposure is nil. Kept as an
   option of the policy, off by default.
4. **Query an online vulnerability service at each publication.** No offline mode, a third party sees every package
   a tenant uses, and an outage blocks publication. Rejected for an imported snapshot.

## Consequences

- The platform carries a snapshot of advisories and a matcher per ecosystem (npm semantic versions, PEP 440).
- An operator must keep the snapshot current; its age is a metric and a warning on publication.
- A blocked version stops at its next script step. The runbook says how to publish a fixed version and how runs that
  failed can be rerun.

## Evidence

None yet: this is a proposal. The tests named under "Enforced by" are required before acceptance.
