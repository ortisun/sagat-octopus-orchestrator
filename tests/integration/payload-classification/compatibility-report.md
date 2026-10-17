# Payload classification prior-release compatibility

## Evidence identity

- Requirement: T016-052, FR-016-021, FR-016-028, NFR-016-004, SC-016-004.
- Actual prior release: `v0.1.0`.
- Immutable source commit: `6ede1c7849c4ac5a209c08da8ad25294fbdd7795`.
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

The executable gate asserts the following using one fresh PostgreSQL database and real old and
current process binaries. This list describes the gate; executed results are recorded below:

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

The gate requires the old process to continue returning raw event `data`, dead-letter `body` and
dead-letter `headers` after the additive database upgrade. The current process must replace those
three fields with structural answers. This comparison is the documented event-read break; it is not
claimed as byte-compatible.

## Command and result

Command:

```console
CARGO_TARGET_DIR=<coordinated-target> \
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 \
scripts/test-payload-classification-compat.sh
```

Result: **in progress; not a passing upgrade gate**. The actual annotated `v0.1.0` tag resolves to
`6ede1c7849c4ac5a209c08da8ad25294fbdd7795`; the original harness incorrectly pinned its parent and
refused before building. The corrected source produced both released debug binaries (40.46 s),
the exact locked Studio bundle and the current debug control plane (1m38s). The manifest is retained
at `/Users/marcelo/Documents/Projects/sagat-octopus-compat016-artifacts/run.1mfG99/artifacts/SHA256SUMS`.

Subsequent test prerequisites exposed the new SHA-256 array's lack of `LowerHex`, missing explicit
HTTP crypto-provider initialization and the event deployment gate's required key ring. The harness
now uses bytewise hex encoding, initializes the pinned provider and shares a random test-only ring
between its child processes. Logs are retained outside temporary source worktrees. Reused local
crate metadata is refreshed before switching the coordinated target back to current source.

The next executed attempt reached publication, run creation and the current migration sequence,
then failed at 20261205000000 with PostgreSQL 55006: CREATE INDEX on steps had pending deferred
quota-trigger events (`/tmp/sagat-completion-compat-release-4.log`). A focused populated-step
regression reproduced that error, then passed 1/1 after moving index creation before the backfill
(`/tmp/sagat-completion-discovery-migration-{red,green}.log`). All 60 released migration files remain
byte-identical. Only the new unreleased discovery migration was reordered, in the same transaction.

The old/current full-process gate must be repeated with a rebuilt current binary containing that
new checksum. It has not passed yet. T016-052 remains open for that run and the browser/SDK limits.

## Limits

- Reconstructing a current service is not accepted as release-process evidence; this gate launches
  the actual binaries produced from the immutable release commit.
- This gate builds the old Studio bundle and reads its index over HTTP from the current server's
  configured static mount. It does not launch Chromium or exercise the bundle's browser behavior.
- The five old SDK bundles are not built or executed by this gate. Their compatibility remains
  separate multi-language protocol conformance evidence.
