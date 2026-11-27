Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: c00d7999f24de5eef7cbe2e2a95d1059bf27f03e
Reviewer: egm-zoom-out (egm-review stage 8, intermediate zoom-out, round 2)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 0, Major 2, Nit 4

Context check passed. Round 1: findings 1 to 5, 7 and 9 answered; 6 not answered (finding 1 below); 8 partly (finding 5).

1. **Major.** The gateway has no database and a job token with only a policy digest, yet filters by up to 1,000 rule rows, the quarantine and reserved names, and lists private versions held in PostgreSQL; range matching is in the adapter, not the domain; most of the filter repeats the resolver flags and the evaluation. Fix: narrow the gateway's filter to hiding reserved names and serving uploads; pass bounded inputs as claims or one cached internal lookup.
2. **Major.** The control plane downloads OSV archives from the internet, contradicting "the gateway is the only process with internet egress" and the Helm rule. Fix: the gateway fetches the archives as an upstream and the control plane pulls them over the internal listener.
3. **Nit.** Three Python lock paths; let the probe choose one before Task 046-4.
4. **Nit.** The bytecode compile container and materialize-ahead are unmeasured optimisations.
5. **Nit.** Low-level details still in the design (severity mapping, modes, wheel data, volume names, metric and audit names).
6. **Nit.** Cut the warm-container paragraph to one sentence.
