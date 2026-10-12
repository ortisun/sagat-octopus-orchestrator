# Validation quickstart: Triggers, webhooks, messaging and correlation

Everything below was run against this repository. The pieces that are **not** built are listed at the end.

## Prerequisites

- The Rust toolchain pinned by `rust-toolchain.toml` and Docker (the suites start throw-away containers through Testcontainers: PostgreSQL 16 for all of them, and Apache Kafka, ElasticMQ (an SQS-compatible queue) and NATS JetStream for the connector stories; no port is published on the host).
- Node 22 (`npx`) only for the contract linters in CI.

## Validation flow

1. **The user stories, the contracts and the failures**, against real PostgreSQL, real brokers, a fake OIDC provider and real HTTP receivers:

   ```sh
   cargo test -p sagat-tests --test e2e_events_webhooks_messaging            # US01–US04 through REST, the public webhook route, the engine as a process, three real brokers
   cargo test -p sagat-tests --test integration_events_webhooks_messaging    # ingress, dispatch, cron, outbound deliveries, the sender, connectors (84 tests)
   cargo test -p sagat-tests --test contract_rest_events_webhooks_messaging  # every event operation against the OpenAPI document, normal and error paths
   cargo test -p sagat-tests --test contract_asyncapi_events_webhooks_messaging  # the AsyncAPI document: complete, only grown since the baseline, and what is really sent conforms to it
   cargo test -p sagat-tests --test fault_events_webhooks_messaging          # dispatchers cut off at every await, racing dispatchers, a worker that died after sending, connections killed under load
   cargo test -p sagat-domain -p sagat-application -p sagat-infra-ir -p sagat-infra-egress -p sagat-infra-connectors -p sagat-infra-postgres -p sagat-scheduler
   ```

2. **Receiving an event by hand** (the recipe, with a test vector for an implementation, is in [`docs/runbooks/events.md`](../../docs/runbooks/events.md) section 1): store the secret, create the endpoint, sign `octopus-event-in-v1\n{timestamp}\n{id}\n{type}\n{subject}\n{key}\n` and the body, `POST /v1/webhooks/{id}`.

3. **Failure injection** (`tests/fault/events-webhooks-messaging/`, plus the failure tests of the integration suite). Each states an invariant that must hold whatever was cut off:

   | Class | Test | What it does |
   |---|---|---|
   | Crash at a critical point | `fault dispatch::a_dispatcher_cut_off_at_any_await_leaves_nothing_started_twice_and_nothing_lost` | A dispatcher's round is dropped at every await in turn (before the claim, between the run and its delivery row, before the mark) and run again: each event ends with one run per trigger and one wake per wait, none lost. |
   | Restart / lease | `fault dispatch::an_event_claimed_by_a_dispatcher_that_died_is_taken_over_when_its_lease_ends`, `integration dispatch::a_claimed_event_is_not_claimed_by_another_dispatcher_until_its_lease_ends` | A claimed event is invisible to others until its lease ends, then taken over. |
   | Duplicates | `dispatch::dispatching_an_event_again_after_a_crash_starts_nothing_new`, `ingress::a_signed_webhook_is_admitted_once_and_a_retry_is_a_duplicate`, `ingress::the_deduplication_window_ends_and_the_same_id_is_a_new_event`, `dispatch::an_id_admitted_again_after_its_window_is_a_new_event_and_starts_its_run_again` | The same event twice is one event; the same id after its window is a new one that starts its run again, once. |
   | Race, event against timeout | `dispatch::the_event_and_the_timeout_racing_resolve_to_exactly_one_of_them`, `fault dispatch::the_event_the_timeout_and_a_second_dispatcher_race_and_exactly_one_outcome_wins_every_time` | Exactly one of the event and the timeout takes effect, with a second dispatcher in the race too. |
   | Race, dispatchers | `fault dispatch::dispatchers_racing_for_the_same_events_start_each_run_once`, `cron::two_dispatchers_that_find_a_trigger_due_together_start_one_run` | Several dispatchers over one inbox, several over one schedule. |
   | Event before the wait | `dispatch::an_event_that_came_before_the_wait_is_delivered_only_within_its_lookback`, `an_event_that_arrives_while_the_matcher_is_looking_is_not_lost`, `us02_an_event_that_came_just_before_the_wait_is_delivered_when_the_wait_asks_to_look_back` | An event that arrives before, or while, a wait is registered is found from the wait's side. |
   | Valid signature, repeated id | `ingress::an_event_id_reused_for_another_payload_is_a_conflict_and_the_first_is_kept`, `ingress::what_the_sender_says_the_event_is_is_covered_by_the_signature` | A repeated nonce (the event id) is a duplicate or a conflict; changing a header the signature covers is a bad signature. |
   | Lost broker delivery / lost offset commit | `us03_kafka_redelivery_poison_messages_and_a_lost_offset_commit_leave_one_run_per_event`, `us03_sqs_messages_are_deleted_only_after_they_are_in_the_inbox`, `us03_nats_messages_are_acknowledged_after_the_event_is_in_the_inbox` | Redelivered, out-of-order and poison messages; the position moves in the transaction of the event; nothing is acknowledged before the event is in the inbox. |
   | Receiver that accepts then drops | `fault deliveries::a_worker_that_died_after_sending_sends_again_with_the_same_delivery_id` | At-least-once, with a stable delivery id. |
   | A worker that was too slow | `outbound::a_worker_whose_lease_ran_out_cannot_undo_what_another_one_recorded` | A late record changes nothing. |
   | Failed side | `outbound::a_delivery_that_fails_every_attempt_is_set_aside_and_a_redrive_gives_it_a_full_set_again`, `a_subscription_whose_deliveries_keep_dying_is_switched_off_and_turning_it_on_redrives` | The compensation of an outbound delivery that never works: set aside, switched off, redriven. |
   | Connections killed | `fault ingress::connections_killed_under_load_lose_no_accepted_event_and_admit_none_twice` | Providers retrying while backends are terminated: every event answered `202` is in the inbox, none twice. |
   | The database fails while verifying | `ingress::a_database_failure_while_verifying_is_never_a_verdict_on_the_request` | A correct request is told to try again, not that it is unauthenticated. |

4. **The benchmark** (ingress over HTTP, dispatcher scaling, the cost of waiting, fan-out and rate of deliveries, connector ingest):

   ```sh
   SAGAT_PERF_REPORT=tests/performance/reports/events-webhooks-messaging.md \
     cargo test -p sagat-tests --test perf_events_webhooks_messaging --release -- --ignored --nocapture
   ```

5. **Operating it:** [`docs/runbooks/events.md`](../../docs/runbooks/events.md).

## Story evidence

| Story | Acceptance scenario | Evidence |
|---|---|---|
| US01 Start a workflow via webhook | An endpoint with HMAC configured; a provider sends a valid event; the inbox confirms 202 and a run is created exactly once as a logical admission | `us01::us01_a_signed_webhook_starts_a_workflow_exactly_once` (the whole path over REST and the public route: secret, endpoint, trigger, signed post, a duplicate, a refusal, one run); `ingress::*` (21 tests: signatures, skew, rotation, reuse, rate, size, backlog, paused, tenant scoping, audit); `dispatch::an_event_starts_the_run_of_a_matching_trigger_once_and_gives_it_the_event`, `a_filter_decides_which_events_start_a_run`, `a_webhook_trigger_is_about_its_endpoint_and_an_event_trigger_about_a_type`, `the_triggers_in_force_are_those_of_the_active_version`; `cron::*` (9) |
| US02 Wake a suspended run on an event | A persisted wait; the correlated event arrives; the scheduler releases the step once and stores the evidence | `us02::us02_the_correlated_event_releases_the_waiting_step_once_and_the_evidence_is_kept`, `us02_an_event_that_came_just_before_the_wait_is_delivered_when_the_wait_asks_to_look_back`; `dispatch::an_event_wakes_the_step_that_waits_for_it_and_gives_it_the_event`, `an_event_goes_to_every_wait_with_its_key_once_each`, `an_event_with_more_waits_than_a_page_reaches_all_of_them_in_one_dispatch`, `a_wait_that_timed_out_is_not_woken_by_a_late_event_and_the_event_is_not_recorded_as_delivered` |
| US03 Integrate Kafka and SQS | An authenticated adapter; messages are redelivered and out of order; inbox dedupe, DLQ and offset checkpoints work | `us03_kafka_redelivery_poison_messages_and_a_lost_offset_commit_leave_one_run_per_event`, `us03_sqs_messages_are_deleted_only_after_they_are_in_the_inbox` (credentials as a secret, a full backlog leaves the messages in the queue), `us03_nats_messages_are_acknowledged_after_the_event_is_in_the_inbox`, `us03_kafka_a_paced_batch_longer_than_the_lease_keeps_renewing_it`, `us03_sqs_batches_far_larger_than_a_webhook_answer_are_read_and_not_a_wedge`; `connectors::*` (10) |
| US04 Deliver an outbound webhook | A completed run event; the endpoint answers 503 twice; the system retries with backoff and records every attempt | `us04::us04_a_finished_run_is_delivered_after_the_receiver_comes_back_and_every_attempt_is_recorded` (REST, a real receiver, the delivery worker, three attempts listed with their statuses); `outbound::*` (19 tests: signing, rotation, backoff, Retry-After, set aside, redrive, switch-off, rate, tenant scoping, secrets that are gone, answers that are pages); `sender::*` (the guarded client) |

## Requirement evidence

| Requirement | Evidence |
|---|---|
| FR-004-001 manual/API, webhook, event, cron, message and signal triggers | triggers registered at publish (`dispatch::the_triggers_in_force_are_those_of_the_active_version`, `crates/domain/tests/workflow.rs`), the `api` source (`ingress::publishing_through_the_api_…`), `cron::*`, `us01`, `us03`; direct signals are feature 002's |
| FR-004-002 one inbox per tenant/source/event_id with configurable deduplication TTL | the primary key of `inbox_events`; `ingress::the_same_source_and_event_id_in_two_namespaces_are_two_events`, `the_deduplication_window_ends_and_the_same_id_is_a_new_event`, `an_event_published_through_the_api_may_have_a_short_window_and_not_a_silly_one`; the window rules in `crates/domain/src/events.rs` |
| FR-004-003 event_type+correlation subscriptions with timeout and atomic competition between event and timer | `dispatch::*` (wake, lookback, race with the timeout), `fault_events_webhooks_messaging` |
| FR-004-004 HMAC/timestamp/nonce on webhooks; OIDC/mTLS per profile | HMAC, timestamp window and nonce (the event id) built and tested (`ingress::*`, `crates/application/src/signing.rs` unit tests with independent vectors); OIDC/mTLS **not built** (G75) |
| FR-004-005 inbound/outbound adapters for NATS, Kafka and SQS with ack/offset after persistence | inbound built for all three (`us03`, `connectors::*`); outbound producers **not built** (G77) |
| FR-004-006 outbound HTTP webhook with signing, retry jitter, TLS, rate limit, DLQ, redrive, endpoint status | `outbound::*`, `us04`, `sender::*`; TLS is the scheme of the URL and the egress policy's (`http` only when the namespace allows it) |
| FR-004-007 AsyncAPI versioned for messages and a schema evolution policy | `events.asyncapi.yaml` 0.2.0, `events.baseline.json`, `asyncapi/*` (4 tests), `docs/event-schema-evolution.md` |
| FR-004-008 backpressure, size limits, retention, safe filters and quotas per tenant/connector | the 10 000-event backlog (`ingress::a_full_backlog_refuses_new_events_and_still_answers_duplicates`, `connectors::a_full_backlog_leaves_the_message_with_the_broker_…`), the body limit, windows and purge (`dispatch::expired_events_are_purged_but_one_still_waiting_…`), expression limits (feature 002's evaluator), quotas (`ingress::endpoints_are_scoped_to_their_tenant_and_to_a_namespace_quota`, `connectors::a_connector_is_administered_…`) |
| NFR-004-SEC | tenant on every statement and composite foreign keys of every new table; `dispatch::events_never_cross_a_tenant_boundary`; the uniform 401; no secret, signature, payload, URL path or receiver body in any record (`outbound::what_the_receiver_says_is_never_stored`, `us04::a_reader_sees_…`, `telemetry::*`); two independent reviews (D94) |
| NFR-004-DUR | `fault_events_webhooks_messaging`, the integration crash tests, the transactional boundaries of D84/D88/D90 |
| NFR-004-OBS | `e2e_events_webhooks_messaging telemetry::events_webhooks_and_dispatches_are_counted_without_naming_anyone`; counters, gauges and histograms listed in the runbook; audit records |
| NFR-004-COMP | additive, reversible migration; AsyncAPI only grown; OpenAPI additive; the workflow schema gained optional fields |
| NFR-004-PERF | `tests/performance/reports/events-webhooks-messaging.md` |

| Success criterion | State |
|---|---|
| SC-004-001 signed webhooks, duplicates, retries, out-of-order and event/timeout race in integration tests | Met: `integration_events_webhooks_messaging`, `fault_events_webhooks_messaging` |
| SC-004-002 100% of US01..US04 pass in a reproducible automated suite | Met for the scenarios of the specification (`e2e_events_webhooks_messaging`, 11 tests, with real brokers) |
| SC-004-003 no violation of the Constitution gates and no silent loss of persisted state | Partial: no loss is provable by the tests above; only principle II has an automated gate (architecture check); the constitution was ratified on 2026-10-11 |
| SC-004-004 critical errors expose `code`, `trace_id` and remediation without leaking | Partial, as in 003 and 009: `code`, `traceId` and a stable `details.reason` are proven; no separate `remediation` field |

## The review (T004-035)

Two reviewers who had no part in writing it read the uncommitted change, one for security and tenant isolation and one for correctness and crash safety. Together they reported 37 findings, 33 distinct (four were found by both). Each was checked against the code before anything was changed. The result and what was not changed, and why, is D94 in [`research.md`](./research.md); the gaps they added are G87–G94. The guards of the feature were then mutation-checked (78 mutations, 70 killed at once, 8 survivors answered with tests, an existing test or the removal of an equivalent expression); the second round of those checks was **not** run, because mutation testing is paused by the maintainer's decision until the most important features are done.

## What is not built

The list, with the reason for each, is G73–G95 in [`research.md`](./research.md): OIDC and mTLS profiles for webhooks (G75), producers to Kafka, SQS and NATS (G77), a shared or per-address ingress limit (G78), trace propagation from an event into a run (G79), a retained-bytes quota and field-level redaction for the inbox (G80), per-trigger ordering policies (G74), a `source` on waits (G87), fairness across tenants in the dispatcher (G88), SDK helpers (G86), Studio screens and dashboards (G85), use-time re-checks of secret bindings (G90).
