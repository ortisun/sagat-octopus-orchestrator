# Targeted architecture re-read of design 013 for approval, round 9 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: a8820014ea23e0d9a2b5305437956597d29e197a
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a confirmation of round 8 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. Both round-8 findings are closed, and the changed paragraph adds nothing a builder would trip on.
Counts: Blocker 0, Major 0, Nit 1

1. **Nit. Technical Plan 9, the assertions of `the_control_plane_renders_two_openbao_agents_when_federation_is_on`.** The test checks that file paths lie under mounts and that no mount path repeats, not that every mount names a rendered volume, so a CA mount moved without its volume passes the render and `helm lint` and fails only at apply. Fix: in both modes, every `volumeMounts[].name` of every container and init container of the control-plane pod names an entry of the pod's `volumes`.
