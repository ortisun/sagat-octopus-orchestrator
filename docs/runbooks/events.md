# Runbook: events, webhooks and messaging (feature 004)

For the people who connect the platform to the outside world: providers that post events, brokers that carry them, and receivers that want to hear what the platform did. It covers how to set each of them up, how to sign and verify, what the platform does when something is down, and what to look at. Design: [`docs/adr/ADR-0012-events-pass-through-a-durable-inbox-and-delivery-table.md`](../adr/ADR-0012-events-pass-through-a-durable-inbox-and-delivery-table.md); decisions and gaps: [`specs/004-events-webhooks-messaging/research.md`](../../specs/004-events-webhooks-messaging/research.md); the message contracts: [`events.asyncapi.yaml`](../../specs/001-platform-foundation/contracts/events.asyncapi.yaml) and [`docs/event-schema-evolution.md`](../event-schema-evolution.md); the engine's own runbook: [`execution-engine.md`](execution-engine.md).

## 0. What is promised, and what is not

- **An event the platform answered `202` for is durable**: it is a row in the inbox, committed. It is dispatched to every trigger and every waiting step it matches; each effect (a run started, a step woken) happens **once** per event, however the dispatcher is interrupted or repeated.
- **Events are deduplicated** by `(tenant, namespace, source, event id)` for the window of the endpoint or connector (24 hours by default; for a webhook never less than twice the signature skew and a minute). After the window the same id is a new event and starts its runs again. A repeat of the same body is answered `202` with `status: DUPLICATE`; the same id with a different body is `409 EVENT_ID_REUSED`.
- **Nothing is ordered.** Two events of one correlation key can be dispatched in either order. A workflow that needs order carries a sequence number in the event and checks it.
- **Outgoing webhooks and broker subscriptions are at-least-once.** A receiver can see the same delivery twice (the worker died after the receiver or broker acknowledged it, or the acknowledgement was lost). Every attempt carries the same delivery id; use it to drop repeats.
- **A message is acknowledged to its broker only after the event is in the inbox.** A crash in between means the broker delivers it again, and the inbox recognises it.
- The platform does **not** promise exactly-once *delivery* to or from anything outside it.

## 1. Receiving events from a provider (signed webhook)

### 1.1 Set up

1. Store the shared secret in the namespace (any value up to 16 KiB; **at least 16 bytes are required** — a shorter secret verifies nothing — and 32 random characters are recommended, such as `whsec-` and 32 of them): `PUT /v1/namespaces/{ns}/secrets/provider-signing` with `{"value": "…"}`. Needs `secret:manage`; the secret is sealed with the key ring (`SAGAT_SECRETS_KEYS`) and never shown again.
2. Create the endpoint: `POST /v1/namespaces/{ns}/event-endpoints` with `{"slug": "billing", "secretName": "provider-signing"}`. Optional: `defaultEventType` (used when the provider sends no type), `dedupeTtlSeconds`, `skewSeconds` (default 300), `maxBodyBytes` (at most 240 KiB), `ratePerSecond`, `status` (`ACTIVE` or `PAUSED`). Needs `event:manage` and permission to use the secret. The response has `webhookPath`: `/v1/webhooks/{id}`. The id is random and is the only thing in the URL; give the provider the full URL.
3. Tell the provider how to sign (1.2). Give it nothing else.

The slug names the **source** of the events (`event.source` in the workflow, `source` in the triggers). It must be unique among the endpoints and connectors of a namespace; `api` and `cron` are reserved.

### 1.2 Signing

The signature is `v1=` and the lower-case hex HMAC-SHA256, under the secret, of these bytes:

```text
octopus-event-in-v1\n{timestamp}\n{event id}\n{event type}\n{subject}\n{correlation key}\n{body}
```

The type, the subject and the correlation key are the values you send in the headers below, **as you send them**: an absent one is the empty string, and the endpoint's default type is not something you signed (a request that names no type signs an empty one). The signature therefore covers what decides which workflows start and which waits wake, as well as the payload, and a proxy that changes a header changes the signature. None of these values can contain a newline.

```sh
SECRET='whsec-…'; HOST=https://platform.example.com; ENDPOINT=<id>
body='{"invoice":"INV-1","amount":150}'
ts=$(date +%s); id="evt-$(uuidgen)"; type=invoice.paid; subject=''; key=INV-1
sig=$(printf 'octopus-event-in-v1\n%s\n%s\n%s\n%s\n%s\n%s' "$ts" "$id" "$type" "$subject" "$key" "$body" \
      | openssl dgst -sha256 -hmac "$SECRET" -hex | sed 's/^.* //')
curl -sS -X POST "$HOST/v1/webhooks/$ENDPOINT" \
  -H "X-Octopus-Signature: v1=$sig" -H "X-Octopus-Timestamp: $ts" \
  -H "X-Octopus-Event-Id: $id" -H "X-Octopus-Event-Type: $type" \
  -H "X-Octopus-Correlation-Key: $key" -H 'Content-Type: application/json' \
  --data-binary "$body"
```

(Leave out `X-Octopus-Subject` when the subject is empty, and send exactly the values that were signed.) A test vector to check an implementation against: key `a-secret-of-some-length`, timestamp `1700000000`, id `evt_1`, type `invoice.paid`, subject `subject/1`, correlation key `ord-7`, body `{}` gives `v1=552996eb858bb3ef71451dbeeb116324e3094fcb4c711cb56cf759d3e061eaec`.

- The body is a JSON object of at most 240 KiB (or the endpoint's lower limit), sent **exactly** as signed.
- The timestamp is seconds since the epoch and must be within the skew of the platform's clock in either direction. Keep clocks in sync (NTP); a provider whose clock is minutes off gets `401`.
- `X-Octopus-Event-Id` is the provider's own id for the event. Retrying the same event **must** reuse it.
- A header may carry up to four signatures (comma separated, at most 512 bytes in all); more is refused before anything is computed.
- Optional: `X-Octopus-Event-Type` (required unless the endpoint has a default), `X-Octopus-Subject`, `X-Octopus-Correlation-Key` (what waiting steps match on).

### 1.3 Answers

| Status | Meaning | The provider should |
|---|---|---|
| `202` `ACCEPTED` | The event is durable. | Stop. |
| `202` `DUPLICATE` | It was already admitted (a retry, or a replay within the window). | Stop. |
| `401` | **One answer for everything**: unknown endpoint, missing or malformed header, bad or stale signature, no usable secret. Deliberately says nothing more. | Check the secret, the clock and the signed bytes. The operator sees the reason in the audit trail and the `webhook_rejected_total{reason}` counter. |
| `409` `EVENT_ID_REUSED` | The id was used for a different body inside the window. | Use a new id for a new event. |
| `413` | The body is over the endpoint's limit. | Send less. |
| `422` | Verified, but not an event: the body is not a JSON object, or the type is missing, or a header is not usable. | Fix the event. |
| `429` + `Retry-After` | Over the endpoint's rate, or the platform's backlog of undispatched events for the tenant is full. | Wait that long and retry the same event with the same id. |
| `503` | The endpoint is `PAUSED`. | Retry later. |
| `5xx` | The platform could not decide (a database error while verifying counts: it is never a verdict on the request). | Retry. |

### 1.4 Rotating the secret

1. Put the new value under the **same name** (`PUT …/secrets/provider-signing`): it becomes the newest version.
2. The platform tries the **three newest versions**, so requests signed with the old value keep verifying. Move the provider to the new value at leisure; a provider may even send both signatures, comma separated (`v1=aaa…,v1=bbb…`).
3. A version older than the third newest is no longer tried. **Putting the secret twice more is therefore not a way to revoke a leaked one while it is still among the three newest; if a secret leaked, delete it** (`DELETE …/secrets/provider-signing`: events then get `401` and the platform logs an error until you create it again with a new value) **and create it anew.**

### 1.5 Pausing and removing

`PUT` the endpoint with `"status": "PAUSED"` to answer `503` without losing anything already admitted. `DELETE` removes the endpoint (the URL then answers `401`); events already in the inbox are still dispatched.

## 2. What an event does

- **Triggers.** A workflow lists triggers in its definition: `"triggers": [{"kind": "webhook", "endpoint": "billing", "eventType": "invoice.paid", "filter": "data.amount > 100"}, {"kind": "message", "connector": "orders"}, {"kind": "event", "source": "api", "eventType": "order.created"}, {"kind": "cron", "cron": "0 2 * * *", "timezone": "Europe/Lisbon"}]`. `webhook` names the slug of an endpoint, `message` the slug of a connector, `event` matches an event of a type from any source (or from the `source` given), `manual` is a person or the API starting it, `cron` is a schedule (five fields). They are registered when a version is published and follow the workflow's latest published version. Each matching event starts **one** run per trigger; the input of the run is `{"event": {id, source, type, subject, correlationKey, occurredAt, receivedAt}, "data": <body>}`. A `filter` (a side-effect-free condition over `event` and `data`) that is false, or cannot be evaluated on the event, starts nothing.
- **Waiting steps.** A `waitEvent` node waits for an event type and a correlation key (`correlationExpr`), with a timeout. The first matching event wakes it, once; the step's output is the event document, and the delivery is recorded as evidence. `lookbackMs` lets a wait also take an event that arrived up to that long **before** the wait began (events that come early are not lost), but only as far back as the event is still remembered: the window of its source (24 hours for a connector and by default for an endpoint; an endpoint's `dedupeTtlSeconds` goes up to 30 days). A longer look-back than the window finds nothing older than the window. Waits are matched on type and correlation key from **any** source, including `api`: anyone with `event:publish` can wake one, as anyone with `run:operate` can already signal the run. A timeout and an event that arrive together decide by who commits first.
- **Cron.** Evaluated by the dispatcher with the trigger's time zone. A tick starts a run under a key derived from the tick, so two replicas starting the same tick start one run. **Ticks missed while the platform was down are coalesced into one** (a platform that was off for an hour fires once on return), and a start that fails is retried after 30 seconds.
- **Publishing from your own code** needs no signature: `POST /v1/events` with a bearer token (`event:publish`) creates an event with source `api`. The window (`dedupeTtlSeconds`) is between a minute and 30 days, 24 hours by default. **Who published an event is in the audit trail** (`event.published`, once per event admitted, resource `api/{id}`), because the runs it starts are attributed to the engine and the event itself is purged with its window.

Look at what became of an event: `GET /v1/namespaces/{ns}/events/{source}/{eventId}` (`event:read`) gives its status (`PENDING`, `DISPATCHED`, `DEAD`), attempts, last error (never the payload) and the deliveries — which run each trigger started, which step each wait woke.

## 3. Reading messages from Kafka, SQS and NATS (connectors)

A connector is read by the engine (`execution-engine`): **every replica runs a reader for every active connector**, and the replicas share the work through the broker — a NATS durable consumer and an SQS queue hand each message to one of them, and every Kafka partition is read by whoever holds its lease. The `ratePerSecond` of a connector is therefore a limit **per replica**. Create one with `POST /v1/namespaces/{ns}/connectors`:

| `kind` | `settings` | Credentials secret (JSON object, optional) |
|---|---|---|
| `NATS` | `{"servers": ["tls://nats.example.com:4222"], "stream": "ORDERS", "consumer": "sagat-orders", "filterSubject": "orders.>"}` (the JetStream stream must exist; the platform creates the durable pull consumer) | `{"creds": "<.creds file>"}`, or `{"token": "…"}`, or `{"user": "…", "password": "…"}` |
| `KAFKA` | `{"servers": ["tls://kafka.example.com:9093"], "topic": "orders", "startFrom": "earliest"}` | `{"username": "…", "password": "…", "mechanism": "PLAIN" \| "SCRAM-SHA-256" \| "SCRAM-SHA-512"}` |
| `SQS` | `{"queueUrl": "https://sqs.eu-west-1.amazonaws.com/123/orders", "region": "eu-west-1"}` | `{"accessKeyId": "…", "secretAccessKey": "…", "sessionToken": "…"}` |

Common fields: `slug` (the source), `secretName` (the credentials), `defaultEventType` (for messages that name none), `ratePerSecond` (ingest pace, up to 10 000, 200 by default), `status`.

**The message.** The body must be a JSON object (the event data) of at most 240 KiB. The envelope travels in Kafka/NATS headers or SQS message attributes: `eventId`, `eventType`, `subject`, `correlationKey`, `occurredAt`. Without `eventId` the broker's own identity of the message is used (topic/partition/offset, stream sequence or `Nats-Msg-Id`, SQS message id), so a redelivery is recognised either way. Without `eventType` and without a `defaultEventType` the message is set aside.

**What the connector guarantees.** (SQS: a message that is left in the queue — the backlog is full, the database is down — is received again after its visibility timeout, and the queue counts every receive; the reader waits 5 s, then 10, 20 and 30 s between reads while that goes on, but a queue with a redrive policy of two or three receives will still send such messages to its dead-letter queue. Give the queue a `maxReceiveCount` that outlasts the outages you expect.) The event and the position (Kafka offset) are written in one transaction, and the message is acknowledged (NATS ack, SQS delete, Kafka position moved) only after that. A lost acknowledgement means a redelivery that the inbox answers as a duplicate. A message that cannot become an event — not JSON, not an object, too big, no type, an id reused for another body — is **set aside** as a dead letter in the same transaction and does not hold up what follows. If the tenant's backlog of undispatched events is full the connector stops taking messages and leaves them with the broker.

**Operating it.**

- `GET …/connectors/{id}` shows `health`: `lastError`, `lastMessageAt`, counts consumed and dead-lettered. Counters: `connector_messages_total{kind,outcome}`, `connector_failures_total`, gauge `connectors_running`.
- Dead letters: `GET …/connectors/{id}/dead-letters` (the reason, the broker's position, the first 64 KiB of the message), `…/{deadLetterId}/redrive` puts the message through the connector again (after you fixed the cause, such as the default type), `DELETE …/{deadLetterId}` discards it. At most 1 000 are kept per connector, **the first ones**: once it is full, new poison messages are acknowledged to the broker and counted (`dead_lettered`, and `connector_dead_letters_dropped_total`) but not kept, so look at the ones that are kept, fix the cause and discard them. A message the broker delivers again after it was set aside is not counted again. They hold up to 64 KiB of what was sent until someone discards them.
- **Kafka positions live in PostgreSQL**, not in a consumer group: there is no rebalancing, a partition is read by whoever holds its lease (30 s, renewed), and `startFrom` applies only the first time. If the broker's log no longer reaches the saved position (retention, a recreated topic) the connector restarts from `earliest` and counts `connector_offset_resets_total`.
- Credentials are read when a reader starts; to rotate them, put a new version of the secret and pause and resume the connector (or wait: a reader that fails five calls in a row stops and starts again, which reads them anew; a NATS or Kafka reader that loses its connection does the same). **A password or token is never sent to a server whose address is not `tls://`**: the reader refuses to start, and says so in `health.lastError`. (A NATS `creds` file only signs a challenge and may go either way.) A topic with more than 256 partitions is refused.
- The egress policy of the namespace must allow the bootstrap hosts. **It is checked on the addresses given, not on what the brokers advertise afterwards** (G76): restrict the network of the engine as well.

## 4. Telling other systems what happened (outbound subscriptions)

### 4.1 Webhooks

`POST /v1/namespaces/{ns}/webhook-subscriptions` with `{"name": "billing-hook", "url": "https://hooks.example.com/octopus", "secretName": "hook-signing", "events": ["run.finished"], "ratePerSecond": 20, "timeoutMs": 10000}`. Needs `event:manage`. The URL must be `https` (or `http` if the namespace's egress policy says so), must not carry credentials, and must be allowed by the **namespace's egress policy** (egress policy: [`security-governance.md`](security-governance.md)); loopback and private addresses can never be allowed, redirects are never followed, and the policy is applied again at every attempt.

**What a receiver gets.** `POST` with `Content-Type: application/json`, the body `{"id", "type": "run.finished", "time", "data": {"tenantId", "namespaceId", "runId", "workflow", "status"}}`, and the headers `X-Octopus-Signature` (signed like an inbound webhook; **one signature for each of the two newest versions of the secret**, comma separated; the signed bytes are `octopus-event-out-v1\n{timestamp}\n{event id}\n{event type}\n\n\n{body}` — the same scheme as for events you send us, with its own label so that a delivery can never be passed off as an event, and with the type from `X-Octopus-Event-Type`), `X-Octopus-Timestamp`, `X-Octopus-Event-Id` (the same for every subscription the event goes to), `X-Octopus-Event-Type`, `X-Octopus-Delivery-Id` (the same on every attempt of this delivery), `X-Octopus-Attempt` and `X-Octopus-Schema-Version`. Verify the signature over the raw body, reject timestamps further than your tolerance from your clock, and deduplicate on the delivery id. **The URL is shown in full only to those who can manage events**: to others the path and query (which can be the credential of a chat tool) are replaced by `/…`.

**What the platform does with the answer.**

| Answer | Outcome |
|---|---|
| `2xx` | Delivered. |
| `408`, `425`, `429`, `5xx`, a timeout, a network error | Retried with backoff: about 5 s, 30 s, 2 min, 10 min, 30 min, 1 h, 3 h, 6 h, 12 h, then 24 h (jittered ±20 %), at most 12 attempts, about three days in all. `Retry-After` is honoured (capped at 6 h) and never shortens the schedule. |
| anything else (`4xx`, a `3xx` — redirects are not followed) | The delivery is set aside as `DEAD`. |
| a destination the egress policy refuses | Not sent; the refusal is audited (`egress.denied`) and the delivery is retried like a network error, so a policy that is fixed lets it through. |

The body of an answer is **never read, stored or logged**, however long it is; only the status, the outcome and the time are.

**Rate.** `ratePerSecond` (1 to 100) is enforced in the database, across every worker and replica: a worker is offered a subscription only when its slot has come, and one claim takes at most a quarter of a second's rate (25 at most). A subscription is never sent more than its rate plus one claim in any second.

**Dead deliveries and switching off.**

- `GET …/webhook-subscriptions/{id}/deliveries?status=DEAD` lists them, newest first, with every attempt (status, outcome, time); `…/deliveries/{deliveryId}/redrive` puts one back in the queue, `…/webhook-subscriptions/{id}/redrive` all of them. Attempt numbering continues, so `X-Octopus-Attempt` keeps growing.
- After **20 deliveries in a row** were set aside the platform switches the subscription off (`status: DISABLED`, `health.disabledReason`), so a receiver that is gone for good is not tried for every new event. Nothing new is queued for it. **Setting `status: ACTIVE` turns it back on, forgets the failures and redrives what was dead.** (Setting `ACTIVE` on a subscription that was already active changes nothing about its deliveries.)
- Pause a subscription with `"status": "PAUSED"`: deliveries already queued stay queued, nothing new is queued. Resuming it (`"ACTIVE"` again) changes nothing that was set aside; use the redrive routes for that. Only a subscription **the platform** switched off has its failures forgotten and its dead deliveries put back (up to 1 000; the response says how many).
- A subscription with **10 000 deliveries already waiting** is switched off too (`health.disabledReason`: `too many deliveries were waiting`, audited): it is not keeping up, and the platform does not queue for it without end. Fix the receiver or the rate, then set it `ACTIVE` (what is waiting is still waiting).

**Rotating the signing secret.** Put the new value under the same name. The next attempts carry two signatures (new and previous); receivers that verify against either keep working. Remove the old value from the receivers when they have moved.

### 4.2 NATS, Kafka and SQS

Create a broker subscription with `POST /v1/namespaces/{ns}/broker-subscriptions`. Common fields are `name`, `kind`, `settings`, optional `secretName`, `events`, `ratePerSecond`, `timeoutMs` and `status`. Credentials use the same JSON shapes as inbound connectors. Password and token credentials require TLS addresses.

| `kind` | `settings` | Stable delivery metadata |
|---|---|---|
| `NATS` | `{"servers":["tls://nats.example.com:4222"],"subject":"runs.finished"}` | `Nats-Msg-Id` and `delivery-id` header |
| `KAFKA` | `{"servers":["tls://kafka.example.com:9093"],"topic":"runs-finished"}` | record key and `delivery-id` header |
| `SQS` | `{"queueUrl":"https://sqs.eu-west-1.amazonaws.com/123/runs-finished.fifo","region":"eu-west-1"}` | `delivery-id` message attribute; for FIFO, `MessageDeduplicationId` and a subscription-stable `MessageGroupId` |

Success means a JetStream PubAck, a Kafka produce acknowledgement, or a successful SQS `SendMessage` response. A lost acknowledgement can still cause a repeat, including outside the broker's deduplication window. Kafka partition selection is stable while the topic's partition set is unchanged; changing the partition count can move a delivery key. The same twelve-attempt retry, `DEAD` state, redrive, rate limit, pending-delivery cap and fenced attempt recording used by webhooks apply to brokers. Manage failures through `/broker-subscriptions/{id}/deliveries` and its redrive routes.

The current namespace egress policy is checked for every attempt. NATS and Kafka bootstrap addresses are checked before the native client connects; broker-advertised Kafka addresses remain subject to the network-boundary limitation in G76. SQS uses the guarded HTTP client. Missing or unreadable credentials produce `CREDENTIAL_UNAVAILABLE`, which retries so credential repair can recover the existing delivery. Subscription changes, deletion, disablement and redrive are audited without credential values or payloads.

## 5. What to look at

| Question | Where |
|---|---|
| Are events piling up undispatched? | gauge `inbox_pending_events`; histogram `event_dispatch_lag_seconds` (admission to dispatch). A tenant at the backlog limit (10 000) gets `429` on ingress. |
| Is an event stuck? | `GET …/events/{source}/{eventId}`: `status`, `attempts`, `lastError`. An event that fails to dispatch is retried with backoff (a second, doubling, up to ten minutes) and set aside as `DEAD` after 12 attempts; an event that is only waiting for room (a quota of the tenant, a database that cannot be reached) is retried about every half minute without using its attempts, for a day, and then set aside. Each consumer of an event (a trigger, a waiting step) is tried even when another failed, and a retry repeats only what was not done. The error never contains the payload. A `DEAD` event keeps its payload for inspection until its window ends. |
| Are webhooks being refused? | `webhook_rejected_total{reason}` (`bad_signature`, `stale_timestamp`, `malformed`, `unknown_endpoint`, `no_signing_secret`, `paused`, `too_large`, `attempts`, `rate`) and the `event.rejected` audit records. A rise in `no_signing_secret` means an endpoint's secret was deleted or no key of the ring opens it: it is logged at error level. |
| Do waits wake? | `event_wakeups_total{outcome=woken\|lost_race\|refused}`, `trigger_events_total{outcome}`, `runs_started_total{source="trigger"}`. |
| Are cron ticks late? | `cron_fire_lag_seconds`, `cron_fires_total{outcome}`. |
| Are deliveries going out? | `outbound_pending_deliveries`; fixed-label `outbound_attempt_seconds{destination}`, `outbound_attempts_total{destination,outcome}`, `outbound_delivery_lag_seconds{destination}`, `outbound_deliveries_total{destination,outcome}` and `outbound_subscriptions_switched_off_total{destination}`. Existing `webhook_*` counters remain for compatibility. |
| Are connectors reading? | `connectors_running`, `connector_messages_total`, `connector_failures_total`, `connector_dead_letters_dropped_total`, `connector_consumers_abandoned_total` (a reader that did not stop when told to and was cut off), and `health` on each connector. |

The engine's loops are tuned with `SAGAT_EVENT_POLL_MS` (200), `SAGAT_EVENT_BATCH` (100), `SAGAT_DELIVERY_POLL_MS` (250), `SAGAT_DELIVERY_BATCH` (32), `SAGAT_DELIVERY_RETENTION_DAYS` (14) and `SAGAT_CONNECTOR_REFRESH_SECS` (5). The loops are safe to run in every replica: claims use leases and `FOR UPDATE SKIP LOCKED`, and every effect is recorded in the transaction that makes it.

## 6. Limits and quotas

| What | Limit |
|---|---|
| Event body | 240 KiB (an endpoint may lower it) |
| Event endpoints per tenant | 100 (`event_endpoints`) |
| Webhook subscriptions per tenant | 50 (`webhook_subscriptions`) |
| Broker subscriptions per namespace | 50 (`broker_subscriptions`) |
| Connectors per tenant | 20 (`event_connectors`) |
| Undispatched events per tenant | 10 000, then `429`/backpressure |
| Dedupe window | 24 h by default; at least twice the skew |
| Dead letters per connector | 1 000 (the first ones), 64 KiB each |
| Deliveries waiting per subscription | 10 000, then the subscription is switched off |
| Signatures in a header / header size | 4 / 512 bytes |
| Partitions read per Kafka connector | 256 |
| Permissions | `event:publish`, `event:read`, `event:manage` — Developer: all three; Operator: publish and read; Viewer: read |

Quotas are changed through the quota API of feature 009; capacity per tier is an open question until measured on a production-like environment.

## 7. Known gaps

The honest list, with the reasons, is in [`research.md`](../../specs/004-events-webhooks-messaging/research.md) (G73–G95). The ones that matter to an operator:

- **No ordering** between events (G74).
- **The ingress limiter is in memory, per replica and per endpoint**; there is no per-address or global limit, and traffic that has not proved anything draws on the endpoint's attempt budget, so anyone who knows the URL can make an endpoint answer `429` to its provider for a while (G78). Put a proxy with its own limits in front of `/v1/webhooks`.
- **Broker sockets are not guarded after the first connection**: the egress policy is applied to the bootstrap names and addresses given, not to what a broker advertises afterwards or to a second resolution of the name (G76). Restrict the network of the engine as well.
- **No retained-bytes quota for the inbox** (G80): events published through the API can be kept up to 30 days at up to 240 KiB each, within the per-subject and per-tenant request limits. Watch the size of `inbox_events`.
- **Waits and `event` triggers match type and key from any source** (G87), a look-back reaches only as far as the window of the event's source (G89), and the dispatcher is one queue for all tenants (G88).
- **No SDK helpers** (G86) and no dashboards (G85). Outbound Kafka, SQS and NATS producers close G77; OIDC and mTLS profiles close G75.
- **A binding to a secret is checked when it is made, not each time it is used** (G90), and a revoked secret has to be deleted to stop being tried (G92).

## Webhook identity profiles

The default endpoint profile remains HMAC and requires `secretName`. To use an
external OIDC sender, configure `SAGAT_WEBHOOK_OIDC_PROFILES_FILE` with a bounded
JSON array, for example:

```json
[{"name":"provider","issuer":"https://id.example/","audience":"workflow-events","jwksUri":"https://id.example/jwks.json","requiredTokenType":"at+jwt"}]
```

Only deployment configuration supplies issuer, audience and JWKS URLs; endpoint
authors cannot change trust roots or discover arbitrary URLs. At most 32 profiles
are loaded, the file is at most 256 KiB, redirects are refused and production key
URLs require HTTPS. Each profile has a separate bounded key cache. API tokens still
require a tenant claim; external webhook senders need only their verified subject.
Create/update an endpoint with `authentication: {"kind":"OIDC","profile":"provider",
"subject":"expected-provider-subject"}` and omit `secretName`. A request carries
`Authorization: Bearer ...`, `X-Octopus-Timestamp` and `X-Octopus-Event-Id`.

For mutual TLS, set `SAGAT_HTTP_TLS_CERT`, `SAGAT_HTTP_TLS_KEY` and
`SAGAT_HTTP_TLS_CLIENT_CA` to readable PEM files (each at most 256 KiB). The public
listener serves native HTTPS. Configure `authentication: {"kind":"MTLS",
"certificateSha256":"<64 lowercase hex digits>"}` and omit `secretName`. The pin is
SHA-256 of the DER leaf certificate, not the PEM file. Rotate the endpoint pin in an
audited management update when replacing its certificate. Trust roots and server
certificates reload on process restart. Client certificate validity is rechecked on
every keep-alive request; TLS session resumption is disabled. Forwarded certificate
headers confer no identity. A TLS-terminating proxy must pass through TLS for this
profile; plaintext requests cannot use it.

The three profiles are exclusive: a failed OIDC credential cannot fall back to a
valid HMAC signature or TLS client certificate. The timestamp and ID remain
mandatory, with the same replay window, duplicate 202 and changed-content 409.
OIDC/mTLS authenticate the sender over TLS; they do not sign the event body. Invalid
identities return the uniform webhook 401. Unavailable JWKS returns 503, never an
authentication success. No credential, certificate, payload or provider response is
stored in refusal logs. `http_tls_handshakes_total{outcome}` reports bounded TLS
outcomes; existing event admission and refusal metrics cover all profiles.

Validate profile changes against `contract_rest_events_webhooks_messaging`,
`integration_events_webhooks_messaging` and `sagat-api-http`'s `webhook_oidc` tests.
The real transport contract covers native TLS, spoofed certificate headers, OIDC
without a platform tenant claim, subject mismatch, duplicate/reused event IDs and
cross-tenant management reads.
