# Resource types v1 and managed operations (draft contract)

Draft for FR-021-001, FR-021-003, FR-021-010..017. Merged into canonical documents by T021-060. Not
implemented. Revised 2026-10-27 after the architecture and security reviews.

## Common rules

- A type version is immutable. Settings schemas are closed (`additionalProperties: false`), bounded
  and contain no secret value; a string setting that parses as a URL with userinfo, or a header
  setting named `authorization`, `cookie` or `proxy-authorization`, is refused (`details.reason =
  CREDENTIAL_IN_SETTINGS`).
- Each type version names, per operation, the managed executor role allowed to resolve it
  (`integration-runner` for every v1 operation below; 025 and 024 may add operations resolved by
  their own runners, D1154).
- **Host-bearing settings** are `baseUrl`, `oauth2_client_credentials.tokenUrl` (`http_api`,
  `graphql`), `endpoint` and `bucket` (`s3_bucket`), `host` (`smtp`) and `apiBaseUrl` (`slack_bot`).
  A revision that creates the resource, changes a host-bearing setting or changes a slot's secret
  requires `secret:manage` on every affected slot secret (`SLOT_DESTINATION_REQUIRES_MANAGE`);
  `secret:use` suffices for other revisions (D1158).
- Header names authored anywhere (`api_key_header.headerName`, `defaultHeaders`, operation
  `headers`) are checked against the refused list of `http.request`; `captureHeaders` refuses
  `Set-Cookie`, `Set-Cookie2`, `WWW-Authenticate`, `Proxy-Authenticate` and `Authorization`.
- Slots name namespace secrets by logical name. The runner receives the slot value only through
  `ResolveResource` ([integration-runner.md](integration-runner.md)).
- Every connection passes the guarded resolver: resolve once, check every address, connect to a
  checked address, verify TLS against the configured host name, no redirects, no environment proxy.
- Each operation is **safe** (no change at the provider) or **effect**. The class is fixed here.
- Every operation output is a JSON envelope; the node's `outputSchema` may classify it (undeclared
  fields are `pii`). The runner scans the envelope for every resolved slot value in raw,
  JSON-escaped, base64 (three alignments), base64url and percent-encoded forms before reporting.
- Blob outputs (`s3.getObject`, HTTP `response.mode: blob`) pass the bounded cross-chunk streaming
  scanner before the blob is `UPLOADED`; the server rescans before `COMMITTED`. Until that scanner's
  tests pass (T021-076) publication refuses them (`BLOB_MODE_UNAVAILABLE`). This refines ADR-0023's
  refusal of uploads from secret-bearing steps for managed integration steps only.

## `http_api` (version 1)

| Settings | Bounds |
|---|---|
| `baseUrl` | `https` URL without userinfo, query or fragment; ≤ 2,048 bytes; `http` only in development |
| `auth` | `none`, `bearer` (slot `token`), `basic` (slot `credentials`: username/password), `api_key_header` (`headerName` + slot `apiKey`), `oauth2_client_credentials` (`tokenUrl` https, `clientId`, slot `clientSecret`, `scopes[]` ≤ 16, `audience?`, `clientAuth`: `basic` or `body`) |
| `defaultHeaders` | ≤ 16 non-auth headers |
| `tls` | `verifyFull` with optional CA PEM (≤ 64 KiB, no private key) |
| `probe` | `{method: GET|HEAD, path, expectStatus: [..] }`, default `HEAD /` expecting any status below 500 |
| `limits` | per-resource concurrency and deadlines within the plan ceilings |

OAuth2: one token request per attempt (`grant_type=client_credentials`), response ≤ 16 KiB,
`access_token` and `token_type` `Bearer` required, token never logged, cached or reported.

**`http.request`** — safe for `GET`/`HEAD`, effect otherwise.

| Field | Rule |
|---|---|
| `method` | `GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE` |
| `path` | template beginning with `/`; literal segments `[A-Za-z0-9._~!$&'()*+,;=:@-]`; parameters `{name}` evaluated and percent-encoded as one segment; no `..`, no `//`, no absolute URL |
| `query` | map name → value source; names ≤ 128 bytes |
| `headers` | map name → value source; refused: `Authorization`, `Proxy-*`, `Host`, `Content-Length`, `Transfer-Encoding`, `Connection`, `Upgrade`, `TE`, `Trailer`, `Keep-Alive`, and `Cookie` |
| `body` | one of `{json: valueSource}`, `{text: valueSource, contentType}`, `{blob: valueSource}` (a `$blob` descriptor) |
| `response` | `mode`: `json` (default), `text`, `blob`, `none`; `successStatuses` (default 200–299); `maxBytes`; `captureHeaders` ≤ 16 names |
| `idempotency` | `{header: "Idempotency-Key"}` sends the effect key on every attempt |

Result: `{status, headers{captured}, contentType, sizeBytes, body | blob}`. A status outside
`successStatuses` fails: `401` → `AUTHENTICATION`, `403` → `PROVIDER_FORBIDDEN`, `408`/`429` →
`PROVIDER_RATE_LIMITED` (retryable, honours `Retry-After` ≤ the deadline), other `4xx` →
`PROVIDER_REJECTED`, `5xx` → `PROVIDER_UNAVAILABLE`. A `3xx` is `PROVIDER_REJECTED`. For an effect
without idempotency, `PROVIDER_UNAVAILABLE` after the request was written is `OUTCOME_UNKNOWN`.

## `graphql` (version 1)

Settings as `http_api` with `endpointPath` instead of probe; probe sends `{ __typename }`.

**`graphql.execute`** — `kind: query` is safe, `kind: mutation` is effect. Fields: `document`
(≤ 64 KiB, static), `operationName?`, `variables` (map → value source), `idempotency`, `response`
(`json` only, bounds). The runner parses the document; the selected operation's type must equal
`kind`, otherwise `OPERATION_MISMATCH` before sending. Subscriptions are refused. Result:
`{status, data, errors[{message?, path?, extensions?}]}` bounded; `errors` with `data: null` →
`GRAPHQL_ERRORS` (not retryable by default).

## `s3_bucket` (version 1)

The client is a `CustomerBucket` built by `crates/infra-blobstore` over `object_store` with a
guarded `HttpConnector` (ADR-0023 decision 1 kept). Settings: `endpoint` (https, optional for AWS), `region`, `bucket`, `addressing` (`path`,
`virtualHosted`), `keyPrefix?` (every key is under it), slot `accessKey` (access key id + secret,
optional session token). Probe: `HeadBucket`.

| Operation | Class | Rule |
|---|---|---|
| `s3.getObject` | safe | `key`; streams into a platform blob through `WriteBlob` under the lease; result `{blob, etag, versionId?, contentType, sizeBytes, lastModified}`; over the blob ceiling → `RESULT_LIMIT` |
| `s3.headObject` | safe | `key`; result metadata only |
| `s3.listObjects` | safe | `prefix`, `continuationToken?`, `maxKeys`; result keys and sizes, bounded |
| `s3.putObject` | effect | `key`, body `{blob}` or bounded `{json}`/`{text}`; sends `x-amz-checksum-sha256`; optional `ifNoneMatch: "*"`; built-in reconciliation: `HEAD` and compare the stored SHA-256 checksum (applied on match, not applied on 404, unknown otherwise) |
| `s3.deleteObject` | effect | `key`, optional `versionId`; S3 delete of a missing key is success; irreversible on unversioned buckets |

Keys are evaluated, ≤ 1,024 bytes UTF-8, no control characters, and joined to `keyPrefix`.
A managed lease reads only the blob ids its server-evaluated operation names.

## `smtp` (version 1)

Settings: `host`, `port`, `tls` (`implicit` on 465 or `starttls` required), `auth` (`none` or
`login`/`plain` with slot `credentials`), `allowedFrom[]` (≤ 16 addresses or `@domain`),
`messageIdDomain`. Probe: connect, `EHLO`, TLS, `AUTH`, `QUIT`; no `MAIL FROM`.

**`smtp.send`** — effect, irreversible. Fields `from` (must match `allowedFrom`), `to[]`, `cc[]`,
`bcc[]`, `replyTo?`, `subject`, `text?`, `html?`, `attachments[{blob, filename, contentType}]`.
Every header value refuses CR, LF and NUL; addresses are parsed (RFC 5322 addr-spec). `Message-ID`
is `<{effect key}@{messageIdDomain}>`. A `5xx` reply before `DATA` completes → `PROVIDER_REJECTED`;
`4xx` before `DATA` → retryable; a broken connection after the end-of-data marker and before the
reply → `OUTCOME_UNKNOWN`. Result `{messageId, acceptedRecipients, rejectedRecipients}`.

## `slack_bot` (version 1)

Settings: `apiBaseUrl` (default `https://slack.com/api`, fixed host), slot `botToken`. Probe:
`auth.test`.

**`slack.postMessage`** — effect, irreversible. Fields `channel` (id), `text?`, `blocks?`
(bounded JSON array), `threadTs?`, `unfurl?: false`. HTTP `429` → `PROVIDER_RATE_LIMITED` with
`Retry-After`; `ok: false` with a closed set of Slack error names mapped to `PROVIDER_REJECTED` or
`AUTHENTICATION`; any response lost after the request was written → `OUTCOME_UNKNOWN`. Result
`{channel, ts}`.

## Database drivers (listed, not managed here)

`postgresql` (017) and `mysql`, `mariadb`, `sqlserver`, `bigquery`, `snowflake`, `duckdb` (024,
`specs/024-data-engines-pipelines/contracts/engines.md`) appear in `GET …/resource-types` as
`{key, managedBy: "connections"}` with a link to the connection API. `PUT …/resources/{name}` with
any of them fails `422` with `MANAGED_BY_CONNECTIONS`. A driver added later to the connection catalog
joins this list in the same delivery.
