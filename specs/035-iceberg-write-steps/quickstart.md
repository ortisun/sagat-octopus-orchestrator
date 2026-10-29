# Quickstart and validation guide — 035 Iceberg writes and step-output destinations

How to prove the feature end to end once it is built. Nothing here runs today; commands name the test
targets and files the tasks will create. Contracts: [contracts/](contracts/); model:
[data-model.md](data-model.md).

## Prerequisites

- Docker (or the Colima profile used for sagat tests), at least 8 GiB of memory for the lake overlay.
- A private `CARGO_TARGET_DIR` outside the repository and `df -h` checked first (AGENTS.md rule 19).
- `infra/docker/compose.lake.yaml` (new): RustFS (S3), `apache/iceberg-rest-fixture:1.10.1`, Lakekeeper
  (REST with vended credentials), Hive standalone metastore 4.2.1, PostgreSQL (JDBC catalog and the
  PostgreSQL destination), moto (Glue), fake-gcs-server, Azurite (blob endpoint), ClickHouse 26.8,
  NATS and Kafka from the existing files; loopback-only ports, healthchecks, pinned tags (D035-041).

```sh
docker compose -f infra/docker/compose.yaml -f infra/docker/compose.database.yaml \
  -f infra/docker/compose.lake.yaml up -d --wait
```

## Scenario 1 — connection and probe (US01, SC-035-001)

1. Create a namespace secret and an Iceberg connection `lake` (REST catalog
   `http://iceberg-rest:8181`, storage `s3` at the RustFS endpoint, `warehouseLocations: ["s3://lake/"]`,
   `allowedModes: [append, overwrite, upsert]`, `egressCeiling: internal`).
2. `POST /namespaces/dev/connections/lake/probe` → every check `PASSED`, with the tested revision.
3. Reload the connection: no credential field appears. Expected failure variants: a wrong secret gives
   `CATALOG_AUTH` `FAILED` with a typed code; a `warehouseLocations` that excludes the table's location
   gives `ICEBERG_LOCATION_NOT_ALLOWED` on the first write.

Automated: `cargo test -p sagat-tests --test integration_iceberg_writes connections::`.

## Scenario 2 — append, crash, no duplicate (US02, SC-035-002, SC-035-003)

1. Create `sales.orders` with the fixture's admin client (the platform never creates tables).
2. Publish `examples/iceberg/orders-append.workflow.json`: a script step writes 10,000 records to a
   Parquet blob; an `icebergWrite` step appends them.
3. Run it; the receipt shows `COMMITTED`, `rowsAdded: "10000"`, a snapshot id and the effect key.
4. Read the table with the independent reader (iceberg-rust 0.10.1, a test-only dependency of
   `tests/`): exactly 10,000 rows; the snapshot summary holds `sagat.effect-key`.
5. Fault run: `cargo test -p sagat-tests --test fault_iceberg_writes` kills the host or the runner
   before the commit request, after it and before the response, and after the response before
   `CompleteStep`, plus a stale lease racing its successor (held with a barrier, never a sleep). Every
   run ends with one commit carrying the key, or `OUTCOME_UNKNOWN` when the test expires snapshots to
   make the lookup inconclusive.

## Scenario 3 — overwrite and upsert (US04, US05, SC-035-002, SC-035-004)

1. `examples/iceberg/daily-overwrite.workflow.json` overwrites partition `day = 2026-10-26` twice:
   only the second batch remains and other partitions are unchanged.
2. `examples/iceberg/customers-upsert.workflow.json` upserts 3 existing and 2 new customers and marks
   customer 42 deleted: one commit; one live row per key; 42 is gone.
3. Concurrency: `cargo test -p sagat-tests --test fault_iceberg_writes concurrency::` runs 2–8 writers
   (platform runs and a direct PyIceberg writer) and compares the table with a serial replay.

## Scenario 4 — the checkout demo (US07, US08, SC-035-013, SC-035-014)

1. Publish `examples/iceberg/checkout.workflow.json`: *finish payment* declares synchronous
   destinations `lake` (Iceberg append), `finance` (PostgreSQL upsert by `payment_id`),
   `archive` (object storage, Parquet), `analytics` (ClickHouse) and `bus` (Kafka subscription with
   purpose `STEP_OUTPUT` and `egressCeiling: internal`); *send receipt* follows. The Iceberg, PostgreSQL,
   object-storage and ClickHouse destinations are published by an admin (`database:write`, D035-066);
   the broker destination needs `event:manage`.
2. Run it: the run graph shows five `finish payment~d.<name>` steps; *send receipt* starts after all
   five succeeded; each destination holds the payment once; Kafka holds one message with a stable id.
3. Stop the REST catalog and run again: the `lake` destination exhausts its retries and the run waits
   (`hold`); restart the catalog, `RETRY_STEP` on `finish payment~d.lake`; the payment step is never
   re-executed.
4. With 020's `ICEBERG` sink (when 020 is built), the same payment reaches the lake asynchronously and
   the run never waits for it.

Automated: `cargo test -p sagat-tests --test e2e_step_destinations -- --ignored checkout`.

## Scenario 5 — security and classification (SC-035-006, SC-035-007)

- `cargo test -p sagat-tests --test integration_security_multitenancy` with the new battery entries:
  another tenant's connection, browse and destination are `404`; denials audited.
- T-isolation: from inside a `lake` sandbox container, reading the runner's token file, another
  attempt's directory, the pod network, the metadata service and the engine socket all fail.
- Relay and hostile-catalog canaries: the container's only route is the authenticated relay socket; a
  catalog that returns `py-io-impl`, endpoint, proxy or signer overrides, or a table location pointing
  at `169.254.169.254` or the platform database, is refused and the denial audited.
- Classification: publishing a destination that maps a `pii` field to a connection with ceiling
  `internal` fails `CLASSIFICATION_EGRESS_REFUSED`; credential canaries never appear in logs, spans,
  audit, receipts or API responses.

## Scenario 6 — five SDKs and the Studio (SC-035-005)

```sh
SAGAT_SDK_SCENARIO=iceberg-writes cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
SAGAT_SDK_SCENARIO=step-destinations cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture
```

Both scenarios publish the same definition from every SDK and compare canonical IR, receipts and
errors; the Studio journey builds the same steps (`frontend/e2e`).

## Gates before a pull request

`cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`python3 scripts/check-architecture.py && python3 scripts/check-agent-config.py`, the feature targets
above, `SAGAT_TEST_AS_RUNTIME_ROLE=1` over them, the per-SDK lints of `sdks/AGENTS.md`, `pip-audit` on
`runners/lake/requirements.lock`, and `(cd frontend/apps/studio && npm run api && npm run check:api)`.
Report skipped items (Glue conflicts, ADLS DFS, real cloud accounts) as gaps, not passes.
