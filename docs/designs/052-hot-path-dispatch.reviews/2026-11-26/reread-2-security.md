# Targeted re-read 2 of design 052: security (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: e1cfebe4e97bd6af007f1ab25e7d1ef0022e7ea6
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`reread-1-security.md` (all seven resolved), not a full review. The reviewer read files only and ran nothing.

No Blocker, no Major.

## Nit

1. A Redis channel pattern limits channels, not commands, so "the control plane publishes only" has no Redis form, and
   the engine's commands are unnamed. Fix: the full ACLs, asserted by the chart's contract test.
2. The Redis start-time probe is undefined (a `PING` answers wrongly both ways). Fix: an unauthenticated `SUBSCRIBE` and
   `PUBLISH` on a probe channel, both refused.

Counts: Blocker 0, Major 0, Nit 2

Both were fixed at the next commit.
