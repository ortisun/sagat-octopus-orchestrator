# Security review of design 052, full round of 2026-11-26

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: d734e0266bc4333e3e2cf8944a37034d296ab278
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing; each claim is verified in the code or marked as a suspicion.

## Major

1. Tenants can reach the platform's broker, and the design adds the first subscriber that acts on what is published
   there, without requiring the broker to authenticate or restrict the wake subjects. Verified: a tenant's NATS
   publisher and connector run in the engine pod, which the network policy lets reach the platform's NATS; the guard
   checks resolved addresses, then the client resolves the name again (a DNS rebind passes); a tenant admin with
   `egress:manage` can allow a private range covering the broker; the platform's broker credentials are optional in
   Helm and absent in Compose. Suspected: discovered servers are followed by default. A tenant's publish on another
   tenant's wake subject wakes that tenant's streams on every replica. Fix: cross-process wakes only when the broker
   authenticates the platform with its own user, publish and subscribe on `sagat.wake.v1.>` for engines, publish only
   for control planes, no JetStream API; the chart fails when the wake is on without credentials outside development;
   a gap for the guard's bypass (pin the checked addresses, `ignore_discovered_servers()`).
2. A wake removes the per-stream claim-rate bound `idle_retry` gives today (each empty claim re-authenticates, reads the
   session, scans and probes), to about 100 claims per second per stream with 10 ms coalescing, on every replica; the
   same legitimately when the woken stream cannot take the step; returning a refused stream to parking adds load while
   shedding; a count in the message is the attacker's. Fix: a woken claim that leased nothing makes the stream ignore
   wakes until `idle_retry` has passed; at most one stream per message and a token bucket per key on receipt; an
   `empty` outcome counter; correct the cost stated in Non-functional requirement 052-002 and ADR-0134.

## Nit

1. The subject should carry the namespace identifier and an opaque activity token (activity names are tenants' and the
   encoder is lossy); the key taken from the subject only.
2. The spread's hash must be of the server-issued session identifier, never the client's `worker_id`.
3. The wake subscriber on a connection of its own with a bounded channel; a received wake only increments entries that
   exist.
4. The gate trusts a threshold the pull request can edit (sd in a Markdown report; the harness from the head tree).
   Read them from the base; changes to the gate's files select the gate.
5. Gate job hygiene: `pull_request` only, no `pull_request_target` or `workflow_run` with a head checkout, results as a
   summary or artifact, nothing committed back; `timeout-minutes` and a concurrency group.

Tenant boundary and timing, authorization after a revocation, metric labels, secrets in CI: checked, no finding.

Counts: Blocker 0, Major 2, Nit 5
