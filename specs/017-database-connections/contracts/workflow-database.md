# Database step in canonical IR

The canonical schema is `specs/001-platform-foundation/contracts/workflow.schema.json`.
`type: database` is a closed object: `id`, `connectionRef`, `operation`, `statement`, optional
`parameters`, `limits`, `next`, `onError` and `timeoutMs`. A reference has a namespace-local `name`
and optional positive `revision`; run admission freezes the revision. No credential value belongs
in a workflow. Bind values through positional parameters rather than SQL interpolation.

```json
{
  "id": "orders",
  "type": "database",
  "connectionRef": {"name": "orders-db"},
  "operation": "query",
  "statement": "SELECT id, total FROM orders WHERE customer_id = $1",
  "parameters": [{"type": "integer", "expression": "input.customerId"}],
  "limits": {"maxRows": 100, "timeoutMs": 5000}
}
```

Each parameter is either `{"literal":{"type":"integer","value":"123"}}` or
`{"type":"integer","expression":"input.customerId"}`. The expression uses the existing bounded
workflow evaluator and must produce the value representation of its declared type. It does not
interpolate the statement. Integer and decimal values use strings, preserving precision across all
five SDKs. Types: null, boolean, integer, decimal, float, text, timestamp (RFC3339), date (ISO date),
binary (canonical padded base64), json (a JSON value). Null is `{"type":"null","value":null}`.
Integers have at most 40 decimal characters and decimals 16384; finite float only; text excludes NUL.
Parameter counts and aggregate UTF-8 bytes obey effective limits. Unsupported driver types fail
with a safe category. Parameter expressions are compiled at publication; values are checked at use.

The managed activity namespace `sagat.database.*` is reserved, including compensation and
reconciliation activity references. Ordinary activity declarations cannot impersonate this node.
The domain carries a typed operation; it does not interpret SQL or perform I/O. Every operation has
one attempt. Writes (`execute`) are high-risk and irreversible by default: unknown outcome enters
manual review using existing engine semantics. Caller-supplied retry/effect/secret overrides are
rejected. A later operation that compensates a write must be declared explicitly.

The shared lossless fixture is
`tests/contract/database-connections/typed-parameters.workflow.json`. Database validation diagnostics
contain safe categories and paths, never SQL or parameter content. Execution uses the dedicated
executor and [capability checks](managed-executor.md) verified in T017-013..017. Publication
and run admission validate current catalog/ACL authority, and runs retain immutable
connection revisions in their creation transaction. Replaying a start retains the original revision
and rechecks current authority. A rerun is a new admission and resolves an unversioned reference
again; specify `revision` to require the same settings across separate runs.
