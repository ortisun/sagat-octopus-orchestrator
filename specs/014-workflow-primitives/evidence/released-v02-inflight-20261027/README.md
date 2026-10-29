# Genuine released-worker in-flight report, 2026-10-27

Reproduce with the pinned toolchain, Docker reachable and an owned absolute target outside
the repository:

```sh
CARGO_TARGET_DIR=<owned-private-target> \
  python3 scripts/test-released-v02-profile.py --scenario inflight-worker
```

Final runner passes10.47s; old/current cached builds0.88s/0.49s; coordinator1/0/0 passes6.25s
with its actual released-client phase1/0/0 in2.67s. These nested phases are not an aggregate.
The old capability stays in process memory, crosses ordinary schema upgrade and is reported
and replayed by the old generated client to current runtime-role services. Final output,
physical attempt/fence and reported token commitment, one exact fingerprinted receipt and
one logical-byte charge are asserted. Two actual current scheduler ticks finish the run.
The exact source/executable/coordinator/overlay hashes are in build.json/current-source.json.

Retained separate invocations:

- Initial receipt assertion failed0pass/1fail in5.99s (121.27s runner). The old client itself
  passed2.41s. The test incorrectly compared persisted status|runStatus|version to ACK
  run:version; correcting the fixture required no production change.
- Corrected preflight passed1/0/0 in5.12s (61.14s runner; old phase1.99s). Its isolated
  coordinatorSha256 field still selected the active-data coordinator, so it is not final
  provenance acceptance; the complete input manifest still identified the source.
- Strengthened self-source assertion exposed that actual runner bug before starting PG:
  0pass/1fail in0.00s (82.91s runner). Scenario-selected coordinator mapping then fixes
  the genuine provenance RED. No platform behavior was changed.
- Final active-data regression passes1/0/0 in15.50s (19.74s runner), with nested released
  producer/reader phases. It remains separate from the in-flight result.
- Strict integration Clippy passes18.62s. Eight Python verifier tests pass0.074s.

Three bounded reviews found and resolved lease-margin/transport/replay/external-effect claim
issues, then coordinator/schema/finality/capability-receipt proof gaps; the last found none.
Reviewers made no edits and ran no builds/tests. Real root-run results are preserved here.

Transcript whitespace alone is normalized; transcripts.json records raw and normalized hashes.
No authentication/task/session tokens or payload bytes are retained in the evidence.
This is actual released-source behavior, not a redistributed SDK asset, expired-task proof,
mixed-dispatcher stop/join/credential fence or public scoped-wait activation. All-five SDK,
Applied/reconciliation frontier, nesting/capacity and full release/resource gates stay open.
