# sdks

Six official SDKs (TypeScript, Go, Python, C#, Java, Rust) with one semantic contract: they pass the same conformance suite in `tests/contract/sdk-conformance/` and never fork the server's behaviour (rule 8, ADR-0007, ADR-0015).

- A new client operation or builder is added to all six, with a scenario and a reviewed golden. Builders emit the same canonical document in every language.
- Publishing a graph workflow is not an SDK operation: the platform's own validation is proved in the Rust contract suites.
- Publishing a code version is (`definitions.publishCode(manifest, envelope)`, design 044, `SPEC.md` section 12): only the SDK computes the code digest, so it is added to all six with a scenario like any client operation.
- Checks CI runs per SDK; run them all, they are sequential in CI:
  - TypeScript: `cd sdks/typescript && npm ci --ignore-scripts && npm test` (the workflow tests run under `TZ=UTC`, Node 22.8 or later)
  - TypeScript workflows bundler (design 044): build the SDK first (`npm run build` in `sdks/typescript`), then `cd sdks/typescript-workflows-bundler && npm ci --ignore-scripts && npm audit signatures && npm test`
  - Python: `cd sdks/python && python3 -m venv .venv && .venv/bin/pip install --require-hashes -r requirements-dev.lock.txt && .venv/bin/python -m pytest && .venv/bin/mypy && .venv/bin/ruff check .`
  - Go: `cd sdks/go && go test -race ./... && go vet ./... && test -z "$(gofmt -l .)"`
  - C#: `cd sdks/dotnet && dotnet test && dotnet format --verify-no-changes`
  - Java: `cd sdks/java && ./mvnw -q verify`
  - Rust: `cd sdks/rust && cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked` (owned external `CARGO_TARGET_DIR`; the public SDK is an independent workspace).
- Conformance, one scenario: `SAGAT_SDK_SCENARIO=<name> cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`; all: omit the variable. The workers an SDK starts offer the pinned protocol minor in the scenario, not the newest.
- Delete `node_modules`, `.venv`, `bin/obj` and build output when you finish; they are large.
