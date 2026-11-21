# Push invocation envelope and response contract (draft)

An engine replica invokes a push endpoint after the claim that leased the attempt has committed (ADR-0051). The
envelope has a fixed, platform-defined shape and goes to a tenant-configured destination, so it is case (b) of the
placement rule held by ADR-0055 (coordinator decision C3); the signing key is listed in 032's key inventory (C11).
Per 031's encoded-field registry (`specs/031-client-side-encryption/contracts/encoded-field-registry.md`) the
envelope's `input` carries the same payload plan as a lease, and an endpoint is codec-capable only when its revision
declares `codecCapable: true`; otherwise a task with a non-empty payload plan is gated exactly like a session without
a codec (FR-031-015: it stays `READY` for a capable worker). The
request is made by the guarded egress client of `crates/infra-egress` (destination and every resolved address
checked, no redirects, no environment proxies, bounded time and size). Delivery is **at least once**: the same
attempt can be invoked again after a lost lease, and AWS Lambda `Event` invocations can be retried by Lambda itself;
the function deduplicates on `idempotencyKey`.

## Request

`POST <target>` (kind `https`) or Lambda `Invoke` with the envelope as payload (kind `aws-lambda`).

Headers for kind `https` are below. For kind `aws-lambda` the Invoke payload is a wrapper
`{"sagat": {"envelopeVersion", "keyId", "timestamp", "signature", "deliveryId", "traceparent"}, "body": "<the
canonical envelope as a JSON string>"}` and the signature covers the bytes of `body`; Lambda's `ClientContext` is
not used because it is limited to 3,583 bytes and passed only to synchronous invocations
([AWS Invoke API](https://docs.aws.amazon.com/lambda/latest/api/API_Invoke.html)).

| Header | Value |
|---|---|
| `Content-Type` | `application/json` |
| `X-Sagat-Envelope-Version` | `1` |
| `X-Sagat-Key-Id` | id of the signing key (see `GET /v1/push-signing-keys`) |
| `X-Sagat-Timestamp` | Unix seconds when signed |
| `X-Sagat-Signature` | `ed25519=<base64url(signature)>` |
| `X-Sagat-Delivery-Id` | the task id `run/node/generation`, stable for one attempt |
| `traceparent` | W3C trace context of the attempt span |
| `Authorization` | only for `aws-sigv4` (SigV4 signature) or `google-oidc` (`Bearer <ID token>` whose audience is the target's canonical origin, minted against the pinned `https://oauth2.googleapis.com/token`, ≤1 h) |

Body (canonical JSON: sorted keys, no insignificant whitespace, UTF-8; at most the endpoint's
`maxEnvelopeBytes`):

```json
{
  "version": 1,
  "tenantId": "…uuid…",
  "namespaceId": "…uuid…",
  "endpoint": {"name": "thumbnails", "revision": 3},
  "task": {
    "taskId": "…run…/render/7",
    "runId": "…uuid…",
    "stepId": "…uuid…",
    "node": "render",
    "activity": "thumbnails.render",
    "attempt": 2,
    "generation": 7,
    "idempotencyKey": "…stable per step…",
    "deadline": "2026-10-26T12:00:30Z",
    "asyncCompletionAllowed": true,
    "taskToken": "sgtt1.… (present only when asyncCompletionAllowed is true)"
  },
  "input": {"run": {"id": "…"}, "node": "render", "input": {}, "steps": {}},
  "checkpoint": {"details": {"offset": 1024}, "recordedByAttempt": 1}
}
```

Signature input: the ASCII bytes `sagat/push/v1\n<timestamp>\n<key-id>\n` followed by the exact body bytes. The
signature is Ed25519 with the deployment's push signing key (`SAGAT_PUSH_SIGNING_KEY`, key id configured next to
it; retired public keys stay published until their `notAfter`). The envelope binds tenant, namespace, endpoint
and task. **One key signs every tenant's envelopes, and any tenant can aim an endpoint at any public URL**, so a
signature alone proves only that the platform sent it — not that it was meant for this function. Protection against
another tenant's envelopes therefore rests on the verifier checking the expected identity below.

**Verifier rules** (all five SDKs, `verifyPushEnvelope`, shared vectors in
`tests/contract/sdk-conformance/scenarios/push-envelope.json`, new): reject an unknown key id, a timestamp outside
±300 s of the verifier's clock, a `task.deadline` already passed, a signature that does not verify over the raw body
(never a re-serialized one), an envelope `version` other than 1, and a tenant id, namespace id or endpoint name other than the **required**
`expected` argument (`verifyPushEnvelope(rawBody, headers, keys, expected{tenantId, namespaceId, endpoint}, now)`;
there is no overload without it). The vectors include a validly signed envelope of another tenant and of another
endpoint of the same namespace, both of which MUST be rejected. Unknown body properties are ignored (the envelope
only grows).

What the envelope never contains: provider credentials, secret values (steps that declare secrets are not
push-eligible), blob bytes or descriptors (blob-bearing steps are not push-eligible), OpenBao routing, platform
keys or another attempt's token. `taskToken` is present only when the endpoint mode is `async` and the node declares
`asyncCompletion`. It is the attempt's capability: together with an authenticated platform identity holding
`task:complete` it lets the function heartbeat and complete *this* attempt and nothing else (D1057, D1058). A
function in `async` mode therefore needs its own machine identity (OIDC client credentials, like any worker); the
platform never sends one. Whether a token alone should suffice is owner question O1041.

## Response

| Response | Meaning | Step effect |
|---|---|---|
| `200`/`201`/`204` with `{"output": <json>}` (or empty for no value) | success | `CompleteStep` under the lease, key `push-<taskId>` |
| `2xx` with `{"error": {"code", "retryable", "safeMessage"}}` | function failure | `FailStep` with that code (free text becomes `WORKER_ERROR`; reserved `OUTCOME_UNKNOWN` keeps its meaning) |
| `202` | accepted for later completion | detach (as `DetachStep`) if endpoint mode is `async` and the node declares `asyncCompletion`; otherwise non-retryable `PUSH_ASYNC_NOT_ALLOWED`. The function may complete through REST **before** the invoker records the `202` (fast functions, Lambda `Event`): such attempts accept external reports while push-held, first commit wins, and the invoker's later `202` handling finds the step ended and records nothing |
| Lambda `200` with `X-Amz-Function-Error` | function error | retryable `PUSH_FUNCTION_ERROR` within the step's retry policy |
| `429`, `502`, `503`, `504`, Lambda `TooManyRequestsException` | overloaded | retryable `PUSH_UNAVAILABLE`, `Retry-After` / `retryAfterSeconds` honoured as the earliest next attempt |
| other `4xx` | refused | non-retryable `PUSH_REJECTED` |
| `3xx` | redirect | not followed; non-retryable `PUSH_REJECTED` |
| timeout or connection broken **after** the request was written | unknown | `OUTCOME_UNKNOWN` (reconcile with `reconcileWith`, otherwise manual review for `highRisk`) |
| connect failure **before** sending, DNS failure | not delivered | retryable `PUSH_UNAVAILABLE` |
| destination refused by the egress guard | denied | `PUSH_DESTINATION_DENIED`, endpoint health `DENIED`, audited once per window |
| body larger than 256 KiB or not JSON | invalid | non-retryable `PUSH_RESPONSE_INVALID`; the bytes are discarded, never logged |

Lambda `Event` invocation type returns `202` and requires mode `async`: the function completes with the token and
its own platform identity. Lambda's own asynchronous retries can arrive after the ±300 s window and are then
rejected by the verifier (G1058); set the function's `MaximumRetryAttempts` to 0 and rely on the platform's attempt
retries, which carry the same idempotency key.

**Credentials.** The invoker resolves a revision's provider credential after the claim commit, as that revision's
secret binding: the secret must exist and its access list must still allow the revision's `boundBy` principal, who
must still be a member holding `push:manage` (the outbound broker-subscription binding of ADR-0025 is the
precedent). Otherwise the attempt fails retryable `PUSH_CREDENTIAL_UNAVAILABLE` and the endpoint reports
`FAILING`/`CREDENTIAL_UNAVAILABLE`. The credential is dropped when the invocation ends.
The output is scanned like any worker output; the response body never appears in logs, spans or audit.
