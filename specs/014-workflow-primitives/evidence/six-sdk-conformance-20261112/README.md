# Six-SDK conformance of the collection and loop scenarios (2026-11-12)

Every collection and loop scenario of `tests/contract/sdk-conformance/scenarios/` played in the six official SDKs, one scenario per run, at commit `c1d2e4ed` (`develop` `2a1bdf92` plus a test-only commit). Feature 014 recorded these scenarios in five SDKs; this adds the Rust SDK (feature 034) to the same runs.

Command, once per scenario:

```sh
SAGAT_SDK_SCENARIO=<scenario> SAGAT_RUST_SDK_TARGET_DIR=<owned external dir> SAGAT_TEST_DATABASE_URL=<local PostgreSQL 18.6> \
  cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
```

Toolchains: Node v26.13.0, go1.24.2, .NET 10.0.401, openjdk 23.0.2, Python 3.15.2 (`sdks/python/.venv` from `requirements-dev.lock.txt`), cargo 1.98.0. PostgreSQL 18.6, local, `fsync=off`; CI uses PostgreSQL 16.

| Scenario | Languages | Result | Wall time |
|---|---|---|---:|
| `collection-blob-output` | 6 | passed | 47 s |
| `collection-blob-source` | 6 | passed | 47 s |
| `collection-builders` | 6 | passed | 35 s |
| `collection-compensation` | 6 | passed | 67 s |
| `collection-control-entry` | 6 | passed | 28 s |
| `collection-control-routing` | 6 | passed | 41 s |
| `collection-events` | 6 | passed | 34 s |
| `collection-fail-fast-drain` | 6 | passed | 27 s |
| `collection-limits` | 6 | passed | 31 s |
| `collection-local-recovery` | 6 | passed | 35 s |
| `collection-ordering` | 6 | passed | 72 s |
| `collection-runtime` | 6 | passed | 26 s |
| `collection-step-source` | 6 | passed | 30 s |
| `collection-timers` | 6 | passed | 32 s |
| `loop-builders` | 6 | passed | 13 s |
| `loop-cancel` | 6 | passed | 33 s |
| `loop-compensation` | 6 | passed | 60 s |
| `loop-error-activity` | 6 | passed | 28 s |
| `loop-runtime` | 6 | passed | 26 s |
| `loop-semantics` | 6 | passed | 71 s |

All 20 passed in all six languages. Each run prints `--- <scenario> (6 languages)` and fails the test on any language's failure; none was retried. A first run of `collection-ordering` failed only in the Rust runner because `SAGAT_RUST_SDK_TARGET_DIR` was not set (an environment requirement of `sdks/rust/conformance/run.sh`), and passed once it was.
