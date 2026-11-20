# Actual released-v0.2 active-data profile, 2026-10-27

Reproduce from the repository root, with the pinned toolchain and Docker reachable:

```sh
CARGO_TARGET_DIR=<owned-absolute-target-outside-repository> \
  python3 scripts/test-released-v02-profile.py
```

The runner creates and verifies an immutable v0.2.0 archive, overlays only test fixtures,
uses distinct old/current build targets, copies both executables and owns its disposable
PostgreSQL through Testcontainers. No external database is accepted by the coordinator.
The input manifest includes tracked and untracked source files before/after compilation;
its parent HEAD plus the recorded source digest identifies the actual dirty test input.
Evidence/documentation added afterwards are not claimed as compiled inputs.

Final runner: exit0,325.75s; isolated old build0.59s, clean no-debug current build313.03s,
upgrade command9.66s. Coordinator1passed/0failed/0ignored in8.42s; its old public producer
1passed in3.86s and final actual released reader1passed in0.30s are nested phases, not an
independent aggregate. Both old runs finish and replay, with immutable old prefixes retained.
The schema moves normally from20261220000000 to20261223000000.

The preflight files retain an earlier functional pass before source-boundary hardening:
current build137.56s, upgrade command20.84s/coordinator18.66s, outer runner161.52s.
They are separate invocations. The ledger records the initial UUID serialization compile
failure and the invalidated shared-cache attempt; neither is acceptance or a behavior RED.
The final old executable comes from the separate marked release target.

This proves actual released-source runtime behavior, not a redistributed release asset.
It covers an old READY/unleased activity and a legacy root event wait. It does not prove
old in-flight worker reports, new scoped waits in old readers, a mixed-dispatcher cutover,
public scoped waits, five-SDK parity, nesting/capacity or complete release readiness.
The operator must retain compatible engines/readers and preserve required migration history.
No tokens, credentials, task capabilities or payload bytes are stored in this evidence.

Transcript files strip trailing line whitespace and final blank lines for Git hygiene;
transcripts.json retains both original-byte and normalized checksums. Test content is unchanged.
