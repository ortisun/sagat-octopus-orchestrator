# Collection finalization and scoped successful-output refusal

Production base: `05c53badf8edd081b4f94ac93f0cebba2fa866c2`, with the coupled working
changes pinned in the manifests here. These are focused integration results, not a release gate.
The PostgreSQL, scheduler and worker increments must ship together after remaining acceptance.

## Execution

The real owned PostgreSQL 16.15 server was `sagat-project-completion-scoped-schema`,
loopback port 65117. Each fixture created its own database. Commands, UTC starts, wall
times, exit codes and complete harness output are retained in matching JSON/TXT files.

```sh
export CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216
export SAGAT_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:65117/postgres
cargo test --locked -p sagat-tests --test integration_workflow_primitives collection_end_result -- --nocapture --test-threads=1
```

Repeat the recorded selections with `SAGAT_TEST_AS_RUNTIME_ROLE=1` for the runtime pool.
Public Flow producers use `sagat_app` even in the default invocation. Native collection
data-limit tests explicitly use a runtime pool in both invocations; the default pass does
not establish a second database-owner native writer path.

## Results

| Selection | Default result | Runtime result |
| --- | --- | --- |
| End semantics and genuine instance ceiling | 3 passed | 3 passed |
| Empty collection quota | 3 passed | 3 passed |
| Byte ceilings and successful-effect refusal | 21 passed | 21 passed |
| Native collection data-limit guards | 4 passed | not separately repeated here |
| Drain restoration | not selected | 4 passed |
| Existing collection compensation | not selected | 19 passed |
| Existing gRPC results | 5 passed | 5 passed |
| Existing classified Run result | 1 passed | 1 passed |
| Native source/scoped Finish regression | not selected | 58 passed, 2 ignored |

The two ignored native entries are private subprocess probes; the enclosing process tests
passed. They are not counted as passing tests. Final strict Clippy, workspace formatting
and the architecture gate passed. The architecture gate inspected 24 crates without cycles.
Separate selections, roles and revisions are not combined into a single suite count.

The End matrix covers omitted output, explicit null, expression failure and an oversized
array result. The empty collection cases include a genuine second root collection after
the first spends the last two bytes. The two successful-effect refusal cases retain
successful forward/Applied evidence, original ACK replay and zero report-created outbox
rows; ordinary discovery then drains the stopped collection and executes 17 distinct
compensations before the terminal compensated/failed outcome. The final source regression
preserves existing native source bindings and scoped Finish refusals.

## Revisions and failed attempts

The current `source-before.json` and `source-after.json` contain 1,119 native inputs and
match. Prior source pairs are named separately and are not used as evidence for a later
revision. Source hashes cover the new worker module as well as tracked inputs.

Initial Clippy attempts found two local argument-count lints and one `map_or` simplification;
the subsequent passes fixed those without changing behavior. The first empty-quota attempt
failed only its obsolete audit reason oracle: the typed contract uses `aggregate`. The first
byte-ceiling attempt passed 19 tests and failed two cross-iteration compensation ordering
oracles. Collection iterations are independent branches, so the corrected assertions compare
all 17 distinct compensations without requiring a cross-iteration order. Both complete
selections then passed in default and runtime modes. Original failed records are retained.

## Remaining acceptance

This evidence does not close Q9/Q10 or canonical feature 014 tasks. The normal scheduler
candidate query still needs acceptance for a genuine old-produced run with E above the
ceiling and a full-concurrency RUNNING member. Child-to-parent scalar propagation,
collection-specific classified disclosure, late Finish fault/metadata-only replay, report
fault/secret races, cold recovery/uncertain COMMIT and complete upgrade/release gates remain
separate work. No general nesting, production source/scoped Finish caller or release INSERT
grant is enabled. No mixed-writer compatibility or exactly-once external effects are claimed.
