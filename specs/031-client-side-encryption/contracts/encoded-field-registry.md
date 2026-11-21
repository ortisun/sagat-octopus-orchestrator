# Encoded-field registry (normative; coordinator decision C7)

Feature 031 owns this registry: every position where the engine reads a payload value, every carriage position,
every carrier that may declare `x-sagat-encoding` and every write path that stores a value into a carrier, across the
existing IR and the parity drafts 018–030 (027 included). A draft that adds such a position registers it here (a change to this file in
the same delivery) and uses only the codes of [errors.md](errors.md); no draft defines its own encrypted-field code.
Each entry was checked against the draft text on 2026-10-27; a draft that changes the construct must update its row.

Kinds: **I** inspection position (refused when it reads an encoded location, FR-031-004); **C** carriage position
(allowed only as an exact path, under the location rule of FR-031-005); **K** carrier (may declare encoding);
**N** carrier for classification only (declaring encoding is refused); **W** write path (FR-031-010 checks apply);
**S** side channel (FR-031-035).

## Existing IR v1 (014, 016, 017)

| Construct | Kind | Code on violation |
|---|---|---|
| `condition.expression`, `switch.cases[].when`, `while`/`repeatUntil` `condition`, `initial`, `carry`, `forEach.result`, `waitEvent.correlationExpr`, activity `idempotencyTemplate`, `continueAsNew.state` | I | `ENCODING_READ_BY_EXPRESSION` |
| `database` parameter `expression` (017) | I | `ENCODING_READ_BY_EXPRESSION` |
| `state.initial`, `query.expression`, `update.validate`/`state`/`result` (014) | I (no carriage) | `ENCODING_READ_BY_DECLARED_OPERATION` |
| `child.input`, `end.outputExpr`, `return.result`, `continueAsNew.input`, `forEach.items` (array holding encoded locations only) | C | `ENCODING_READ_BY_EXPRESSION`, `ENCODING_CARRIAGE_MISMATCH` |
| workflow `inputSchema`, `outputSchema`; `outputSchema` of `activity`, `child`, `waitEvent` | K | — |
| `outputSchema` of `script`, `database` | N | `ENCODING_NOT_PRODUCIBLE` |
| state schema, Query/Update `argsSchema`/`resultSchema`, `humanApproval.formSchema`, triggers, metadata | not a carrier | `ENCODING_NOT_APPLICABLE` |
| REST `startRun` (inline, `inputBlobRef`), child admission, `CompleteStep` (inline, blob), `end`, `continueAsNew`, `signalRun`, `signalExecutionChain`, inbox match of `waitEvent`, event/webhook/message/cron trigger start | W | `ENCODING_REQUIRED`, `ENVELOPE_INVALID`, `ENVELOPE_LEVEL_MISMATCH`, `ENVELOPE_CONTEXT_MISMATCH`, `ENCODING_CARRIAGE_MISMATCH` |
| rerun (ADR-0039), compensation and reconciliation inputs | W (copy) | same; a copy into the same version and namespace passes unchanged |

## Parity drafts

| Draft | Construct (where checked) | Kind | Rule and code |
|---|---|---|---|
| 018 | schedule target `input` (`contracts/rest.openapi.yaml` `ScheduleTarget`) | W | checked against the plan of the version the schedule would use at create/update, and again at every firing (run admission); a firing refused by a changed active version fails that occurrence |
| 018 | `nominalTimeField` | W (platform-produced) | MUST name a non-encoded top-level property: `ENCODING_NOT_PRODUCIBLE` |
| 018 | `businessIdPrefix`, `businessId` | — | literals; read no payload (no workflow-id template exists in the draft; one added later is **I**) |
| 018 | Signal-/Update-with-Start start input; batch signal payloads; reapplied signals after reset | W | as `startRun` / `signalRun`; keyed writes follow FR-031-013 |
| 018 | Update arguments (with-start, batch, reapplied) | not a carrier | cannot be encoded (FR-031-001); an Update that reads an encoded location is `ENCODING_READ_BY_DECLARED_OPERATION`. The 018 note about "encrypted Update arguments" (`018 spec.md` assumptions) must read "cannot carry encoded values" |
| 018 | reset and batch rerun copies | W (copy) | same version: unchanged; another version: source-plan mapping (FR-031-005) |
| 019 | `dispatch.fairnessKey.expr` (root and node, `contracts/workflow-ir.fragment.schema.json`) | I | `ENCODING_READ_BY_FAIRNESS_KEY` |
| 019 | `heartbeatSchema` of `activity` | K, S | checkpoint details and final failure details are a write path; delivered checkpoints are in the next lease's decode plan |
| 019 | `heartbeatSchema` of `script` | N | `ENCODING_NOT_PRODUCIBLE` |
| 019 | checkpoints of an attempt with a non-empty payload plan and no encoded `heartbeatSchema` | S | refused `ENCODING_REQUIRED` (replaces 019 `REFUSED_ENCODING`) |
| 019 | push invocation `input` (`contracts/push-envelope.md`) | W (delivery) | the push envelope carries the same payload plan as a lease; a push target is codec-capable only when its registration declares it; otherwise gated like a session (FR-031-015) |
| 020 | `searchAttributes.<name>.from`; node `upsertSearchAttributes.<name>` | I | `ENCODING_READ_BY_SEARCH_ATTRIBUTE` |
| 020 | node `analytics[].project.<field>` | I (carriage included) | `ENCODING_READ_BY_ANALYTICS` |
| 020 | archive export | carriage out | envelopes exported unchanged, never decoded; C9 applies by the envelope's `lvl` |
| 021 | `resourceOperation` dynamic values (`{expression}` in path, query, headers, body; `answer` over the response) | I (no carriage: a managed executor holds no customer key) | `ENCODING_READ_BY_EXPRESSION` (replaces 021 runtime `ENCRYPTED_VALUE`) |
| 021 | `resourceOperation.outputSchema` | N | `ENCODING_NOT_PRODUCIBLE` |
| 021 | trigger filters, `vars` comparisons over event data | — | event data has no carrier (G1524); an envelope met at run time fails as an expression error, with no new code |
| 022 | node `cache.key` (and a key derived from the whole input) | I | `ENCODING_READ_BY_CACHE_KEY` |
| 022 | root and node `concurrency.key` | I | `ENCODING_READ_BY_CONCURRENCY_KEY` |
| 022 | `earlyReturn.result` | C (same document, into `outputSchema`) | as `end.outputExpr` |
| 022 | failure-handler child `input` | C (other document) | as `child.input` |
| 022 | cache hit copy | W (copy) | allowed only when the entry's fingerprint includes the same `encoding_rules` and output-plan digest; envelopes are copied unchanged within one namespace |
| 022 | pin `{mock}` | W | the mock is checked against the node's output plan |
| 022 | pin `{fromRun, fromNode}` | W (copy, other run) | source-plan mapping: `ENCODING_CARRIAGE_MISMATCH` |
| 022 | standalone step input | W | as `startRun` of the private one-node definition |
| 024 | `database` / `dataTransform` `parameters`, `scriptInput`; `dataPublish` | I | `ENCODING_READ_BY_EXPRESSION`; outputs `ENCODING_NOT_PRODUCIBLE` |
| 025 | agent `context.<name>`; tool `result` projection | I (sent to a provider or model) | `ENCODING_READ_BY_EXPRESSION` |
| 025 | agent `outputSchema`; tool `inputSchema` (model-written arguments) | N | `ENCODING_NOT_PRODUCIBLE`; a tool bound to a target whose input declares encoding is refused the same way |
| 026 | derived start form, `x-sagat-form` hints, approval `form`, approval `summary.fields[].value`, public link presets | I | `ENCODING_READ_BY_FORM` at publication; the derived form reports the same code at request time (replaces 026 `ENCRYPTED_FIELD`) |
| 026 | public form submission | W | as `startRun`; a form never produces envelopes, so it cannot target an encoded input |
| 026 | `streams.<name>.schema` | K, S | stream appends are a write path; an attempt with a non-empty plan may append only to channels whose schema declares the encoded locations it uses |
| 026 | run-and-wait result | carriage out | envelopes returned unchanged under 016 disclosure; never decoded |
| 027 | display bindings (`runResult`, `query`/`update` results, `listRuns`) rendered by components (`027 spec.md` FR-027-018, FR-027-020) | carriage out (display) | an encoded node is shown whole as "Encrypted" and never decoded by the platform or the app runtime; codec decoding in apps is out of 027's scope |
| 027 | references and predicates (the closed language of FR-027-003, D1366) that descend into, compare or test the content of an encoded location | I | `ENCODING_READ_BY_EXPRESSION` at app publication |
| 027 | app-issued `start` input mapping and `update` argument mapping (FR-027-012) | W (app-produced) | an app runtime cannot produce envelopes: a mapping that writes into an encoded location of the target's input is `ENCODING_NOT_PRODUCIBLE` at app publication; at run time the `startRun` write path still refuses plaintext (`ENCODING_REQUIRED`). Update arguments are never carriers (FR-031-001). Both codes replace 027 `ENCRYPTED_FIELD_UNSUPPORTED` (`027 contracts/errors.md`) |
| 030 | `serviceCall.input`, `compensate.input` | C (other namespace) | refused when the source holds an encoded location (`ENCODING_CARRIAGE_MISMATCH`): encoded values do not cross namespaces in v1 (D1667, G1536) |
| 030 | operation `inputSchema`, `outputSchema` (`contracts/service.schema.json`) | N | `ENCODING_NOT_APPLICABLE`; a handler version whose input or output declares encoding cannot be bound |
| 030 | handler admission; result return to the caller | W | runtime check: any envelope → `ENCODING_CARRIAGE_MISMATCH` or `ENVELOPE_CONTEXT_MISMATCH` |

## Rules every row obeys

1. An expression position not in this file is refused for documents with encoded locations until it is registered
   (fail closed).
2. A carrier not marked **K** refuses `x-sagat-encoding`.
3. Every **W** row applies the location rule: under `encoding_rules: 1`, envelopes exist exactly at the declared
   locations of the receiving carrier, each bound to the run's tenant, namespace and the location's pointer pattern.
4. Every **S** row applies FR-031-035.
