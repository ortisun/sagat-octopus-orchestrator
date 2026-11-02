# Actual released-worker in-flight lease checkpoint

Refs: T014-016/017/040/043, FR-014-014/015/017. No production behavior or public capability changes.

A separate ignored coordinator extends the verified actual v0.2.0 source profile. The old
public producer completes reserve, then leases charge through its real generated gRPC1.5
client and an explicitly configured one-hour server lease. Its exact task capability and
completion request remain in that process memory. The current coordinator owns disposable
PostgreSQL, attaches that exact database, applies ordinary current migrations and starts
current runtime-role services. Before migration and immediately before GO it matches the
physical identity, path, attempt/generation, old token SHA256 commitment and wire expiry to
the database and requires more than600seconds remaining, above the entire240second scenario.
Inline report acceptance alone is not considered proof that the lease remained live.

A safely inherited anonymous Unix socket pair carries length-prefixed frames capped at16KiB.
All I/O and child joins are async and bounded; kill-on-drop cleans up an interrupted child.
No pathname listener, exposed control port or persisted capability handoff is used. The parent
refreshes only identity-provider authentication for the same existing worker subject/tenant;
it does not reconstruct, renew, replace or register another task lease. JWT/task/session tokens
and report payload bytes never enter witness files, build manifests or diagnostics.

The exact old generated client submits its in-memory request twice to the current endpoint.
First already_applied=false, then true; command_id, step_status and run_status must match.
The old process checks immutable completed/captured/journal prefixes and native replay before
exit. The parent joins it, checks one durable scoped completion, the exact retained output,
one idempotency receipt and a single logical-byte charge, then expires the stopped old
scheduler member/shards explicitly and uses actual current Keeper takeover to finish/replay.
This makes no exactly-once external-provider claim.

The runner verifies every old source file against the immutable tag and the exact five test
overlays (phase registration, phase implementation, root-wait SQL, in-flight phase and framed
transport). The coordinator additionally binds each overlay hash to its compiled inputs and
the old registration hash. Old/current build targets remain separate and source inputs stay
frozen across compilation. Previous active-data evidence is retained independently.

Run with the pinned toolchain, a private absolute CARGO_TARGET_DIR outside the repository,
and Docker reachable:

```sh
python3 scripts/test-release-upgrade-profile.py --scenario inflight-worker
```

This is a test-only released-source proof, not a distributed old SDK asset or an automatic
credential/capability rollout fence. Old API services remain idle while the old worker reports;
no old scheduler tick occurs after the metadata barrier. The old worker exits before current
scheduler takeover. Mixed-dispatcher stop/join/credential fencing, expired/stale-task behavior,
new scoped waits in old code, public activation, all-five SDKs, nesting/capacity and final
fault/resource/release acceptance remain separate gates.
