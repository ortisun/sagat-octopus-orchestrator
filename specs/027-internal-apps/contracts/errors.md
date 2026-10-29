# Typed errors — feature 027 (draft)

No new `Error.code`. Every error uses the canonical envelope (`code`, `message`, `retryable`,
`traceId`, `remediation`, `details`) of `specs/001-platform-foundation/contracts/openapi.yaml` and
`sdks/error-catalog.json`. The refinements below are additive `details.reason` values, merged with
the OpenAPI by T027-004. A message never contains an argument, a value, a document excerpt, a token
or another tenant's data.

| `code` (HTTP) | `details.reason` | When | Retryable |
|---|---|---|---|
| `UNAUTHENTICATED` (401) | `APP_SESSION_EXPIRED` | Session past `expires_at`, ended or revoked | no — reopen the app |
| `UNAUTHENTICATED` (401) | `APP_SESSION_INVALID` | Unknown token on a runtime route, or a non-session token there | no |
| `UNAUTHENTICATED` (401) | `AUTHORIZATION_CODE_INVALID` | Embedded session creation whose code, verifier or redirect URI the issuer refuses (no issuer detail echoed) | no — sign in again |
| `FORBIDDEN` (403) | `APP_AUDIENCE` | Subject outside a non-empty audience | no |
| `FORBIDDEN` (403) | *(existing permission refusal)* | Missing `app:*` or the underlying permission at invocation | no |
| `FORBIDDEN` (403) | `COMPONENTS_DISABLED` | Upload, or publication of a version with `custom`, while the namespace policy is off | no |
| `FORBIDDEN` (403) | `PII_REVEAL_NOT_GRANTED` (existing, 016) | `reveal: [pii]` without a reveal grant | no |
| `NOT_FOUND` (404) | *(none)* | Missing or foreign app, version, component, request, session route; a run of another workflow, namespace or tenant | no |
| `NOT_FOUND` (404) | `BINDING_NOT_DECLARED` | Binding id absent from the pinned version | no |
| `NOT_FOUND` (404) | `ROUTE_NOT_FOUND` (existing) | Any non-apps API route called on the apps origin | no |
| `CONFLICT` (409) | `REVISION_CONFLICT` (existing) | Draft save with a stale `expectedRevision` | no |
| `CONFLICT` (409) | `APPROVAL_REQUIRED` (existing, 011) | Direct publish into a namespace that requires approvals | no |
| `CONFLICT` (409) | `ACTIVE_VERSION_CHANGED` (existing, 011) | Activation with a stale `expectedVersion` | no |
| `CONFLICT` (409) | `IDEMPOTENCY_KEY_REUSED` (existing) | Same action key or idempotency key with different content | no |
| `CONFLICT` (409) | `APP_DELETED` | Draft, publication or session on a tombstoned app | no |
| `CONFLICT` (409) | `COMPONENT_IN_USE` | Removing a component version pinned by a published app version | no |
| `VALIDATION_FAILED` (422) | `APP_DOCUMENT_INVALID` | Publication of an invalid draft; `details.problems` lists code + JSON pointer (≤ 256) | no |
| `VALIDATION_FAILED` (422) | `BINDING_CAPABILITY_UNAVAILABLE` | `operation` kind, or a target capability the server has not enabled | no |
| `VALIDATION_FAILED` (422) | `BINDING_CONTRACT_CHANGED` | Arguments no longer match the target's active schema | no — republish the app |
| `VALIDATION_FAILED` (422) | `INVALID_ARGUMENTS` (existing) | Invocation values fail the mapping or target schema | no |
| `VALIDATION_FAILED` (422) | `EMBED_ORIGIN_NOT_ALLOWED` | Origin outside namespace policy ∩ deployment allowlist, or not exact `https` | no |
| `VALIDATION_FAILED` (422) | `EMBEDDING_UNAVAILABLE` | No apps origin configured | no |
| `VALIDATION_FAILED` (422) | `BUNDLE_UNSAFE_SEQUENCE` | Component bytes contain `</script`, `</style` or `<!--` in any ASCII letter case, or are not UTF-8 | no |
| `VALIDATION_FAILED` (422) | `RESERVED_KEY_PREFIX` | An ordinary caller's `Idempotency-Key` or `updateKey` starts with `app:` (reserved for app actions, D1365) | no — choose another key |
| `VALIDATION_FAILED` (422) | `COMPONENT_SANDBOX_UNAVAILABLE` | Enabling custom components, uploading or publishing one, without a configured component sandbox origin | no |
| `VALIDATION_FAILED` (422) | `MISSING_DEPENDENCY` | Promotion target lacks a workflow, operation or component (`details.missing` names, ≤ 64) | no |
| `BAD_REQUEST` (400) | *(existing)* | Malformed JSON, unknown body field, missing `actionKey` on a write | no |
| `BAD_REQUEST` (413) | *(existing `PayloadTooLarge`)* | Body above the route's ceiling | no |
| `QUOTA_EXCEEDED` (429) | *(resource name)* | `apps`, `app_components`, `app_sessions`, content bytes | no |
| `RATE_LIMITED` (429) | *(existing)* | Namespace API bucket or per-session invocation rate; `Retry-After` set | yes |
| `UNAVAILABLE` (503) | *(existing)* | Database or audit unavailable | yes |

Problem codes inside `APP_DOCUMENT_INVALID` (closed): `UNKNOWN_FIELD`, `LIMIT`, `DUPLICATE_ID`,
`UNRESOLVED_REFERENCE`, `PREDICATE_DEPTH`, `UNKNOWN_TARGET`, `UNKNOWN_OPERATION`,
`BINDING_CONTRACT_CHANGED`, `BINDING_CAPABILITY_UNAVAILABLE`, `MISSING_ACCESSIBLE_NAME`,
`CHART_WITHOUT_TITLE`, `TABLE_WITHOUT_CAPTION`, `CLASSIFICATION_PASSING`, `SECRET_BINDING`,
`COMPONENTS_DISABLED`, `UNKNOWN_COMPONENT`, `LAYOUT_OVERLAP`,
`COMPONENT_AUTHOR_LEVEL` (the publisher or a component uploader lacks the rights for the level the
version passes to that component), `FRAME_TAINTED_FLOW` (a frame-originated value reaches the target
or arguments of a binding whose result can reach a frame, or of a write binding),
`COMPONENT_SANDBOX_UNAVAILABLE`.

Encoded fields (feature 031) use only the codes of 031's registry and errors
(`specs/031-client-side-encryption/contracts/encoded-field-registry.md`,
`specs/031-client-side-encryption/contracts/errors.md`): `ENCODING_READ_BY_FORM` for a `form` component
over a target whose schema declares encoding, `ENCODING_NOT_PRODUCIBLE` for a `start` binding whose input
mapping reaches an encoded location (refused at runtime by `startRun` with `ENCODING_REQUIRED`), and
`ENCODING_READ_BY_EXPRESSION` for a reference or predicate that reads inside an encoded value. This feature defines no
encrypted-field code of its own.

A frame capability that is unknown, expired, already consumed or whose session ended is a bare `404`
on the sandbox origin, with the sandbox headers and no body detail.
Warnings (never block): `UPDATE_WITHOUT_EXPECTED_REVISION`, `START_WITHOUT_DEDUP_HINT`,
`REFRESH_INTERVAL_CLAMPED`.
