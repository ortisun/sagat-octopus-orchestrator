# Validation quickstart: TypeScript, Go, Python, C# and Java SDKs

Every command below was run on macOS 26 (arm64, Darwin 25.3) against this repository. Results are in the last
section; what was **not** run is stated there too.

## Prerequisites

- The Rust toolchain pinned by the repository, and Docker (the tests start their own PostgreSQL; the real execution
  engine and the REST and gRPC APIs run in the test process — no compose file is needed).
- To run an SDK's own unit tests or its conformance runner: Node (run: 26.8.1), Python (3.10–3.14), Go ≥ 1.25
  (run: 1.25.14), .NET SDK (run: 10.0.401; the library also targets `net8.0`) and a JDK ≥ 17 (run: 23.0.2, with the
  repository's Maven wrapper). Each SDK's README says what it needs and which versions were exercised.
- The conformance, user-story and fault targets are `#[ignore]` in the workspace run because they need all of that; they are run with `-- --ignored`, and by the `sdks` job of CI (`.github/workflows/ci.yml`, whose commands were run locally; **the job itself has not run on GitHub yet**).
- No dev token, tenant or fixture is prepared by hand: every scenario creates its own tenant, namespace, identities
  and worker bindings through the public API.

## Validation flow

1. **The five SDKs' own tests** (offline, against fakes of the REST and gRPC servers):

   ```text
   cd sdks/typescript && npm ci --ignore-scripts && npm test
   cd sdks/python     && python3 -m venv .venv && .venv/bin/pip install --require-hashes -r requirements-dev.lock.txt \
                         && .venv/bin/python -m pytest && .venv/bin/mypy && .venv/bin/ruff check .
   cd sdks/go         && go test -race ./... && go vet ./... && gofmt -l .
   cd sdks/dotnet     && dotnet test && dotnet format --verify-no-changes
   cd sdks/java       && ./mvnw -q verify
   ```

2. **The protocol contract of the server** (the negotiation of the worker protocol's minor version, including the 1.0
   heartbeat):

   ```text
   cargo test --offline -p sagat-tests --test contract_grpc_006
   ```

3. **The conformance suite** — the contract of the five SDKs (15 scenarios, each played by each language through its
   runner against a platform and an engine of its own, faults injected at the SDK's transport hooks; the results and
   the state the platform ends in must equal each other and the reviewed golden files):

   ```text
   cargo test -p sagat-tests --test sdk_conformance -- --ignored --nocapture
   SAGAT_SDK_LANGS=go cargo test -p sagat-tests --test sdk_conformance -- --ignored --nocapture     # one language
   SAGAT_SDK_BLESS=1 …                                                                       # rewrites the goldens: read every diff
   ```

   Contract: `tests/contract/sdk-conformance/README.md`.

4. **The user stories and the faults** (`spec.md` US01..US04; five languages each; the C# one through an ASP.NET Core
   service registered with `AddSagatOctopusOrchestrator`):

   ```text
   cargo test -p sagat-tests --test e2e_006 --test fault_006 -- --ignored
   ```

5. **Confirm** the outcomes, the journal and the traces: every scenario reads the state of the platform over REST,
   without the SDK, and compares it; the harness fails a scenario when a token, a secret or a marked payload appears
   in any SDK output, error or log.

6. **Take a worker down at a controlled moment**: the scenarios `lease-lost`, `worker-shutdown` and
   `worker-transport-faults` stop, restart and cut off workers mid-task and check that the step is retried once and
   never lost; `fault_006` repeats it with every language.

7. **The benchmark** (release mode, a quiet machine, writes the report):

   ```text
   SAGAT_PERF_REPORT=tests/performance/reports/006-sdk-multilanguage.md \
     cargo test -p sagat-tests --test perf_006 --release -- --ignored --nocapture
   ```

## Gates

- Same golden flows and failure cases executed in TS, Go, Python, C# and Java with normalized results.
- Reproducible, annotated errors; no confirmed transition lost.
- Update tasks `[x]` only when real tests have passed.

## Evidence

Run on 2026-10-10 on the final code of the branch (all five SDKs, macOS arm64, Docker Desktop):

| Check | Result |
|---|---|
| Conformance suite, 15 scenarios × 5 languages (`sdk_conformance`) | passed three times in a row on the final code: 229.1 s (every runner rebuilt), 222.1 s, 222.3 s; the results equal the goldens and each other. The `secrets` scenario was added last (a finding of this closing: no scenario resolved a secret through the real server); the other 14 passed three times before it, in 207.6 s, 207.6 s and 208.8 s. A fourth pass, with the exact command of the `sdks` CI job, took 231.0 s |
| `e2e_006` (US01..US04, with the ASP.NET Core service) | 6 passed (120.1 s, with the command the CI job runs, `-- --ignored`; 108.7 s before the 15th scenario) |
| `fault_006` (network, workers, tenant) | 9 passed (162.4 s, same command; 151.5 s before) |
| `contract_grpc_006` (negotiation) | 7 passed |
| TypeScript `npm test` (Node 26.8.1) | 96 passed |
| Python `pytest` | 226 passed on 3.10.21, 3.11.15, 3.12.12, 3.13.9 and 3.14.7 (the pinned, hashed dev lock); `mypy --strict` (34 files) and `ruff check` passed on 3.14.7 |
| Go `go test -race ./...`, `go vet`, `gofmt -l` | 165 top-level tests passed (0 failed) on 1.25.14; vet clean; gofmt prints nothing. `govulncheck` (run on 2026-10-08): 9 findings in the standard library and `x/net`, G135 |
| C# `dotnet test`, `dotnet format --verify-no-changes` | 410 passed on net10.0; format clean. The `net8.0` build of the SDK ran the conformance suite once, on an earlier revision |
| Workspace gate | `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `scripts/check-architecture.py`, `cargo audit --deny warnings` clean; `cargo test --workspace --locked`: 87 test binaries, 1 341 passed, 0 failed, 23 ignored (the doc-test step hung once, rustdoc at 0 % CPU, as clippy-driver did earlier, and passed when run again); OpenAPI lint valid (the same 9 warnings as `develop`), AsyncAPI valid, `worker.proto` compiles |
| Benchmark `perf_006` (release, `--ignored`), report `tests/performance/reports/006-sdk-multilanguage.md` | three complete runs of ~285 s; all hypotheses held in the last one after H1, H3 and H4 were restated (the report says how and why); finding G141 |
| Java `./mvnw -q verify` | 202 tests run, 0 failures, 0 errors on JDK 23.0.2 (the `SEVERE` lines in its output are logged by tests that provoke a failure on purpose) |

Reviews: two independent read-only security reviews and two correctness reviews, then a re-verification of each
after the fixes (four rounds); every finding was read against the code and fixed failing-first (`fix(sdk)` commits,
`research.md` G121–G141). The open ones are G134 (no local lease fencing), G141 (a heartbeat due as a step ends is answered `aborted`), G135 (Go advisories fixed only in a
toolchain younger than two weeks) and G136 (Java has no `tls.serverName`; unpinned `protoc` binaries).

Not run: Linux, Windows or another architecture; the SDKs against a TLS-terminating proxy or a real cloud; the
Python conformance suite on 3.10–3.13 with the final code (unit tests only); `net8.0` and Java 17/21 execution
beyond what each README states; publishing to any registry; streaming of tasks (not built).
