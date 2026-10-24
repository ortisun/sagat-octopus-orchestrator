# Validation guide: per-field payload classification

Implementation is in progress. The focused commands below have actual evidence on the integration
branch; the manual stack walkthrough and complete release gates remain open. A story is validated
only by its full named suite passing against the required infrastructure.

## Integration checkpoint — 2026-10-17

- `cargo test -p sagat-tests --test contract_payload_classification --test integration_payload_classification --test fault_payload_classification`: 15 contract, 21 real PostgreSQL/HTTP and 2 deferred-audit fault tests passed (`/tmp/sagat-completion-classification-combined-gates-3.log`).
- `SAGAT_STUDIO_NO_SANDBOX=1 SAGAT_STUDIO_REBUILD=0 SAGAT_STUDIO_GREP='viewer, developer, operator and revealer' cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture`: one real Chromium disclosure journey passed (2.5 s browser, 7.66 s harness; `/tmp/sagat-completion-disclosure-browser.log`). The unrelated sandbox journey was deliberately excluded and is not proven by this command.
- Python SDK: 261 tests, mypy's 38 files and Ruff passed; this is not all-five runtime conformance.

The commands used the shared target path recorded in the integration notes, three build jobs,
`CARGO_INCREMENTAL=0` and `RUST_MIN_STACK=16777216`. The Studio build was produced by
`npm run build --workspace @sagat/studio` before the browser run. Do not interpret this checkpoint
as proof that every story command or upgrade/crash scenario below has been executed.

## Executed walkthrough and focused gates — 2026-10-17

The checked-in walkthrough is exercised by:

```sh
cargo test --locked -p sagat-tests --test e2e_payload_classification quickstart:: -- --test-threads=1
```

The actual `sagat` and `curl` processes publish and run the examples against real HTTP, gRPC and
PostgreSQL, using the production engine entry point. The worker's answer is deliberately canned;
this is no claim about external payment effects. Input, result and journal selectors, role-based
reads, grant/revoke, version pinning and the event/dead-letter read recipes passed in 21.00 seconds.
The retained letter is a fixture in the isolated database, not a live Kafka ingestion claim. All
commands below use synthetic data. Binary compatibility, the Chromium journey, fault and performance
gates have separate evidence in the integration notes and compatibility report; the full workspace
release gate remains open.

The specification prerequisite command below passed; targeted `PayloadDisclosure`/`WorkflowSettings`
component tests passed 8/8. The full E2E classification target passed 5/5 before adding this walkthrough;
the stronger CLI key matrix separately passed 96 process calls. Do not add these aggregate counts as
if they were distinct workflow runs.

## Specification checks

From the repository root:

```sh
SPECIFY_FEATURE_DIRECTORY=specs/016-payload-classification SPECIFY_FEATURE_NO_PERSIST=1 \
  .specify/scripts/bash/check-prerequisites.sh --json --require-spec --require-tasks --include-tasks
git diff --check
# the relative-link check of the Markdown set is the snippet recorded in analysis.md
```

The first command lists the documents the feature has. The second finds whitespace errors. The comment line stands for
the relative-link check recorded in [analysis.md](./analysis.md). The checks prove the set is well formed, not
that the design works.

## Prerequisites for the story checks

A running stack as in `docs/quickstarts/README.md` (compose with PostgreSQL, the control plane, the
engine and the development identity provider), a tenant with four members whose roles are `viewer`,
`developer`, `operator` and `admin` (on this branch only the first administrator can be created by
`provision-tenant` or `seed-dev`; the integration suites insert the others through
`tests/support`, and a manual run inserts rows into `memberships` as the database owner, in a
development database only; see G290; use subject `developer-canary` for the grant commands below), a namespace `payments`, `SAGAT_SERVER` and `SAGAT_NAMESPACE=payments` exported as in that guide, and a token for each member
(`sagat dev token --subject <SUBJECT>`). The
`worker dev` helper of feature 012 serves the activities of the test workflow with canned answers. Use
the canary strings of `tests/contract/payload-classification/fixtures/canary.json`; never real data.

The reference workflow, `orders-classified`, has this `inputSchema` (the document of
[contracts/README.md](./contracts/README.md) section 1.3), an `outputSchema` with a `public` `status`,
an `internal` `ledgerRef` and a `pii` `receiptEmail`, and a `charge` activity whose `outputSchema`
marks `authCode` as `secret` and `amount` as `internal`.

## US01 - classify at authoring time, pinned

Use a development namespace where `orders-classified` has not been published; the version-1/2
recipe below assumes its first publication.

```sh
EXAMPLES=specs/016-payload-classification/examples
sagat workflow validate "$EXAMPLES/orders-classified.json"
sagat workflow validate "$EXAMPLES/classification-loosened.json" # expected nonzero: CLASSIFICATION_LOOSENED
sagat workflow publish "$EXAMPLES/orders-classified.json"       # version 1, rules 1
# In a second terminal with the same server, gRPC address, namespace and admin token:
sagat worker dev --mocks "$EXAMPLES/mocks.json" --workflow "$EXAMPLES/orders-classified.json" --bind --max-tasks 1 --idle-exit 20
# Back in the first terminal; copy the printed run UUID into RUN:
sagat workflow run orders-classified --input "$(cat "$EXAMPLES/input.json")" --wait
sagat workflow publish "$EXAMPLES/orders-classified-relaxed.json" # version 2 makes customer public
curl -s "$SAGAT_SERVER/runs/$RUN" -H "Authorization: Bearer $SAGAT_TOKEN"
# Copy workflowId from that response into WORKFLOW:
curl -s -X PUT "$SAGAT_SERVER/workflows/$WORKFLOW/active-version" \
  -H "Authorization: Bearer $SAGAT_TOKEN" -H 'Content-Type: application/json' \
  -d '{"version":2,"reason":"classification demo activation"}'
```

The checked-in files include complete workflows, synthetic input and canned worker outputs. The
result and activity carriers both mark authCode secret; classification is interpreted at the receiving
carrier and does not track data flow. Run version 1 before publishing version 2, make version 2 active,
and disclose the old run: customer is still pii. Suites:
`cargo test --locked -p sagat-tests --test contract_payload_classification`,
`cargo test --locked -p sagat-tests --test integration_payload_classification pinning`. Pass when each defect class of
`contracts/README.md` section 1.4 is refused with its code and pointer and a legacy version discloses
nothing.

## US02 - public and internal values by role

```sh
curl -s -X POST "$SAGAT_SERVER/runs/$RUN/payload-disclosures" \
  -H "Authorization: Bearer $VIEWER" -H 'Content-Type: application/json' \
  -d '{"source":"input"}'
```

As the viewer, only `orderId` (public) is in `tree`. Repeat with `$DEVELOPER`: `amount` (internal) is
added and `customer` is absent. With `$OPERATOR`: `customer` appears as a `withheld` node of level
`pii`. Its children remain hidden until an explicit PII reveal; then cardToken is still withheld as secret. `sagat workflow inspect $RUN --values` as
the developer prints the same values with control characters neutralised. `source: "result"` is available through the integrated G21 result ledger. Suites: `cargo test -p sagat-tests --test contract_payload_classification -- rest`,
`cargo test --locked -p sagat-tests --test integration_payload_classification disclosure`, `cargo test --locked -p sagat-tests --test e2e_payload_classification cli`.
Pass when the audience matrix table test has no failing cell.

## US03 - reveal personal data

```sh
sagat namespace reveal-grants grant developer-canary --namespace "$SAGAT_NAMESPACE"      # as admin
curl -s -X POST "$SAGAT_SERVER/runs/$RUN/payload-disclosures" -H "Authorization: Bearer $DEVELOPER" \
  -H 'Content-Type: application/json' -d '{"source":"input","reveal":["pii"]}'
sagat namespace reveal-grants revoke developer-canary --namespace "$SAGAT_NAMESPACE"
```

Before the grant the call is `403` with `details.reason = PII_REVEAL_NOT_GRANTED`. After it the
`customer` values are present and `cardToken` is not; read the audit trail (`GET $SAGAT_SERVER/audit`) and find
exactly one `payload.pii_revealed` for the call, with no value, key or digest in it. After the
revocation the call is `403` again. Suites: `cargo test --locked -p sagat-tests --test integration_payload_classification grants::`,
`cargo test --locked -p sagat-tests --test fault_payload_classification`. Pass when an injected audit failure returns no
value and two reveals leave two records.

## US04 - nothing forbidden crosses

Run the canary suites: `cargo test --locked -p sagat-tests --test integration_payload_classification canary` and
`cargo test --locked -p sagat-tests --test e2e_payload_classification canary`, then the browser journey
`SAGAT_STUDIO_NO_SANDBOX=1 SAGAT_STUDIO_REBUILD=0 SAGAT_STUDIO_GREP='viewer, developer, operator and revealer' cargo test --locked -p sagat-tests --test e2e_workflow_studio -- --ignored journeys --nocapture`.
This uses the existing Studio build and deliberately excludes the unrelated sandbox journey. Pass when no canary of a forbidden class
is found in any captured response, error, log, span, metric label, audit record, stream frame, CLI
output, DOM node, storage entry or network payload, for every role, with and without a grant, including
a payload that violates its schema and a version published before the feature.

## US05 - step outputs and the SDK builders

Disclose the `step.completed` sequence of `charge` (`{"source":"journal","sequence":N}`) as each role:
`amount` is `internal`, `authCode` never appears. Suites: `cargo test --locked -p sagat-tests --test integration_payload_classification step_outputs`; `SAGAT_SDK_SCENARIO=classification cargo test -p sagat-tests --test sdk_conformance -- --ignored`
(needs Docker; `SAGAT_SDK_LANGS` selects languages, and a subset cannot close parity) and the
builder tests of each SDK as listed in `docs/runbooks/sdks.md`. Pass when the five canonical documents are
byte-identical.

## US06 - the Studio

Open the run in the Studio as a developer and as a granted developer. Check: levels are marked by text,
the reveal action appears only for the granted user and asks for confirmation, navigating away clears
the values, and the browser storage and network log hold no value the server withheld. Suites:
`npm run test --prefix frontend --workspace @sagat/studio -- PayloadDisclosure` and the Chromium journey above. Accessibility is
checked with tools and a keyboard walkthrough, not claimed from assistive-technology use.

## US07 - the report

`sagat workflow validate` on a `public` root with unannotated children prints one
`CLASSIFICATION_INHERITED_OPEN` warning per child; the Studio settings panel shows the same list; a
version diff shows `CLASSIFICATION_RELAXED`. Suites: `cargo test --locked -p sagat-tests --test contract_payload_classification validate`,
`npm run test --prefix frontend --workspace @sagat/studio -- WorkflowSettings`.

## US08 - event values need the reveal grant

Publish an event that carries a canary (`POST $SAGAT_SERVER/events` as in `docs/runbooks/events.md`) and set
aside a dead letter on a connector (the integration suite inserts one; on a stack, send a message with no
event type to a connector). Then, with `NS` the namespace id:

```sh
curl -s "$SAGAT_SERVER/namespaces/$NS/events/api/evt-canary-1" -H "Authorization: Bearer $VIEWER"
curl -s "$SAGAT_SERVER/namespaces/$NS/events/api/evt-canary-1?reveal=pii" -H "Authorization: Bearer $DEVELOPER"
curl -s "$SAGAT_SERVER/namespaces/$NS/connectors/$CONNECTOR/dead-letters" -H "Authorization: Bearer $VIEWER"
curl -s "$SAGAT_SERVER/namespaces/$NS/connectors/$CONNECTOR/dead-letters/$LETTER?reveal=pii" -H "Authorization: Bearer $DEVELOPER"
```

The first answer has the status, attempts and deliveries, `dataDisclosure: "structure-only"`, `dataSizeBytes`
and `dataStructure`, and no `data` and no canary. The second is `403` with `details.reason =
PII_REVEAL_NOT_GRANTED` until an administrator grants the developer (`sagat namespace reveal-grants grant`),
then returns `data` and leaves one `payload.pii_revealed` record for the event. The list carries no `body`
or `headers`; the single letter with `reveal=pii` and the grant carries both. A client that read `data` as
`event:read` before now gets the structure answer: this is the documented break (G292). Suites:
`cargo test -p sagat-tests --test contract_rest_events_webhooks_messaging`,
`cargo test --locked -p sagat-tests --test integration_payload_classification events`, `cargo test --locked -p sagat-tests --test e2e_events_webhooks_messaging`. Pass when no
canary reaches a caller without the grant and the audit count matches the reveals.

## Performance and failure

`SAGAT_PERF_REPORT=tests/performance/reports/payload-classification.md cargo test -p sagat-tests --test perf_payload_classification -- --ignored` writes
`tests/performance/reports/payload-classification.md`; read its environment section before its numbers.
`cargo test -p sagat-tests --test fault_payload_classification` injects audit failures and transaction
races. The separate real-process crash gate requires an absolute coordinated `CARGO_TARGET_DIR`:
`scripts/test-payload-classification-process-faults.sh` builds the actual control plane and explicitly
runs the ignored process case, retaining its logs and binary manifest. Service reconstruction alone
is not process-crash evidence. A failed infrastructure prerequisite must be reported as a failure.

## Unchanged response compatibility

With the absolute coordinated `CARGO_TARGET_DIR` exported, run
`scripts/test-payload-classification-surfaces.sh`. It builds the immutable integrated server from
before feature 016 and the current server, migrates the same populated fixture and compares the
actual body bytes of all ten FR-016-016 surfaces, including declared Query/Update and a completed
SSE stream. It retains a binary/source manifest and before/after bodies; it never blesses or
normalizes a response. The complete script passed on 2026-10-17; exact artifact and command evidence
is in `tests/integration/payload-classification/compatibility-report.md`.

## What this guide does not prove

It does not prove field classification of events (their data is `pii` as a whole, G282, G291), erasure, retention, taint tracking, or the
production values of the bounds. The numbers of the benchmark are measurements on one environment, not
capacity claims.
