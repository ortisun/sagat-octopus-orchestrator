# Targeted architecture re-read of design 013 for approval, round 7 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: a183013138b16c92354361e3bfcf95239e504d9f
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a confirmation of round 6 and a re-read of the changes since (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. Round 6's Major is closed on its three named points and its Nit is closed; finding 1 is a remaining part of the Major.
Counts: Blocker 0, Major 2, Nit 1

1. **Major. Technical Plan 9, the production chart.** Objects the identity agent needs still render only under `openbao.enabled`: the agent ConfigMap and the pod's checksum annotation, the ServiceAccount objects, the projected token volume `openbao-identity`, and the Kubernetes 1.29 native-sidecar check; a federation-only render can pass the named test while referring to objects that do not exist. Fix: render those under either value; the federation-only test asserts reference closure and the ConfigMap's exact content, with a failing case on Kubernetes 1.28.
2. **Major. Technical Plan 9 against the Settings table and the start-up refusal.** The chart never sets `SAGAT_IDENTITY_FEDERATION=on` nor `SAGAT_PUBLIC_ORIGIN`, and the control-plane template has no hook for extra variables, so a chart with federation enabled runs a control plane with federation off. Fix: both variables, the origin from a new required value checked as an HTTPS origin; the render test asserts them and a missing value fails the render.
3. **Nit. Task 013-8, Verification.** The chart test's target is not run by the task's verification. Fix: run `contract_scalability_ha identity_chart` and add the module to `main.rs`.
