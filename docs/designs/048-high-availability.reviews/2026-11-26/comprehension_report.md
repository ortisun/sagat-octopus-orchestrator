# egm-comprehension on design 048, full round of 2026-11-26

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 1e28351031ae51b7b6625a86f4c4350790ffd2c1
Note: the reviewer's output, condensed by the author; every finding and severity is kept.

## Major

1. A repetition's window after the fault (4 min, 240 s) is shorter than the 300 s gate it enforces.
2. "Both standbys stopped" has no recovery step, so the 300 s gate fails by construction; say when they return and what
   that fault's pass condition is.
3. The negative control needs the driver to see a commit acknowledged by an isolated primary whose address has moved;
   say how the driver reaches it and in which order the commit and the isolation happen.
4. "Outage" is never defined or recorded, yet Requirement 048-016 and Success criterion 048-003 sort by it.
5. P, R and E have no start and end; the budget's sum reads as sequential where detection starts at the fault.
6. The connection budget contradicts itself (twice the pools, a cap of one pool, only the request pool doubled), and the
   pools are not listed.
7. What non-request work (the keeper, the lost-lease pass, the registry heartbeat, gRPC worker calls) does while
   `DATABASE_UNCONFIRMED` is set is not said.
8. Non-Goals says Requirement 048-016 names SDK versions; it names none.
9. The core HA settings are prose, not the CloudNativePG fields and Patroni keys with their values.
10. The drills' fault lists are ambiguous (whether the Kubernetes drill also runs the Compose faults; "a node stopped"
    against "the primary's container stopped").

## Nit

1. The release gate and the gate's reason are used before they are defined.
2. "Newest" identity is undefined for a pair.
3. Terms undefined or unlinked: command identifier, `begin_safely`, a passive process and a cell of feature 032, the
   fork, housekeeping, design 053's lint, expand migration, `reconcileWith`, D1585.
4. The read pool is exempted from a check defined for writer connections only.
5. The confirmation connection is not from a pool, so `acquire_timeout` does not bound it; name its timeout.
6. "A string that calls a writing function" should say a SQL string in Rust source.
7. ADR-0130 says the SDKs give up after about 1.4 s; the design says at most 3 s.
8. Tie "acknowledged work lost" to the integrity oracle.
9. Whose shards "a live shard within 16 s" are is unclear.
10. The open-loop rate and the throttling method are unspecified.

Counts: Blocker 0, Major 10, Nit 10
