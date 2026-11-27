# Targeted re-read 3 of design 052: security (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: 472fb1ffa8a9fb7334303318f960f631d0b4c78c
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation of the
design after the other lenses' re-reads (the instance identifier as payload, the rank from `serving_instances`, the
per-session unary answer, the Redis wake cut, the merged counters), checked with no Blocker or Major. The reviewer read
files only and ran nothing.

No Blocker, no Major.

## Nit

1. A payload shorter than 16 bytes is not defined as malformed; a slice could panic the subscriber. Fix: a payload not
   exactly 16 bytes is malformed, dropped and counted; a test.
2. The control plane's egress rule opens with the wake whatever the transport; on Redis its empty peer list would allow
   any host on port 4222. Fix: only with the wake on, NATS and `nats.url`, with the engine's condition and peer check,
   asserted by the chart's test.

Counts: Blocker 0, Major 0, Nit 2

Both were fixed at the next commit.
