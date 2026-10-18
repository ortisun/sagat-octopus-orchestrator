# sdks

Five official SDKs (TypeScript, Go, Python, C#, Java) with one semantic contract: they pass the same conformance suite in `tests/contract/sdk-conformance/` and never fork the server's behaviour (rule 8, ADR-0007, ADR-0015).

- A new client operation or builder is added to all five, with a scenario and a reviewed golden. Builders emit the same canonical document in every language.
- Publishing a workflow is not an SDK operation: the platform's own validation is proved in the Rust contract suites.
- Checks CI runs per SDK; run them all, they are sequential in CI:
  - TypeScript: `cd sdks/typescript && npm ci --ignore-scripts && npm test`
  - Python: `cd sdks/python && python3 -m venv .venv && .venv/bin/pip install --require-hashes -r requirements-dev.lock.txt && .venv/bin/python -m pytest && .venv/bin/mypy && .venv/bin/ruff check .`
  - Go: `cd sdks/go && go test -race ./... && go vet ./... && test -z "$(gofmt -l .)"`
  - C#: `cd sdks/dotnet && dotnet test && dotnet format --verify-no-changes`
  - Java: `cd sdks/java && ./mvnw -q verify`
- Conformance, one scenario: `SAGAT_SDK_SCENARIO=<name> cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`; all: omit the variable. The workers an SDK starts offer the pinned protocol minor in the scenario, not the newest.
- Delete `node_modules`, `.venv`, `bin/obj` and build output when you finish; they are large.
