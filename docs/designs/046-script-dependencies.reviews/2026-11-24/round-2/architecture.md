Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: c00d7999f24de5eef7cbe2e2a95d1059bf27f03e
Reviewer: architecture (repository agent `.claude/agents/architecture-reviewer.md`, round 2)
Agent-blob: architecture-reviewer
Counts: Blocker 0, Major 7, Nit 6

Condensed by the author; every finding and severity kept. Round 1: answered 1 to 9, 12 to 14, 16, 17; not answered 10 (finding 7), 11 (findings 3 and 4), 15 (findings 5 and 11).

1. **Major.** The gateway has no database and a token with only a policy digest; no contract brings the rules, the quarantine cut, reservations, the private index or job status; asking the control plane breaks "only the package store's credential"; materialization ids have no table; a test run's resolve has no token. Fix: the control plane signs a per-job filter document at claim that the runner attaches; token lifetime equal to the job deadline; a test-run token kind.
2. **Major.** The control plane downloads and parses OSV archives from the internet, contradicting the gateway as the only internet-facing component. Fix: the gateway fetches; the control plane imports over the internal listener with bounded decompression.
3. **Major.** ADR-0121 decision 5 contradicts the accepted ADR-0057 amendment and feature 023's Decisions 1627 and 1235 and plan 8a (a column on the artifact row). Fix: supersede that clause and update ADR-0057, Decision 1627 and plan 8a in this delivery.
4. **Major.** Two features create `script_build_provenance`; feature 023 bounds the envelope at 65,536 bytes, too small for 2,000 dependencies. Fix: one owner; reference the lock and SBOM digests instead of listing dependencies, or raise the bound with arithmetic.
5. **Major.** One import replaces up to 200,000 records while holding the snapshot row `FOR UPDATE`. Fix: stage by snapshot id in paged transactions; a short switch transaction; a held-lock test.
6. **Major.** Phase A writes malware blocks enforced only by Phase B's Task 046-17. Fix: move it into Phase A or refuse activation until it merged.
7. **Major.** The bundle freezes the deployment's target set; adding a target strands published versions. Fix: a consequence in ADR-0120 and the alternative of per-target layer digests in a separately signed record; a typed refusal for no layer on any target.
8. **Nit.** "In the provenance" for the report contradicts Technical Plan 6.
9. **Nit.** The third check names `crates/infra-egress`; the gateway links `crates/infra-packages` for range matching.
10. **Nit.** `scope` literal `*` versus null; use a nullable namespace with `UNIQUE NULLS NOT DISTINCT`.
11. **Nit.** No re-evaluation backlog or import duration metric; no permission for removing a malware block.
12. **Nit.** Feature 023's Decision 1237 reserves `languages.{typescript,python}.packages`; Decision 1625 says the layer is in the blob store.
13. **Nit.** Fitness checks: no token in resolve or install containers; Helm lets only the gateway leave the cluster.
