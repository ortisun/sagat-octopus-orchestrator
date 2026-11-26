# Targeted re-read of design 046 after review round 3: architecture (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: 4b6a3e6d00f2cf217fa7a708f24c532283945767
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A targeted re-read of the
changes since round 3 (approval rule of Owner decision 2026-11-25-2), not a full review; the reviewer read the diff since
`4c35bed4`, the design, the contracts, the round-3 report and resolution, and the crates it names; it ran nothing.

## Round-3 findings

1. Typed admission on both lease paths: answered (feature 023's plan still names only `lease_one`: Nit 10).
2. Backfill: not answered (Major 3). 3. Silent wait: answered in substance (the pass raises Major 5). 4. ADR lag: partly
(Nit 9). 5. Gateway isolation: not answered (Majors 1 and 2). 6. Switch matching: partly (Major 4). 7 to 11: answered.

## Findings

1. Major. The gateway reads its public store through `BlobSettings` of `crates/infra-blobstore`, which depends on
   `sagat-application` and reads `SAGAT_BLOB_*`; a transitive rule like the lake runner's fails once the gateway is built,
   or gets weakened. Fix: a crate without `sagat-application`, or `object_store` directly; state the allow-list by name.
2. Major. No task creates the `check-architecture.py` rules, the gateway allow-list or their negative tests, though
   ADR-0120's "Enforced by" relies on them. Fix: a task, with the `architecture-principles.md` rows.
3. Major. Claims and quota count only jobs of `BUILDING` versions and `BACKFILL` jobs belong to `READY` versions, so none
   can be claimed; the protocol has no `backfill` phase or completion shape. Fix: claims accept `BACKFILL` on `READY`,
   the phase in the protocol, `a_target_added_later_gets_records_without_republishing`.
4. Major. The switch evaluates ranges through `PackageRules` while holding the snapshot row `FOR UPDATE` over an unbounded
   set of rows, stalling publication; `infra-postgres` cannot call `infra-packages`, and no layer is named. Fix: the
   `StaleInputs` pattern: evaluate outside the lock, pass the pairs and the highest `locked_seq` seen, refuse and loop.
5. Major. Two passes fail steps "through `reject_input`", a private worker-session method that records `Actor::Worker`;
   the boundary is unstated, and the "row the block pass reads" is only `steps_failed_at`. Fix: an application-level
   `fail_ready_step` with a lock, recheck and system actor; a barrier test against a concurrent lease.
6. Nit. "Owner decisions needed" cites OD-2026-11-24-312 and OD-2026-11-25-2, not yet in `docs/owner-decisions.md`.
7. Nit. Data-model drift: re-evaluations "per namespace"; the page's lock order omits the reservation-revision row; the
   blocks table should name its pass marker.
8. Nit. `FOR SHARE` needs an UPDATE privilege, so `sagat_app`'s lock of `package_advisory_snapshots` needs a narrow way
   that is not table UPDATE; the import role needs SELECT on `script_locked_packages`; test as the runtime role.
9. Nit. ADR-0120 decision 3 says "private index routes"; both ADRs say "rounds 1 and 2"; ADR-0121's "Enforced by" omits
   the barrier test and `a_large_import_is_held`.
10. Nit. Feature 023's plan describes only `lease_one`.
11. Nit. `telemetry.md` lacks `script_steps_waiting_for_layer` and a materialization-failure counter.

Counts: Blocker 0, Major 5, Nit 6

## The author's answers (all fixed; Decision 046-073)

1. `object_store` directly in `apps/package-gateway/src/store.rs`; the allow-list names `sagat-infra-egress`,
   `sagat-domain` and `sagat-telemetry`. 2. Task 046-26. 3. `BACKFILL` claimable on `READY`, its own bound, a completion
   that writes only the record; the test named. 4. Range evaluation in the application outside the lock, the switch
   refusing with `StaleInputs` above the highest `locked_seq` read. 5. `fail_ready_step` with a per-step lock, recheck,
   `system:engine` and the full write set; `a_lease_racing_the_block_pass_is_left_to_admission`. 6. The rows are
   registered with the approval (Owner decision 2026-11-24-312), and Owner decision 2026-11-25-2 is cited with its pull
   request. 7. Fixed in the data model. 8. `sagat_lock_advisory_snapshot_share(ecosystem)`, `SECURITY DEFINER`; the
   import role's SELECT. 9. Fixed in both ADRs. 10. Fixed in feature 023's plan. 11. A gauge and two counters.
