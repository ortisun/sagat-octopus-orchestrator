# Targeted security re-read of design 013 for approval, round 42 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 550edc201553827f515b732e3f31d85658de6b80
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the fixes of the seventh architecture re-read (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the egress peers, the transport check, the Kubernetes 1.29 check, the ConfigMap and its annotation, and `identity.publicOrigin` sound.
Counts: Blocker 0, Major 1, Nit 0

1. **Major. Technical Plan 9; Task 013-8.** The `<fullname>-control-plane` ServiceAccount is rendered under federation, but `sagat.openbao.serviceAccount` returns it only under `openbao.enabled`, so a federation-only pod runs as the shared `<fullname>` the engine also uses; binding `sagat-identity` there, as the text allows, puts the identity mount's role on an account shared with the engine, and the render test passes either way. Fix: the helper returns `<fullname>-control-plane` for the control plane under either value; the role is bound to that account by name; the test asserts the control plane's `serviceAccountName` in both modes.
