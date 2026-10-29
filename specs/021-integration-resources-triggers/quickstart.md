# Quickstart — feature 021 demo (planned)

Status: **planned**. Nothing below runs today: the routes, binaries, overlay and test targets are
created by the tasks named in each step. When the feature is implemented, this file becomes the
reproducible happy/failure demonstration required by the Definition of Done, with the actual
outputs and timings recorded next to each step (SC-021-018).

## Prerequisites (to be delivered)

- The pinned Rust toolchain, Docker, Node 22.
- `infra/docker/compose.database.yaml` (017: PostgreSQL, persistent TLS OpenBao, demo database) plus
  a new development-only overlay `infra/docker/compose.integrations.yaml` (T021-065) adding the
  integration runner, MinIO, Mosquitto, an SMTP test server, a local HTTP/OAuth2 test server and the
  Pub/Sub emulator. Bootstrap material is generated into an ignored directory; nothing is shared
  with another stack.
- A demo tenant and namespace `demo` with an administrator token from the development identity
  provider (`apps/dev-idp`), and an egress policy allowing the overlay's test hosts.

## Happy path

1. **Secret and resource (US01).** Create the OAuth2 client secret through the 017 secret API
   (`PUT /v1/namespaces/{id}/secrets/billing-client`, OpenBao provider), then:

   ```http
   PUT /v1/namespaces/{id}/resources/billing-api
   {"requestId":"<uuid>","expectedRevision":0,"type":"http_api","typeVersion":1,"status":"ACTIVE",
    "settings":{"baseUrl":"https://billing.test:8443","auth":{"scheme":"oauth2_client_credentials",
      "tokenUrl":"https://billing.test:8443/oauth/token","clientId":"demo","scopes":["invoices"]}},
    "slots":{"clientSecret":"billing-client"}}
   ```

   Expect `200` with revision 1 and no secret value. `POST …/resources/billing-api/probe` returns a
   run; it ends `SUCCEEDED`, and `GET …/probe` shows revision 1.
2. **Variable (US07).** `PUT /v1/namespaces/{id}/variables/region` with `{"valueType":"string",
   "level":"public","value":"eu-west-1",…}`.
3. **Workflow (US02).** Publish (Studio or any SDK) a workflow whose `resourceOperation` node is the
   example in [contracts/resource-operation.schema.json](contracts/resource-operation.schema.json),
   followed by `smtp.send` on resource `mailer` and an `end`.
4. **Route (US03).** Create a `ROUTE` endpoint `POST /orders/{orderId}/paid` (HMAC profile, event type
   `order.paid`) and a `webhook` trigger naming it. Call
   `POST /v1/routes/{prefix}/orders/42/paid` with `Idempotency-Key: k1` and a valid signature.
   Expect `202`; the run starts, the test server records one POST `/v1/regions/eu-west-1/invoices`
   with `Idempotency-Key` equal to the step's effect key and a bearer token, and the SMTP server
   receives one message whose `Message-ID` derives from the second step's effect key.
5. **Observe.** The run graph shows `resourceOperation` nodes without URLs or values; the audit shows
   `resource.changed`, `resource.probe_admitted`, `secret.resolved` and `run.started`; metrics show
   `resource_operations_total{type="http_api",operation="http.request",outcome="succeeded"}`.
6. **CDC and MQTT (US04, US06).** Create the demo publication and slot with the runbook SQL; add a
   `POSTGRES_CDC` connector and an MQTT connector; insert a row and publish a message; each starts
   one run.

## Failure scenario

1. **Crash after send (US02, rule 9).** Run the fault target (T021-028), which stops the
   integration runner after the request reaches the test server and before `CompleteStep`. Expect:
   the attempt's lease expires; with `idempotency.header` the retry sends the same key and body and
   the step completes once; with the declaration removed, the run enters `MANUAL_REVIEW` with
   `OUTCOME_UNKNOWN` and the server recorded exactly one request.
2. **Unknown SMTP outcome.** The SMTP test server drops the connection after the end-of-data marker.
   Expect `OUTCOME_UNKNOWN`, `MANUAL_REVIEW`, no automatic resend.
3. **CDC crash between commit and flush report (T021-040).** Expect the transaction to be resent and
   recognised by the inbox: one run per row, `connector_messages_total{kind="POSTGRES_CDC",outcome="duplicate"}`
   increases.
4. **Egress refusal.** Point a resource at `https://metadata.test/`, a name the overlay's DNS resolves
   to `169.254.169.254`. The save succeeds (names are resolved only when connecting); the probe fails
   `RESOURCE_DESTINATION_DENIED` with an `egress.denied` audit and no connection is made.
5. **Tenant boundary.** With a token of another tenant, `GET …/resources/billing-api` returns `404`.

## Test targets (to be created, T021-003)

```sh
cargo test -p sagat-tests --test integration_integration_resources --locked
cargo test -p sagat-tests --test contract_rest_integration_resources --locked
cargo test -p sagat-tests --test fault_integration_resources --locked
cargo test -p sagat-tests --test e2e_integration_resources --locked
cargo test -p sagat-tests --test sdk_conformance --locked
python3 scripts/check-architecture.py
npm --prefix frontend run test
```

Integration targets need Docker. Real Slack, Service Bus, Pub/Sub and AWS runs need credentials and
are recorded as skipped, with the reason, until supplied.
