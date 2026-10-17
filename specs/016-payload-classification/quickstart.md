# Validation guide: per-field payload classification

Nothing in this feature is built. This guide separates what can be run today (checks on the
specification itself) from the validation of each story, which becomes runnable as the tasks of
`tasks.md` close. A story is validated only by its named suite passing on a real stack; reading this
guide is not evidence.

## Runnable today: the specification

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
development database only; see G290), a namespace `dev`, `SAGAT_SERVER` and `SAGAT_NAMESPACE=dev` exported as in that guide, and a token for each member
(`sagat dev token --subject <SUBJECT>`). The
`worker dev` helper of feature 012 serves the activities of the test workflow with canned answers. Use
the canary strings of `tests/contract/payload-classification/fixtures/canary.json`; never real data.

The reference workflow, `orders-classified`, has this `inputSchema` (the document of
[contracts/README.md](./contracts/README.md) section 1.3), an `outputSchema` with a `public` `status`,
an `internal` `ledgerRef` and a `pii` `receiptEmail`, and a `charge` activity whose `outputSchema`
marks `authCode` as `secret` and `amount` as `internal`.

## US01 - classify at authoring time, pinned

```sh
sagat workflow validate orders-classified.json                  # report lists the effective level of each path
sagat workflow validate defects/loosened.json                   # CLASSIFICATION_LOOSENED at /properties/customer/properties/name
sagat workflow publish orders-classified.json                   # version 1, classification rules 1
sagat workflow publish orders-classified-relaxed.json           # version 2 makes a pii field public
```

Then run version 1, make version 2 active, and disclose the run of version 1: the relaxed field is
still `pii`. Suites: `cargo test -p sagat-tests --test contract_payload_classification -- ir validate`,
`--test integration_payload_classification -- pinning`. Pass when each defect class of
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
`pii` and `cardToken` as a `withheld` node of level `secret`. `sagat workflow inspect $RUN --values` as
the developer prints the same values with control characters neutralised. `source: "result"` needs the
G21 change. Suites: `cargo test -p sagat-tests --test contract_payload_classification -- rest`,
`--test integration_payload_classification -- disclosure`, `--test e2e_payload_classification -- cli`.
Pass when the audience matrix table test has no failing cell.

## US03 - reveal personal data

```sh
sagat namespace reveal-grants grant dev-developer --namespace dev      # as admin
curl -s -X POST "$SAGAT_SERVER/runs/$RUN/payload-disclosures" -H "Authorization: Bearer $DEVELOPER" \
  -H 'Content-Type: application/json' -d '{"source":"input","reveal":["pii"]}'
sagat namespace reveal-grants revoke dev-developer --namespace dev
```

Before the grant the call is `403` with `details.reason = PII_REVEAL_NOT_GRANTED`. After it the
`customer` values are present and `cardToken` is not; read the audit trail (`GET $SAGAT_SERVER/audit`) and find
exactly one `payload.pii_revealed` for the call, with no value, key or digest in it. After the
revocation the call is `403` again. Suites: `--test integration_payload_classification -- grants reveal`,
`--test fault_payload_classification -- reveal races`. Pass when an injected audit failure returns no
value and two reveals leave two records.

## US04 - nothing forbidden crosses

Run the canary suites: `--test integration_payload_classification -- canary` and
`--test e2e_payload_classification -- canary`, then the browser journey
`npm --prefix frontend run e2e -- payload-disclosure.spec.ts`. Pass when no canary of a forbidden class
is found in any captured response, error, log, span, metric label, audit record, stream frame, CLI
output, DOM node, storage entry or network payload, for every role, with and without a grant, including
a payload that violates its schema and a version published before the feature.

## US05 - step outputs and the SDK builders

Disclose the `step.completed` sequence of `charge` (`{"source":"journal","sequence":N}`) as each role:
`amount` is `internal`, `authCode` never appears. Suites: `--test integration_payload_classification --
step_outputs`; `SAGAT_SDK_SCENARIO=classification cargo test -p sagat-tests --test sdk_conformance -- --ignored`
(needs Docker; `SAGAT_SDK_LANGS` selects languages, and a subset cannot close parity) and the
builder tests of each SDK as listed in `docs/runbooks/sdks.md`. Pass when the five canonical documents are
byte-identical.

## US06 - the Studio

Open the run in the Studio as a developer and as a granted developer. Check: levels are marked by text,
the reveal action appears only for the granted user and asks for confirmation, navigating away clears
the values, and the browser storage and network log hold no value the server withheld. Suites:
`npm --prefix frontend test -- PayloadDisclosure` and the Chromium journey above. Accessibility is
checked with tools and a keyboard walkthrough, not claimed from assistive-technology use.

## US07 - the report

`sagat workflow validate` on a `public` root with unannotated children prints one
`CLASSIFICATION_INHERITED_OPEN` warning per child; the Studio settings panel shows the same list; a
version diff shows `CLASSIFICATION_RELAXED`. Suites: `--test contract_payload_classification -- validate`,
`npm --prefix frontend test -- WorkflowSettings`.

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
`--test integration_payload_classification -- events`, `--test e2e_events_webhooks_messaging`. Pass when no
canary reaches a caller without the grant and the audit count matches the reveals.

## Performance and failure

`cargo test -p sagat-tests --test perf_payload_classification -- --ignored` writes
`tests/performance/reports/payload-classification.md`; read its environment section before its numbers.
`cargo test -p sagat-tests --test fault_payload_classification` injects the crashes, the audit failure
and the races of T016-050. A skipped infrastructure test fails the run; report any such failure as it is.

## What this guide does not prove

It does not prove field classification of events (their data is `pii` as a whole, G282, G291), erasure, retention, taint tracking, or the
production values of the bounds. The numbers of the benchmark are measurements on one environment, not
capacity claims.
