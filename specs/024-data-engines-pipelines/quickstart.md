# Quickstart — data engines and pipelines (planned demo)

This is the reproducible demonstration that closes the Definition of Done for feature 024 (SC-024-016,
T024-059). **Nothing here runs today**: the commands, routes and node kinds are specified, not
implemented. When implemented, the demo must run from a clean checkout with only the repository's
own containers (AGENTS.md rule 16); the transcript of a real run is recorded with T024-059.

## What it shows

1. **Happy path**: extract orders from MySQL, transform them with DuckDB into the asset
   `orders-daily`, audit, publish, see lineage and freshness, and start a downstream run from the
   publication event.
2. **Failure path**: a duplicate order makes the `unique(order_id)` audit fail; the snapshot is
   quarantined, the pointer stays on the previous snapshot, and the step fails with
   `DATA_AUDIT_FAILED`.

## Prerequisites (planned)

- Docker and the repository toolchain; `curl` and `jq`.
- `infra/docker/compose.yaml` with the existing feature-017 overlay `infra/docker/compose.database.yaml`
  (TLS OpenBao and a TLS PostgreSQL target) and a planned 024 overlay `infra/docker/compose.data.yaml`
  that adds a pinned MySQL 8.4 demo database (loopback port, seeded `orders` table, restricted
  `demo_reader` login) and the sandbox runner configured with the data runtime image.
- A development namespace (`$NS` = its id) and an administrator access token (`$TOKEN`) from the
  development identity provider, as in `docs/runbooks/developer-experience.md`; `$SAGAT` is the
  control-plane base URL. The current CLI has no connection or asset commands; the demo uses REST.

```sh
docker compose -f infra/docker/compose.yaml -f infra/docker/compose.database.yaml \
  -f infra/docker/compose.data.yaml up -d --wait        # compose.data.yaml: planned 024 overlay
H=(-H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json')
```

## 1. Connection and secret (US01)

```sh
# Credential stored in OpenBao through the 017 facade (requiredProvider: openbao); value is write-only.
curl -sS -X PUT "$SAGAT/v1/namespaces/$NS/secrets/mysql-demo" "${H[@]}" \
  -d @mysql-demo.secret.json        # {"value": "<{username,password} JSON>", "requestId": "...", "expectedVersion": 0, "requiredProvider": "openbao"}
curl -sS -X PUT "$SAGAT/v1/namespaces/$NS/connections/orders-mysql" "${H[@]}" -d '{
  "requestId": "0b8c3f5e-1d2a-4b9e-8f00-0c4c1f0a2b31", "expectedRevision": 0, "status": "ACTIVE",
  "config": {"driver": "mysql", "host": "mysql-demo", "port": 3306, "database": "shop",
             "tls": {"mode": "development-plaintext"}, "secretName": "mysql-demo"}}'
curl -sS -X POST "$SAGAT/v1/namespaces/$NS/connections/orders-mysql/probe" "${H[@]}" \
  -H 'Idempotency-Key: demo-probe-0001' -d '{}'     # durable probe run; poll it until SUCCEEDED
```

The `tls` object follows the canonical 017 `DatabaseTls` schema; `development-plaintext` is accepted
only by a development deployment. `mysql-demo` resolves to a private address on the overlay's
compose network, which the development runner policy lists explicitly; loopback stays refused
(ADR-0035 decision 4). Expected: revision 1, a probe run that succeeds, and no
credential in any response, log or audit record.

## 2. Asset declaration (US03)

`orders-daily.asset.json`:

```json
{
  "requestId": "6f1d6c1e-6f43-4f7e-9f5b-2b7b4c9f0a11",
  "expectedRevision": 0,
  "status": "ACTIVE",
  "declaration": {
    "kind": "MANAGED",
    "contract": {
      "columns": [
        {"name": "order_id", "type": "int64", "nullable": false, "classification": "internal"},
        {"name": "order_date", "type": "date", "nullable": false, "classification": "internal"},
        {"name": "amount", "type": "decimal", "precision": 18, "scale": 2, "classification": "internal"}
      ],
      "key": ["order_id"],
      "compatibility": "additive"
    },
    "producer": {"workflow": "orders-pipeline", "node": "transform"},
    "audits": [{"name": "unique_order_id", "kind": "unique", "columns": ["order_id"]},
               {"name": "not_empty", "kind": "rowCount", "minRows": "1"}],
    "freshness": {"maxLagSeconds": 3600}
  }
}
```

```sh
curl -sS -X PUT "$SAGAT/v1/namespaces/$NS/data-assets/orders-daily" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d @orders-daily.asset.json
```

## 3. Pipeline workflow (US03)

`orders-pipeline.json` (canonical IR; any of the five SDK builders produces the same document):

```json
{
  "apiVersion": "sagat-octopus-orchestrator.io/v1",
  "kind": "Workflow",
  "metadata": {"name": "orders-pipeline"},
  "entrypoint": "extract",
  "nodes": [
    {"id": "extract", "type": "database", "connectionRef": {"name": "orders-mysql"},
     "operation": "query",
     "statement": "SELECT id AS order_id, CAST(created_at AS DATE) AS order_date, amount FROM orders WHERE created_at >= ?",
     "parameters": [{"type": "timestamp", "expression": "input.since"}],
     "next": "transform"},
    {"id": "transform", "type": "dataTransform", "engine": "duckdb",
     "inputs": [{"alias": "orders", "step": "extract"}],
     "outputs": [{"asset": {"name": "orders-daily"},
                  "statement": "SELECT CAST(order_id AS BIGINT) AS order_id, order_date, CAST(amount AS DECIMAL(18,2)) AS amount FROM orders",
                  "publish": "auto"}],
     "next": "done"},
    {"id": "done", "type": "end"}
  ]
}
```

Publish it through the existing definition routes (`POST /v1/workflows`, then
`POST /v1/workflows/{workflowId}/publish` with the namespace header) and start a run with
`POST /v1/runs` and input `{"since": "2026-10-01T00:00:00Z"}`; follow the run until it is terminal.

Expected:

- The run succeeds; `GET …/data-assets/orders-daily` shows `pointerVersion: 1` and a
  `currentSnapshotId`; the snapshot lists both audits `PASSED`, its row count and its provenance
  (`orders` ← step `extract`).
- `GET …/data-assets/orders-daily/freshness` answers `FRESH`.
- `GET …/data-lineage?focus=orders-daily` shows `orders-mysql` (source) → `orders-pipeline/transform`
  (producer) → `orders-daily`, as a graph and as the Studio table.
- A workflow with an `event` trigger (`eventType: data.asset.published`, `source: sagat.data`,
  `filter: event.subject == "orders-daily"`) starts exactly once for that publication.

The casts matter: a staged database result exposes integers as `HUGEINT` and decimals with their
retained scale, and the engine refuses any output whose Parquet schema differs from the contract
(`DATA_CONTRACT_VIOLATION`).

## 4. Failure path (US03)

The 024 overlay's fixture script inserts one order whose `id` repeats an existing one into the
keyless demo table, then the same run is started again with the same input.

Expected:

- The run fails at `transform` with `DATA_AUDIT_FAILED`.
- The new snapshot is `QUARANTINED` with `unique_order_id: FAILED, failingCount: 1`; the pointer is
  still version 1; no `data.asset.published` event is written; `DATA_SNAPSHOT_QUARANTINED` fires for
  `orders-daily` on the next monitor look.
- `POST …/publications` with that snapshot answers `409` reason `SNAPSHOT_QUARANTINED`.
- Logs, spans, metrics and audit contain no SQL text, row value or credential (checked by the
  canary script of T024-063).

## 5. Clean up

```sh
docker compose -f infra/docker/compose.yaml -f infra/docker/compose.database.yaml \
  -f infra/docker/compose.data.yaml down -v   # only this project's containers and volumes
```

## What the demo does not show

Warehouses (they need owner-provided accounts, O1241), Polars (O1242), manual publication with an
approval (US06, covered by T024-049) and external assets (US08). Each has its own integration test.
