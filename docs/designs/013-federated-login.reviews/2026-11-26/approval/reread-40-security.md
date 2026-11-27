# Targeted security re-read of design 013 for approval, round 40 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 9076875d3447780d9ba90a637e260f85ce09e938
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-39 fixes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the peer and port validation refusing an empty peer list, the egress rule on the control plane only, and the provider's pieces staying on `openbao.enabled`.
Counts: Blocker 0, Major 1, Nit 1

1. **Major. Technical Plan 9; Technical Plan 2, the start-up refusals.** Only the chart's transport check gains federation; the control plane's own refusal (`apps/control-plane/src/config.rs`) fires only with secret storage, so outside the chart a federation-only control plane starts in production on a plaintext listener serving connection secrets and refresh tokens. Fix: the start-up refusal gains the federation case, listed among the start-up refusals, with a configuration test.
2. **Nit. Technical Plan 9.** The shared condition of `_helpers.tpl` also guards the engine's gRPC clause, so a federation-only install would need an engine plaintext flag that later silently covers real secrets. Fix: federation joins only the control-plane clause; the renderable federation-only case sets nothing on `engine.grpc`.
