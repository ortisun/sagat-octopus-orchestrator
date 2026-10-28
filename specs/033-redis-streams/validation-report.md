# Feature033 validation — 2026-10-27

Implementation checkpoint; release0.3 acceptance still needs the integrated tree and the staged
released-source cold-rollout proof. No mixed-version/transport rolling safety or arbitrary Redis
failover durability is claimed.

Commands used the owned external CARGO_TARGET_DIR, debug/incremental disabled, one build job and
RUST_TEST_THREADS=2. PostgreSQL/Redis fixtures are disposable Testcontainers; runtime integration
runs set SAGAT_TEST_AS_RUNTIME_ROLE=1 and use sagat_app. Named gate transcripts are in the owning
session's /tmp/sagat-v03-20261026 directory; results below are actual command output summaries.

| Gate | Command / scope | Actual result |
| --- | --- | --- |
| redis-final-broker-regressions | cargo test -p sagat-tests --test integration_redis_streams --test integration_events_webhooks_messaging --locked --no-fail-fast | Redis28 passed/0 failed; existing events111 passed/0 failed |
| redis-process-configs | cargo test -p sagat-control-plane -p sagat-execution-engine --test config --locked --no-fail-fast |36 control-plane and27 engine configuration tests passed |
| redis-domain-properties | cargo test -p sagat-domain --test outbound_redis_properties --locked |2 passed, including generated IPv6/TLS/DB round trips |
| redis-shared-regressions | AsyncAPI, REST events, Helm scalability and developer-reference contracts |4 +2 +25 +8 passed; existing events111 also passed |
| redis-final-workspace-clippy | cargo clippy --workspace --all-targets --locked -- -D warnings | exit0,143.24s |
| redis-final-fmt | cargo fmt --all --check | exit0,4.61s |
| architecture / agent configuration | python3 scripts/check-architecture.py; python3 scripts/check-agent-config.py |24 workspace crates, layers respected/no cycles; agent configuration OK |
| architecture negative controls | python3 -m unittest discover -s scripts/tests -p test_check_architecture.py |8 passed, including Redis forbidden in the domain |
| Studio contract | npm run api; npm run check:api; npm run typecheck; npm run lint | all exit0; generated schema matches canonical OpenAPI |
| frontend advisory check | npm audit --audit-level=moderate |0 vulnerabilities |
| Compose | docker compose --env-file infra/docker/.env.example -p sagat-redis-033-proof-20261027 -f infra/docker/compose.yaml up -d --no-deps --wait --wait-timeout 45 redis (SAGAT_REDIS_PORT=0) | actual service Healthy; CONFIG GET showed appendonly=yes / appendfsync=everysec; published127.0.0.1:32769 |
| Compose cleanup | same private project, down --volumes | owned container/network/redisdata volume removed |

## What the Redis cases prove

- Committed PG outbox → actual Redis XADD → PG confirmation; actual engine selects Redis despite an
  invalid inactive NATS endpoint, starts while Redis is paused, then publishes after recovery.
- Binary payload/subject/stable message identity; exact MAXLEN; owned AOF fixture restart.
  Docker reallocated the test fixture's host port after stop/start: the test explicitly rediscovers
  that routing address. It does not claim recovery through an unchanged stale mapped port.
- Public subscription management + canonical response schema + actual gRPC workflow completion →
  real delivery worker → Redis and DELIVERED. Only networking uses a declared standing fixture,
  as production policy intentionally cannot allow a test container's loopback address.
- Actual control-plane/engine boot without a secret backend: anonymous subscription creation works;
  the delivery loop runs and still refuses production loopback egress. Secrets remain optional.
- Current namespace secret resolution, real private-CA TLS, hostname verification, ACL password
  rotation, concurrent namespace/database separation, plaintext/wrong-type secret rejection.
- One checked DNS resolution/candidate per attempt; resolver outage is NETWORK, policy refusal is
  DENIED. A nonresolving logical hostname connects only through the explicitly pinned fixture IP.
- Timeout closes its owned connection; malformed ACK is refused. A real TCP proxy consumed an actual
  Redis ACK and dropped it, then forwarded the retry on the same configured endpoint: two physical
  entries retain the same delivery id and body.
- Actual crash after XADD before PG settlement and a stale publisher race retain one durable
  settlement/attempt record; physical repeats preserve identity. Outage→DEAD→redrive and WRONGTYPE
  recovery reuse existing PG accounting; callback publishing preserves pre-existing customer entries.
- Additive destination migration down refuses retained Redis rows; empty down/up works.

## Failures observed and corrected

Genuine RED controls covered initial unsupported transport selection, invalid stream ACK acceptance,
internal validation/error-code conflation, malformed IPv6 normalization, anonymous boot wiring,
plaintext secret-reference management, missing content type, DNS socket fan-out, duplicate checked
resolution and resolver-outage misclassification. Each was followed by its named passing gate.

Setup/test corrections were kept distinct: missing thiserror dependency and initial Redis setter /
DeliveryWorker constructor API mistakes were compilation failures, not behavioral evidence. The AOF
restart fixture initially waited on Docker's stale mapped port; rediscovery fixed that fixture. The
stale-worker test initially assumed a different DeliveryRound classification; durable PG row/attempt
assertions now prove the actual fence. Clippy found ordinary style/enum-size/test-placement issues;
all were corrected before the final clean workspace gate. No production failure was retried away.

## Review and remaining acceptance

The internal review found three actionable issues and the callback review found five (the latter
across two cycles); all were reconciled with code and RED/green controls. The callback's third/final
read-only review found no new actionable production issue. Review claims were followed by the root's
actual merged-working-tree gates above. The owner previously selected single-model review only.

The actual released-source callback reader/writer stop/join → explicit activation → new Redis
row proof now passes on the locally integrated 014/033/034 tree. Six released profiles, 139 real
messaging/Redis regressions, 109 Rust SDK tests and five new common scenarios across all six SDKs
pass; see [released-callback-cutover.md](released-callback-cutover.md) and its original evidence.
Final release CI and remaining 014 nested-capacity/fault/resource acceptance remain open.
T033-007 is not closed by these focused gates.
