# REST routes (design 047)

Tenant routes are under `/v1` of the tenant API; operator routes are on the platform-operator surface of Architecture
decision 0074 (Proposed). Until it is accepted, deployment registrations and offers are refused, as Technical Plan 6 of
the design says, and only `hub unblock` exists, as a schema-owner command of `control-plane`. Every tenant route
answers `404` alike for another tenant's object and a missing one, and audits the attempt. Bodies are bounded at 64 KiB.
Errors are in `errors.md`.

| Method and path | Permission | Purpose |
|---|---|---|
| `GET /oauth/providers`, `GET /oauth/providers/{id}/versions/{version}` | `resource:read` | list and read the tenant's providers |
| `POST /oauth/providers` | `oauth:client:manage` | author a provider (a draft version) |
| `POST /oauth/providers/{id}/versions/{version}:publish` | `oauth:client:manage` for an authored provider, `hub:publish` for an imported one | publish after the write-time checks |
| `POST /oauth/registrations`, `GET /oauth/registrations` | `oauth:client:manage` | tenant registrations; the secret or key is write-only |
| `POST /oauth/registrations/{id}:rotate-secret` | `oauth:client:manage` | a new secret revision; clears `BROKEN` |
| `POST /oauth/registrations/{id}:retire` | `oauth:client:manage` | stop new connections; credentials keep refreshing until disconnected |
| `POST /oauth/offers/{offer}:opt-in`, `:opt-out` | `oauth:client:manage` | a tenant's answer to an operator offer |
| `POST /namespaces/{ns}/oauth/credentials` | `oauth:credential:manage` | create a credential (`PENDING`, or `ACTIVE` without a token for client credentials) |
| `GET /namespaces/{ns}/oauth/credentials`, `GET .../{id}` | `resource:read` | state, granted scopes, expiry, outcome; the account label to holders of the namespace's personal-data reveal only |
| `POST /namespaces/{ns}/oauth/authorizations` | `oauth:credential:manage` | body `{credential, scopes, returnTo}`; answer `{authorizationUrl, expiresAt}` |
| `POST /oauth/callback` | the person who started it | body `{registration, code, state, iss}`; answer the credential's state |
| `POST /namespaces/{ns}/oauth/credentials/{id}:disconnect` | `oauth:credential:manage` | move to `DISCONNECTING` |
| Feature 028's `GET /template-catalog/entries` and `GET .../{id}` | `definition:read` | browse, with the new kinds |
| Feature 028's `POST /namespaces/{ns}/template-imports` | feature 028's (`definition:write` or `script:write`) | import workflow and script drafts |
| `POST /template-imports` (tenant-wide) | `hub:import`, plus feature 028's permission for every workflow or script part | import providers, resource types and integrations; body `{entry, version, namespace?, requestKey}`; answer the drafts and what to prepare |
| `POST /hub/resource-types/{key}/versions/{version}:publish` | `hub:publish` | publish an imported resource type |

Operator acts (platform-operator surface, once Architecture decision 0074 is accepted; before it, only `hub unblock`, as a
`control-plane` command):
`oauth deployment-provider import`, `oauth deployment-registration create|rotate-secret|retire`, `oauth offer
create|withdraw` (`oauth:offer:manage`), `hub unblock <digest>` (`hub:version:block`).

Default holders: `oauth:client:manage`, `oauth:credential:manage` and `hub:publish` belong to Admin; `hub:import` to
Developer and Admin; the operator permissions to the platform operator only.
