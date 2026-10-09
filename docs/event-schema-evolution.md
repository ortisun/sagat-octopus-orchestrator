# Evolution of the messages the platform sends and receives

Applies to every message in [`events.asyncapi.yaml`](../specs/001-platform-foundation/contracts/events.asyncapi.yaml): the hints and events on NATS, the webhooks the platform sends (`RunFinishedWebhook`), the ones it receives (`WebhookEvent`), the document a run or a waiting step is given (`EventDocument`) and the envelope read from brokers (`BrokerMessage`). It is the policy `FR-004-007` asks for. The document's `info.version` and the checks below are how the policy is kept.

## The rule

Inside one version of the document a message **only grows**:

1. A property is never removed, renamed or given another type — widening included (`string` to `[string, null]` is another type): the checker compares the declared types exactly.
2. A new property is **optional**. `required` never gains a member.
3. An enumeration gains values and never loses them. A consumer must treat a value it does not know as "something new happened", not as an error (`run.finished` carried `SUCCEEDED, FAILED, CANCELLED, TIMED_OUT` first; feature 003 added `COMPENSATED`, `PARTIAL` and `COMPENSATION_FAILED` this way).
4. The meaning of a property does not change. If it has to, it is a new property.
5. Headers follow the same rule as the payload.

A change that cannot be made that way is a **new message with a new name**, published next to the old one (for example `RunFinishedWebhookV2`), and the old one is kept for the deprecation period below.

## What a producer and a consumer do

- **The platform as producer** (outgoing webhooks, NATS events) adds fields without warning and never removes one. Every outgoing webhook carries `X-Octopus-Schema-Version`: the version of the *shape of the payload*, today `1`. It changes only when a new message name replaces the old one.
- **A receiver of the platform's webhooks** must ignore properties it does not know, accept enumeration values it does not know, and verify the signature over the raw body (never over a re-serialised one — adding a property changes the bytes).
- **A provider that sends events to the platform** may send any JSON object; the platform does not validate the *data* against a schema (the data belong to the provider). The envelope headers are the contract. A new optional header is added the same way as a new property.
- **Workflows** read the event document (`event` and `data`) with expressions. Adding a property to `event` is safe; removing or renaming one would break published workflows, which is why it is not done inside a version.

## How it is checked

- `tests/contract/004-events-webhooks-messaging/asyncapi/compatibility.rs` compares the current shape of every message with the previous one recorded in [`events.baseline.json`](../specs/001-platform-foundation/contracts/events.baseline.json) and fails on: a removed property, a changed type, a removed enumeration value, a property that became required, a message that disappeared. It also fails when a message is **missing from the baseline**, so the next change is judged against it. The checker itself is tested against each kind of break.
- `conformance.rs` checks that what the platform really sends (a NATS hint, a signed webhook of a finished run, the document a trigger gives a run) validates against the message in the document, and that what the document promises about headers is what the code does.
- A change that is meant to grow a message regenerates the baseline on purpose (`SAGAT_UPDATE_BASELINE=1 cargo test -p sagat-tests --test contract_asyncapi_004`), and the diff of the baseline is what a reviewer reads.

## Versions and deprecation

- `info.version` of the AsyncAPI document follows semantic versioning: a patch for wording, a minor for anything that grows a message or adds one, a major only when a message is replaced under a new name and the old one is finally removed.
- A message that is replaced is marked `deprecated` in the document with the date, and both are sent for at least **six months**. For outgoing webhooks, how a subscription would choose between the two (its `events` list names the kinds it wants) is decided when the first such replacement exists; none does yet. The removal is announced one minor version earlier.
- The REST API follows its own rule (`/v1`, additive within a version); the worker protocol follows [`worker-runtime-protocol.md`](architecture/worker-runtime-protocol.md).

## Not covered

- The data a provider sends: the platform stores and passes it on as it is.
- Ordering between messages: none is promised (see the runbook).
- Compatibility of the *payload bytes* with a consumer's own schema registry: the platform does not publish to a registry; this is a gap if outgoing events ever go to Kafka (G77 in `specs/004-events-webhooks-messaging/research.md`).
