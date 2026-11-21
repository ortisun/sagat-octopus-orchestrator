# Workflow IR: `x-sagat-encoding` (draft)

Status: draft fragment of feature 031; the canonical schema stays
`specs/001-platform-foundation/contracts/workflow.schema.json` and is changed only by T031-006. The document stays
`sagat-octopus-orchestrator.io/v1` (ADR-0032): the key is an annotation inside the free-form carrier schemas, which the
base schema already allows, so an engine without this feature ignores it. The complete, cross-draft list of positions,
carriers and write paths is [encoded-field-registry.md](encoded-field-registry.md); this file states the rules.

## 1. The annotation and its carriers

```json
"x-sagat-encoding": "encrypted" | "compressed"
```

| Carrier | Where | Governs |
|---|---|---|
| `inputSchema` | document root | run input (REST start, `inputBlobRef`, child admission, `continueAsNew`, trigger and schedule starts) |
| `outputSchema` | document root | run result (`end.outputExpr`, 022 `earlyReturn.result`) and what a parent receives from a child |
| `outputSchema` | `activity` node | the output a worker reports for that node |
| `outputSchema` | `child` node | the child result as retained by the parent |
| `outputSchema` | `waitEvent` node | a signal or inbox event payload that satisfies the wait |
| `heartbeatSchema` | `activity` node (019) | checkpoint details (side channel, FR-031-035) |
| `streams.<name>.schema` | document root (026) | stream events appended by attempts (side channel, FR-031-035) |

The two side-channel carriers are an amendment to the 016 carrier list (coordinator C6); the first feature implemented
among 019, 026 and 031 opens the single amendment of FR-016-002. Refused elsewhere (`ENCODING_NOT_APPLICABLE`) and
for nodes executed by a platform-managed executor (`ENCODING_NOT_PRODUCIBLE`) as the registry lists.

## 2. Positions inside a carrier

Allowed: the carrier root; a schema under `properties/<name>`; a schema under `items` (pointer segment `*`). The path from
the root to the annotation may use only `properties` and `items`. Refused (`ENCODING_UNSUPPORTED_LOCATION`):
`additionalProperties`, `patternProperties`, `prefixItems`, `allOf`, `anyOf`, `oneOf`, `not`, `if`/`then`/`else`,
`dependentSchemas`, `$defs`, a `$ref` target or a path through a `$ref`, and a property literally named `*`. An annotation
under an encoded location is `ENCODING_NESTED`. Bounds: at most 64 locations per carrier, 4 `*` and 16 segments per
location (`ENCODING_TOO_COMPLEX`).

The subschema at an encoded location describes the **plaintext** (SDKs may validate it before encoding and after
decoding). The server replaces it by [envelope.schema.json](envelope.schema.json) when it validates a payload (the
"encoded view"), and 016 classification annotations inside it collapse to the location's level (FR-031-003).

Example:

```json
"inputSchema": {
  "type": "object", "additionalProperties": false, "x-sagat-classification": "internal",
  "properties": {
    "orderId": {"type": "string"},
    "card": {"type": "object", "x-sagat-encoding": "encrypted", "x-sagat-classification": "pii",
             "properties": {"pan": {"type": "string", "x-sagat-classification": "secret"}}},
    "items": {"type": "array", "items": {"type": "object", "properties": {
      "sku": {"type": "string"},
      "note": {"type": "string", "x-sagat-encoding": "compressed"}}}}
  }
}
```

Plan: `/card` encrypted, level `secret` (strictest in its subtree, warning `ENCODING_LEVELS_COLLAPSED`);
`/items/*/note` compressed, level `internal`. Every envelope at `/card` carries `ctx.p = "/card"`.

## 3. Expression positions

The registry lists every position with its kind. Access paths are computed by `crates/infra-expr` (a variable followed
by constant field and index segments; a non-literal index becomes `*`; `has(path)` is a presence test). Variables map to
carriers: `input` → `inputSchema`; `steps.<node>.output` → that node's `outputSchema`; `item` → the element schema of the
`forEach.items` source; `checkpoint` (019) → `heartbeatSchema`. A position not in the registry is refused for any
document with encoded locations (fail closed).

## 4. Carriage: the location rule

An envelope is bound to a tenant, a namespace and a pointer pattern (`ctx`, [envelope.md](envelope.md)). Hence:

- **Same document** (`end.outputExpr`, `earlyReturn.result` → `outputSchema`; `continueAsNew.input` → `inputSchema`):
  the source path's encoded locations, re-rooted at the target, MUST equal the target's encoded locations with equal
  encoding **and equal pointer patterns** — a carriage that moves an encoded value to another pointer is
  `ENCODING_CARRIAGE_MISMATCH` (re-encode it in a customer worker step instead).
- **Other document, same namespace** (`child.input`, 022 failure-handler child `input`, 022 `fromRun` pins, reset or
  rerun into another version): publication warns `ENCODING_CARRIAGE_UNVERIFIED` when the target version is not pinned;
  at admission the engine maps the source's planned locations through the carriage path and refuses with
  `ENCODING_CARRIAGE_MISMATCH` unless the target's pinned plan declares every mapped location with the same encoding
  and pointer pattern. This holds whatever the target's `encoding_rules` (a target with rules `0` and a non-empty mapped
  plan is refused), so ciphertext never reaches a workflow that would evaluate it. Target locations the source does not
  cover are checked by the write path (present plaintext is `ENCODING_REQUIRED`; absence is valid).
- **Other namespace** (030 `serviceCall.input`, `compensate.input`): refused when the source holds an encoded location
  (`ENCODING_CARRIAGE_MISMATCH`, D1667).
- **Sources without a plan** (REST, signals, inbox events, triggers, schedules, forms, mocks, standalone inputs): the
  write path refuses any envelope at a location the receiving carrier does not declare. This walk runs only for
  carriers of rules-1 versions: a planless write into a rules-0 version admits envelope-shaped data as ordinary JSON
  (G1665).

## 5. Shared check for other features (FR-031-006)

`EncodingCheck::reads(document, consumer, expression_or_pointer) -> Vec<Finding>` in `crates/infra-ir/src/encoding_check.rs`.

| Consumer | Code |
|---|---|
| server expressions (registry kind I) | `ENCODING_READ_BY_EXPRESSION` |
| 020 search attribute `from`, `upsertSearchAttributes` | `ENCODING_READ_BY_SEARCH_ATTRIBUTE` |
| 020 analytics `project` (carriage included) | `ENCODING_READ_BY_ANALYTICS` |
| 022 cache key (or whole-input digest) | `ENCODING_READ_BY_CACHE_KEY` |
| 022 concurrency key | `ENCODING_READ_BY_CONCURRENCY_KEY` |
| 019 fairness `{expr}` key | `ENCODING_READ_BY_FAIRNESS_KEY` |
| 026 forms, summaries, public-link presets | `ENCODING_READ_BY_FORM` |
| 014 Query/Update | `ENCODING_READ_BY_DECLARED_OPERATION` |

## 6. Publication findings

Refused documents answer `422 VALIDATION_FAILED`, `details.reason = "ENCODING"`, `details.issues[] = {path, code,
message}` (at most 20, messages from a closed table, no document excerpt beyond a clipped pointer).

| Code | Severity |
|---|---|
| `ENCODING_INVALID_VALUE`, `ENCODING_NOT_APPLICABLE`, `ENCODING_NOT_PRODUCIBLE`, `ENCODING_UNSUPPORTED_LOCATION`, `ENCODING_NESTED`, `ENCODING_TOO_COMPLEX`, `ENCODING_NOT_ACTIVATED` | error |
| the eight `ENCODING_READ_BY_*` codes of section 5, `ENCODING_CARRIAGE_MISMATCH` | error |
| `ENCODING_LEVELS_COLLAPSED`, `ENCODING_CARRIAGE_UNVERIFIED`, `ENCODING_TRIGGER_UNVERIFIED`, `ENCODING_INERT_VERSION` | warning |
| `ENCODING_ADDED`, `ENCODING_REMOVED` | diff attention findings |

`ENCODING_NOT_ACTIVATED`: the document declares encoding while the deployment has not been activated (FR-031-036).

## 7. Pinning

`workflow_versions.encoding_rules` (`0` before this feature: the key is inert; `1`: this document). A later change of
interpretation adds `2`; `1` is never reinterpreted. The plan is computed at publication and stored with the version
(data-model.md, `workflow_version_encodings`).
