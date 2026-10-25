# Payload classification and audited disclosure

Feature 016 is under integration. The implementation and focused evidence are recorded in
[tasks/project-completion/integration-notes.md](../../tasks/project-completion/integration-notes.md).
The complete canary, upgrade, browser and release gates remain open; this page is not a release
approval. [ADR-0033](../adr/ADR-0033-payload-values-are-classified-per-field.md) defines the policy;
[ADR-0036](../adr/ADR-0036-integrated-scoped-managed-and-classified-execution.md) records the
approved interaction with scoped execution and managed PostgreSQL operations.

## Author a schema

Set `x-sagat-classification` to `public`, `internal`, `pii` or `secret` on a supported JSON Schema.
Carriers are workflow `inputSchema`, workflow `outputSchema`, and `outputSchema` on an activity,
script, wait-event or child node, including those nodes in named bodies.

```json
{
  "type": "object",
  "x-sagat-classification": "public",
  "additionalProperties": false,
  "properties": {
    "reference": {"type": "string"},
    "account": {"type": "string", "x-sagat-classification": "pii"}
  }
}
```

A descendant inherits its parent's effective level and can only increase it. An unmarked root is
PII; an unmarked declared property under a public parent inherits public. This last case produces
an authoring warning. An undeclared property is at least PII, including under a public parent.
Review the validation report before publication. Unsupported uses of classification are refused
with closed `CLASSIFICATION_*` findings; arbitrary JSON Schema applicators do not establish a
new classification policy. References must resolve inside their own carrier.

Published versions are immutable. New publications pin interpretation rules 1. Existing versions
migrated with rules 0 remain undisclosable; publishing a new version does not change a run pinned
to the old version. Digest deduplication does not reinterpret an existing version: publishing the
document of a legacy version again returns that version (`created: false`) with
`classificationRules: 0`, and its runs stay undisclosable. To adopt the rules, publish a changed
document (a changed `metadata.description` is enough). Every version in the API carries
`classificationRules`, so a client can tell. A rerun or a child is evaluated against its own pinned
version.

## Compare versions

The version comparison (`diffWorkflow`, the Studio comparison, `sagat sync`) lists
attention findings, never breaking ones, for the classification:

- `CLASSIFICATION_RELAXED` and `CLASSIFICATION_TIGHTENED`, one per path whose effective level
  changed. A side with no schema for a location, or published under rules 0, discloses nothing and
  counts as `undisclosed`, as strict as `secret`. Adding an `outputSchema` that makes a field
  `public` is therefore a relaxation, and so is the first version under the current rules for a
  legacy workflow (every location its annotations make readable is listed, up to 100 per kind and
  then `MORE_FINDINGS` with the exact remainder). The first draft of a workflow is not compared.
- `CLASSIFICATION_TOO_COMPLEX`, `CLASSIFICATION_UNRESOLVED_REF` and
  `CLASSIFICATION_UNSUPPORTED_KEYWORD`, once per carrier, when a carrier of either side could not be
  walked completely. None of its levels was compared, so a relaxation inside it is not reported:
  fix the carrier and compare again.

`sagat sync` prints each as a `prepare` line. The Studio comparison lists them all under "Needs
preparation"; the classification panel of the workflow settings shows the level changes and a
notice for the carriers that could not be compared.

## Read permitted values

The explicit operation is `POST /v1/runs/{runId}/payload-disclosures`:

```json
{"source":"input"}
```

Use `source: "result"` for the retained workflow result, or `source: "journal"` with a positive
`sequence` for an eligible output event. The result selector reads only `runs.result_digest` and
the workflow output schema. A journal selector uses the exact event and persisted scoped node
identity; it does not substitute another sequence, attempt or same-named node.

| Effective level | Value reader |
|---|---|
| public | A caller with run read permission |
| internal | Developer, operator or administrator |
| pii | A current developer, operator or administrator with a grant in this namespace, explicitly requesting reveal |
| secret | No caller through this disclosure operation |

Structural inspectors receive withheld nodes where permitted. Other readers receive no hidden
keys or values. Missing schemas, legacy versions, invalid payloads and unsupported exact numeric
validation produce bounded structural or unavailable answers; a caller without
`run:payload:inspect` gets `NOT_DISCLOSABLE` for every one of them and no `sizeBytes`, so the state
of a payload is not learned from the answer. A structural answer to an inspector is audited as
`payload.inspected` (reason `classified-v1:<selector>:structure`, one record per 300 s).

Numbers are checked on the spelling that was sent. Ordinary values such as `9.99`, `12.5` or
`1.5e2` disclose under `minimum`, `multipleOf`, `const`, `enum` and `uniqueItems`. A valid payload
is withheld only when a numeric keyword would compare a rounded value: a fraction with more digits
than a 64-bit float keeps (for example thirty decimals), `1e400`, or an integer above 2^53 in a
carrier that has a decimal bound or constant. Inspectors then see `UNSUPPORTED_FORMAT`. The API
and CLI preserve numeric representations; Studio refuses numbers it cannot display exactly and
directs the user to those interfaces.

A carrier is also refused (`SCHEMA_UNUSABLE` for inspectors, `NOT_DISCLOSABLE` for others) when it
holds more than 32 `pattern` keywords and `patternProperties` keys, or when one read would cost
more than the walk budget (4,000,000 steps, five seconds): for example thousands of `allOf` branches
repeated over a long array. Publication does not yet warn about either (G642); keep carriers small.
The compiled validator of a version is built before the read transaction opens and cached per
process; a version whose carrier cannot be compiled is remembered, so it costs one attempt.

Responses use `Cache-Control: private, no-store`. The request body is limited to 1 KiB, and
payload, schema, traversal and response ceilings also apply. Treat truncation as incomplete
information. Disclosure does not expand retention and does not fetch an external blob's contents.

## Grant, reveal and revoke

Only an administrator with membership management may manage the per-namespace list:

```text
sagat namespace reveal-grants list
sagat namespace reveal-grants grant SUBJECT
sagat namespace reveal-grants revoke SUBJECT
sagat workflow inspect RUN --values
sagat workflow inspect RUN --values --reveal-pii
sagat workflow inspect RUN --values --sequence SEQUENCE
```

Use the usual CLI session and namespace settings. A namespace holds at most 256 grants.
Membership deletion cascades the grant. A grant does not supply the role's internal-read
permission; a viewer or machine identity cannot use a leftover grant after a role change.
Administrators also need their own explicit grant to reveal PII. Self-grant is audited.

For API disclosure, add `"reveal":["pii"]`. A successful reveal commits exactly one
`payload.pii_revealed` audit record before returning values; retries produce another record.
A failure to persist or commit that mandatory audit returns no value. A denial returns the typed
`PII_REVEAL_NOT_GRANTED` refusal; its denial audit is best effort. Revocation and current
membership checks are fenced against a concurrent read transaction.

## Event inbox and dead letters: response change

Inbox event details and dead-letter details return structure by default. Explicit
`reveal=pii` additionally requires event read permission, current internal-read permission and a
grant in the event's namespace. The whole event payload is treated as PII until event type schemas
exist. Revealing a dead letter includes its body and header values under the same audited read.
A dead-letter list never carries bodies or headers and does not reveal values even when passed
a reveal query. Non-JSON or truncated bytes receive an opaque structural answer.

Clients which previously read `data`, `body` or `headers` must handle these fields being absent.
Redrive, discard, event dispatch, deduplication and broker acknowledgements retain their existing
semantics. See [events.md](events.md) and the canonical OpenAPI contract.

## Coverage and incident response

The annotation governs the closed carriers above. It does not classify declared state, Query or
Update results, a human approval form, logs, or arbitrary provider data. Native managed database
step outputs are outside the closed journal carriers. Explicitly mapping data into a supported
workflow result applies that receiving carrier's schema; there is no automatic taint tracking.
`secret` is a disclosure level, not a secret store: credentials belong in the secret service backed
by OpenBao and must never be placed in workflow payloads.

When investigating an unexpected reveal:

1. Revoke the namespace grant and verify the subject's current membership.
2. Correlate the request trace with the restricted audit record and pinned workflow version.
3. Check the carrier, effective inherited levels and the exact selector without copying values
   into tickets, logs or telemetry.
4. Correct the schema and publish a new version. Do not mutate an existing version or grant a
   broader role to work around a structural answer.
5. Preserve evidence and run the disclosure, fault and canary suites before considering a release.

Metrics and spans use fixed outcome labels; they must not contain payload values, keys, schema
fragments, tokens or secret material. Audit records describe the action and selector, never the
revealed content.
