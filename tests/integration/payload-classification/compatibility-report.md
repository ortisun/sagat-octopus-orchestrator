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
3. The v0.1.0 HTTP process publishes a document containing an inert classification extension whose
   value (`pre-016-note`) is invalid under the new authoring rules,
   starts an in-flight run, publishes an event, and reads event and connector dead-letter values.
4. The test records the exact published document bytes, digest and run version pin.
5. The current migrator upgrades the same database. The retained version becomes rules `0` without
   changing its bytes or digest, and the run keeps its original version pin.
6. The v0.1.0 execution engine and control plane start again with migration-on-start disabled and
   read the upgraded database.
7. The current control plane serves the old Studio assets, returns `LEGACY_VERSION` without the
   retained input value, and returns structure-only event and dead-letter answers without data,
   body or headers. The current process pauses and resumes the retained run, exercising cold
   executable restoration through a real mutation; the persisted pause projection is checked.
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

Result: **the named feature-016 prior-release cases passed**. The annotated `v0.1.0` tag resolves
to `6ede1c7849c4ac5a209c08da8ad25294fbdd7795`. The full artifact-building process gate passed 1/1
in 49.88 s (`/tmp/sagat-completion-compat-release-5.log`). Its manifest is retained at
`/Users/marcelo/Documents/Projects/sagat-octopus-compat016-artifacts/run.PJb73x/artifacts/SHA256SUMS`;
the current binary is from `2108d08cd117f26847b605c92c8abeb95a3dc5ea`.

The initial harness incorrectly pinned the tag's parent and refused before building. Other
prerequisites needed bytewise SHA-256 hex encoding, explicit HTTP crypto-provider initialization
and the event deployment gate's shared random test key ring. The first populated migration then
found PostgreSQL 55006 in the new unreleased 20261205000000 discovery migration. A focused test
reproduced it, then passed after indexes were created before the backfill, retaining the same
transaction and normal deferred quota triggers. All 60 released migration files remain unchanged.
Those failed attempts are not counted as passing evidence.

The expanded process case executed the archived release Studio in real Chromium against the
current binary and passed 1/1 in 17.48 s (`/tmp/sagat-completion-compat-browser-2.log`). It signs in,
reads the existing run and graph and renders the work row in the node table, with no browser errors
or retained-payload canaries in the DOM or inspected HTTP responses. The exact archived bundle is
served; this is browser behavior proof beyond the earlier nonempty-index check. The first driver
compile required converting its UUID to text and did not execute a browser.

All five released SDK sources were archived with their own canonical contracts from the same
immutable release. The current conformance harness selected them through `SAGAT_SDK_SOURCE_ROOT`,
built their own prepare/run scripts, and ran the reviewed current `run-lifecycle` scenario against
the real current server. It passed one aggregate / five languages in 145.91 s, without blessing
(`/tmp/sagat-completion-old-sdks.log`). Input provenance: 361 source/contract files in
`/Users/marcelo/Documents/Projects/sagat-octopus-compat016-artifacts/run.1mfG99/artifacts/sdk-source.sha256`.
Reproduce from a fresh archive with `scripts/test-released-sdk-compat.sh`; its shell syntax and
ShellCheck passed. The recorded Cargo invocation used the equivalent archived source directly.

## Limits

- Artifacts are built from the actual tagged source with its lock files; reconstructed current
  services are not used as a substitute for old binaries.
- The old execution engine reaches readiness on both schemas and, after upgrade, restores the
  actual retained run for a deliberately unleased heartbeat. Its typed STALE_ATTEMPT refusal
  proves that read path without an old-server state write. Old HTTP processes also read the old
  run and workflow after upgrade. This does not prove that an old server can write arbitrary new scoped
  workflows; feature 014's broader upgrade and rollback gates remain separate.
- The old Studio browser case covers sign-in, run/graph and table rendering, not every editor action.
- Released SDK proof covers the shared root lifecycle scenario in all five languages. It is not the
  complete current primitive/runtime conformance matrix, which remains T014-041.
- T016-052's named compatibility cases are complete. Final integrated architecture, security,
  workspace and quickstart acceptance remain separate open release tasks.


The final probe-enhanced process case passed 1/1 in 24.96 s
(`/tmp/sagat-completion-compat-engine-read-2.log`). It exercises the released engine's actual
run-restoration path after the upgrade and receives `Aborted` / `x-sagat-error: STALE_ATTEMPT`
for the unleased heartbeat. The first probe used a definition node as `step_id`, whereas that
legacy field is an optional UUID; its InvalidArgument result was a harness failure. Omitting
that optional field uses the documented legacy heartbeat form. An earlier final case also
explicitly proved the old disclosure endpoint is 404 and old run readers carry no payload values
(`/tmp/sagat-completion-compat-final.log`, 1/1 in 21.04 s).


### Actual pre-classification byte compatibility

`scripts/test-payload-classification-surfaces.sh` passed its complete build/migrate/HTTP gate
(1 test, 5.96 s runtime; `/tmp/sagat-completion-surface-compat-full-3.log`). The immutable baseline
is 3637ee8c4b98d2e5809f1527a9b2bb1fcaf2406a: the approved integrated 014/017 platform immediately
before feature 016, rather than a release without Query/Update. The actual baseline publishes and
starts terminal and stateful runs and applies a declared Update. Its responses are retained,
then the actual current server migrates the populated database and repeats the exact requests.
No response is normalized and no golden is blessed. Actual bodies are byte-identical for all ten
surfaces: run, journal, graph, graph changes, completed SSE stream, telemetry, attempts, structural
inspection, declared Query, and a replayed declared Update. The latter two positively return the
canary under their explicit declared-result policy; classification does not filter those results.
Telemetry uses a legitimate caught-up persisted projection (lag zero), not a substituted clock.

Manifest/binaries/response pairs: `/Users/marcelo/Documents/Projects/sagat-octopus-surface016-artifacts/run.U6mTUY/artifacts/SHA256SUMS`.
The first complete script run reached the real old server but refused an unbounded-string test
fixture, as the existing contract requires. Adding maxLength=256 to that fixture corrected the
harness. A direct retry passed 1/1 in 5.68 s before the full script was rerun successfully.
The script keeps its detached source read-only and removes only that clean source on exit;
artifacts and logs remain. This extra gate complements the separate actual v0.1.0 release upgrade,
released-five-SDK and old Studio/browser evidence; it is not an arbitrary old-server writer claim.


## Stored extension regression on the combined branch

The frozen v0.1.0 schema accepts arbitrary extension values inside a JSON Schema. A cold cache
restore now distinguishes immutable executable validation from new-publication classification
validation. A unit regression first failed on ClassificationInvalidLevel; the exact frozen schema
has SHA256 5035e781a2a531da0317594cd8b082fb9291d8e3f0a7b0322625c2260d89bff4,
and its digest is asserted by the test. New publication still rejects the invalid annotation.

The strengthened actual-process test was then run against both preserved current binaries, reusing
SHA256-verified old release binaries and every archived Studio asset with a newly detached, clean,
read-only source at the exact release commit. The old artifacts were not rebuilt for this repeat:

- Pre-fix current binary 2108d08cd117f26847b605c92c8abeb95a3dc5ea: **failed**, 21.30 s
  (`/tmp/sagat-completion-legacy-restore-process-red.log`). The old Studio could not render the run:
  current GET graph returned 500 after the executable parser rejected the older inert extension.
  This failed before the later pause/resume checks.
- Fixed current binary b05e3d59: **passed 1/1**, 6.23 s, no ignored
  (`/tmp/sagat-completion-legacy-restore-process-green.log`). This includes old processes before/after
  migration, actual old Studio in Chromium, current-process pause/resume and persisted state checks,
  immutable bytes/digest/version pin, legacy opacity and a strict new rules-1 publication.

Fresh manifest and separate stage logs:
`/Users/marcelo/Documents/Projects/sagat-octopus-compat016-artifacts/run.restore-20261017-b05e3d59/artifacts/SHA256SUMS`.
The pre-fix attempt has its own manifest and logs; the successful manifest was not overwritten.
This gate's current binary predates the independent compensation-routing fix; it proves the
immutable-definition restoration boundary and is not claimed as a complete feature-014 runtime gate.
