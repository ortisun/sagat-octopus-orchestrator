# REST routes (design 047)

Under `/v1`. Every route answers `404` alike for another tenant's object and a missing one, and audits the attempt.
Bodies are bounded (64 KiB unless stated). Errors are in `errors.md`.

| Method and path | Permission | Purpose |
|---|---|---|
| `GET /oauth/providers`, `GET /oauth/providers/{id}/versions/{version}` | `resource:read` | list and read providers (tenant and published hub ones) |
| `POST /oauth/providers` | `oauth:client:manage` | create an authored provider (a draft version) |
| `POST /oauth/providers/{id}/versions/{version}:publish` | `oauth:client:manage` for authored, `hub:publish` for hub | publish after the write-time checks |
| `POST /oauth/registrations`, `GET /oauth/registrations`, `POST /oauth/registrations/{id}:retire` | `oauth:client:manage` | tenant registrations; the secret or key is write-only (rotation is a new registration) |
| `POST /oauth/offers/{registration}:opt-in`, `:opt-out` | `oauth:client:manage` | a tenant's answer to an operator offer |
| `POST /namespaces/{ns}/oauth/credentials` | `oauth:credential:manage` | create a credential (`PENDING`, or `ACTIVE` for client credentials) |
| `GET /namespaces/{ns}/oauth/credentials`, `GET .../{id}` | `resource:read` | state, granted scopes, account label (to holders of `pii` reveal only), expiry, outcome |
| `POST /namespaces/{ns}/oauth/authorizations` | `oauth:credential:manage` | start a connection; body `{credential, registration, scopes, returnTo}`; answer `{authorizationUrl, expiresAt}` |
| `POST /oauth/callback` | the person who started it | body `{registration, code, state, iss}`; answer the credential's state |
| `POST /namespaces/{ns}/oauth/credentials/{id}:disconnect` | `oauth:credential:manage` | move to `DISCONNECTING` |
| `GET /hub/items`, `GET /hub/items/{id}/versions/{version}` | `definition:read` | browse (feature 028's route, more kinds) |
| `POST /hub/imports` | `hub:import` | body `{item, version, namespace?, requestKey}`; answer the drafts and what to prepare |
| `POST /hub/resource-types/{key}/versions/{version}:publish` | `hub:publish` | publish an imported resource type |
| `POST /admin/oauth/registrations`, `POST /admin/oauth/offers`, `:withdraw` | `oauth:offer:manage` (platform operator, Architecture decision 0074) | deployment registrations and offers |
| `POST /admin/hub/blocks`, `DELETE /admin/hub/blocks/{digest}` | `hub:version:block` (platform operator) | block or lift an item digest |

Default holders: `oauth:client:manage`, `oauth:credential:manage` and `hub:publish` belong to Admin; `hub:import` to
Developer and Admin; the operator permissions to the platform operator only.
