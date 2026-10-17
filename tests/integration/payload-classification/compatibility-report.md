# Payload classification prior-release compatibility

## Evidence identity

- Requirement: T016-052, FR-016-021, FR-016-028, NFR-016-004, SC-016-004.
- Actual prior release: `v0.1.0`.
- Immutable source commit: `e06532dd0e288273f63966e0d950bb4d7c743459`.
- Supplemental unreleased baselines: none used for the release claim.
- Harness: `scripts/test-payload-classification-compat.sh`.
- Integration case: `compat::actual_v0_1_0_processes_survive_the_additive_upgrade`.

The release did not publish control-plane, execution-engine or Studio artifacts. The harness builds
them from the tagged commit with its checked-in `Cargo.lock`, `rust-toolchain.toml` and frontend
lockfile. Both old and current Rust builds use the caller-supplied coordinated Cargo target and the
default debug profile. Before the current build can replace the target's binary names, the harness
copies the old binaries to unique immutable names. It records SHA256 digests for both binaries, the
lock files, toolchain file and every Studio asset before starting the compatibility test. The release
source is a detached worktree and must remain clean. The Studio build uses a `git archive` copy
because npm writes `node_modules`.

## Scenarios

The executable gate uses one fresh PostgreSQL database and real old and current process binaries:

1. The v0.1.0 migrator creates the release schema and the v0.1.0 control plane seeds identity data.
2. The v0.1.0 execution engine reaches readiness on that schema.
3. The v0.1.0 HTTP process publishes a document containing the inert classification annotation,
   starts an in-flight run, publishes an event, and reads event and connector dead-letter values.
4. The test records the exact published document bytes, digest and run version pin.
5. The current migrator upgrades the same database. The retained version becomes rules `0` without
   changing its bytes or digest, and the run keeps its original version pin.
6. The v0.1.0 execution engine and control plane start again with migration-on-start disabled and
   read the upgraded database.
7. The current control plane serves the old Studio assets, returns `LEGACY_VERSION` without the
   retained input value, and returns structure-only event and dead-letter answers without data,
   body or headers.
8. A publication through the current HTTP process stores rules `1`. A new run discloses its public
   input while the in-flight release run remains pinned to rules `0` and opaque.

The old process continues returning raw event `data`, dead-letter `body` and dead-letter `headers`
after the additive database upgrade. The current process replaces those three fields with structural
answers. This comparison is the documented event-read break; it is not claimed as byte-compatible.

## Command and result

Command:

```console
CARGO_TARGET_DIR=<coordinated-target> \
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 \
scripts/test-payload-classification-compat.sh
```

Result: **pending an explicitly granted shared-target build window**. No Cargo or npm build was run
while the target was reserved. Update this section only with the actual command output, SHA256
manifest path and observed old-binary failure if the release cannot read the intervening schema.
T016-052 remains open while that process evidence and the browser/SDK limits below are unresolved.

## Limits

- Reconstructing a current service is not accepted as release-process evidence; this gate launches
  the actual binaries produced from the immutable release commit.
- This gate builds the old Studio bundle and reads its index over HTTP from the current server's
  configured static mount. It does not launch Chromium or exercise the bundle's browser behavior.
- The five old SDK bundles are not built or executed by this gate. Their compatibility remains
  separate multi-language protocol conformance evidence.
