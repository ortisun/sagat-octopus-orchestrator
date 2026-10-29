# Errors, reasons and failure vocabulary (draft contract)

Draft for FR-021-017 and the REST surfaces. Destination denials follow the `<AREA>_DESTINATION_DENIED`
naming of the parity drafts (coordinator C16); encoded-field refusals use 031's codes (C7). `Error.code` stays the closed set of
`specs/001-platform-foundation/contracts/openapi.yaml` and `sdks/error-catalog.json`; this feature
adds only `details.reason` values and step failure codes. Messages are fixed text; none carries a
URL, header, body, address, value, credential or provider message.

## REST `details.reason` values (new)

| Code | Reason | When |
|---|---|---|
| `VALIDATION_FAILED` | `RESOURCE_TYPE_UNKNOWN` | type key or version not in the catalog |
| `VALIDATION_FAILED` | `RESOURCE_SETTINGS` | settings fail the pinned type schema (with a JSON pointer, never the value) |
| `VALIDATION_FAILED` | `CREDENTIAL_IN_SETTINGS` | userinfo in a URL or an auth-like header in settings |
| `VALIDATION_FAILED` | `SLOT_SECRET_NOT_OPENBAO` | slot secret has local authority (migrate first) |
| `VALIDATION_FAILED` | `MANAGED_BY_CONNECTIONS` | any database driver type (017/024) through the resource API |
| `FORBIDDEN` | `SLOT_DESTINATION_REQUIRES_MANAGE` | creating a resource or changing a host-bearing setting or slot secret without `secret:manage` on that secret |
| `VALIDATION_FAILED` | `IDEMPOTENCY_GENERATE_NOT_ALLOWED` | `idempotency: generate` on an HMAC or OIDC route |
| `VALIDATION_FAILED` | `BLOB_MODE_UNAVAILABLE` | blob output requested before the streaming scanner is enabled |
| `VALIDATION_FAILED` | `HEADER_NOT_ALLOWED` | a refused header name in settings, operation headers or captures |
| `VALIDATION_FAILED` | `CDC_IDENTIFIER` | `slot` or `publication` outside its identifier rule |
| `VALIDATION_FAILED` | `OPERATION_NOT_IN_TYPE` | node operation not offered by the resource's type |
| `VALIDATION_FAILED` | `VARIABLE_NOT_FOUND` | unknown `vars.<name>` at publication, or deleted at admission |
| `VALIDATION_FAILED` | `VARIABLE_LEVEL` | `pii` or `secret` requested for a variable |
| `VALIDATION_FAILED` | `VARIABLE_VALUE` | value does not match type or bound |
| `VALIDATION_FAILED` | `ROUTE_PATH` | path outside the route grammar or bounds |
| `CONFLICT` | `ROUTE_CONFLICT` | same method and path shape in the namespace |
| `CONFLICT` | `REVISION_CONFLICT` | existing reason, reused for resources and variables |
| `CONFLICT` | `IDEMPOTENCY_KEY_REUSED` | existing reason, reused for receipts |
| `CONFLICT` | `RESOURCE_UNAVAILABLE` | paused or deleted resource at publication, admission or probe |
| `BAD_REQUEST` | `IDEMPOTENCY_KEY_REQUIRED` | route call without its idempotency header |
| `FORBIDDEN` | `PII_REVEAL_NOT_GRANTED` | existing reason, reused for previews over retained events |
| `QUOTA_EXCEEDED` | `LIMIT` with `details.resource` | any ceiling of the plan |
| `VALIDATION_FAILED` | `UNSUPPORTED_NODE` with `dependency: SCOPED_RESOURCE_OPERATION` | `resourceOperation` inside a body (G1120) |

Route and e-mail ingress: before authentication only route-independent refusals (`413` global cap,
`431` headers, `429` from the prefix-slot limiter); every authentication failure is one `401`; after
authentication `503` paused, `413` route limit, `429` route rate, `400 IDEMPOTENCY_KEY_REQUIRED`,
`409 EVENT_ID_REUSED`, `422` invalid document. The `MAILGUN` profile maps permanent refusals to `406`
and admission to `200` (FR-021-028). Existing webhook endpoints keep their order (G1142).

## Step failure codes (managed operations)

| Code | Retryable | Meaning |
|---|---|---|
| `CONFIGURATION` | no | settings or capability inconsistent with the runner build |
| `RESOURCE_DESTINATION_DENIED` | no | egress policy refused a name, address, port or redirect; audited |
| `CONNECTION` | yes (safe only) | connection could not be established; nothing was sent |
| `TLS` | no | certificate or name verification failed |
| `AUTHENTICATION` | no | provider `401`, OAuth2 token refused, SMTP auth refused, Slack auth error |
| `PROVIDER_FORBIDDEN` | no | provider `403` |
| `PROVIDER_REJECTED` | no | other `4xx`, `3xx`, SMTP `5xx` before data, Slack `ok:false` |
| `PROVIDER_RATE_LIMITED` | yes | `408`/`429`; honours `Retry-After` within the deadline |
| `PROVIDER_UNAVAILABLE` | yes for safe and idempotent effects; otherwise becomes `OUTCOME_UNKNOWN` | `5xx` or a reset after sending |
| `TIMEOUT` | as `PROVIDER_UNAVAILABLE` | deadline reached |
| `CANCELLED` | no | lease cancelled; not proof that nothing was sent |
| `RESULT_LIMIT` | no | output over its bound in inline mode |
| `OPERATION_MISMATCH` | no | GraphQL kind differs from the document |
| `GRAPHQL_ERRORS` | no | `errors` with null `data` |
| `SECRET_IN_OUTPUT` | no | a resolved slot value appears in the output |
| `CAPABILITY_UNAVAILABLE` | yes | OpenBao or resolution unavailable; nothing was sent |
| `RESOURCE_UNAVAILABLE` | no | resource paused/deleted or authority revoked at capability use; also ends a connector stream lease |
| `CAPACITY` | yes | per-resource/namespace/tenant concurrency full before dispatch |
| `OUTCOME_UNKNOWN` | never automatically | effect may have happened; reconcile or manual review |
| `ENCODING_READ_BY_EXPRESSION` | no | an expression reached an encoded (client-encrypted) field; code and positions owned by 031's registry (`specs/031-client-side-encryption/contracts/workflow-encoding.md`); publication refuses it where the position is known, and runtime evaluation fails with the same code |
| `WORKER_ERROR` | yes (safe only) | runner fault before dispatch |

Connector health classes (closed): `CONNECTION`, `TLS`, `AUTHENTICATION`, `CONNECTOR_DESTINATION_DENIED`,
`WAL_LEVEL`, `NO_PUBLICATION`, `NO_SLOT`, `WRONG_PLUGIN`, `NOT_PERMITTED`, `LAGGING`, `BACKLOGGED`,
`PROVIDER_REJECTED`, `PROVIDER_UNAVAILABLE`, `FILTER_ERROR`.
