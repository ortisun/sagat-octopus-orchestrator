# Quickstart — run visibility, analytics and archival (feature 020)

**Not runnable yet.** This is the reproducible demonstration the Definition of Done requires once the
feature is implemented (T020-056 turns it into `scripts/demo/visibility-analytics-archival.sh` and an e2e
test). Commands use the CLI and REST shapes of the draft contracts; the development stack is the existing
compose profile (`infra/docker/compose.yaml`) plus a MinIO-class S3 server and a ClickHouse server added
to the test profile (T020-003). Environment: `SAGAT_API`, `SAGAT_TOKEN` (development identity provider),
namespace `demo`.

## 1. Happy path — search attributes

```sh
# A version declaring three attributes (contracts/workflow-ir.fragment.schema.json example)
# examples/visibility/orders.yaml is created by T020-056 from the IR fragment's example
sagat workflow publish examples/visibility/orders.yaml          # registers customerId, orderTotal, fulfilmentState

for i in $(seq 1 20); do
  sagat workflow run orders --input "{\"customer\":{\"id\":\"c-$((i % 4))\"},\"total\":$((i * 10))}"
done

curl -s -X POST "$SAGAT_API/v1/runs/search" -H "Authorization: Bearer $SAGAT_TOKEN" \
  -H 'X-Octopus-Namespace: demo' -H 'Content-Type: application/json' \
  -d '{"query":"customerId = '"'"'c-1'"'"' AND orderTotal >= 50 ORDER BY StartedAt DESC","columns":["customerId","orderTotal","fulfilmentState"]}'
# Expected: only c-1 runs with total >= 50, newest first; fulfilmentState appears once the ship step completed.

curl -s -X POST "$SAGAT_API/v1/runs/count" ... -d '{"query":"Status = '"'"'SUCCEEDED'"'"'"}'
# Expected: {"count": N, "exact": true}
```

Check: the journal of one run shows `search_attributes.upserted` at admission and after `ship`;
`sagat workflow inspect <RUN>` shows the event without values.

## 2. Failure scenarios — search attributes

1. **Classification.** As a developer without the reveal grant, search `customerEmail = 'a@b.c'` (a `pii`
   attribute) → `403 FORBIDDEN`, `details.reason = PII_REVEAL_NOT_GRANTED`. Grant the reveal
   (`sagat namespace reveal-grants grant demo <subject>`), repeat with `"reveal":["pii"]` → results, and
   `GET /v1/audit` shows one `run.search_pii` record without the literal.
2. **Bounds.** A query with 40 predicates → `422 VALIDATION_FAILED`, `QUERY_LIMIT`; no SQL executed
   (`run_search_total{outcome="invalid"}` grows).
3. **Wrong derived type.** Publish `fulfilmentState` upsert from a numeric field → the run succeeds, the
   attribute stays unset and the journal holds `search_attributes.rejected` with `TYPE_MISMATCH`.
4. **Crash.** `SAGAT_FAULT=after_upsert_rows_before_commit` on the engine, complete `ship`, restart →
   the step is retried; index rows equal the journal fold (`verify_run`).

## 3. Happy path — analytics to object storage

```sh
sagat analytics sinks create demo lake --kind OBJECT_STORAGE \
  --settings '{"provider":"S3","bucket":"analytics","prefix":"sagat","endpoint":"https://minio.test:9000","format":"NDJSON_GZIP"}' \
  --credentials-secret lake-s3 --max-level INTERNAL --start EARLIEST
# Run 50 workflows, then:
duckdb -c "SELECT kind, count(*) FROM (SELECT * FROM read_ndjson_auto('s3://analytics/sagat/sagat.analytics.v1/*/*/*/*/*.ndjson.gz')
           QUALIFY row_number() OVER (PARTITION BY key ORDER BY version DESC) = 1) GROUP BY kind"
```

Expected: `run_snapshot` = number of runs, `journal_event` = sum of journal lengths, no payload value in
any record (grep for the input canary `CANARY-020-VALUE` finds nothing). `GET
/v1/namespaces/demo/analytics-sinks/<id>` shows `ACTIVE`, lag under a few seconds on the test machine
(measured, not promised).

## 4. Failure scenarios — analytics

1. **Crash between delivery and ack.** `SAGAT_FAULT=analytics_after_deliver_before_ack` → after restart
   the same object names are rewritten with identical SHA-256 (compare the manifest), then acknowledged
   once; the DuckDB count is unchanged.
2. **Long transaction.** Hold `BEGIN; SELECT pg_current_xact_id(); SELECT pg_sleep(120);` in `psql` while
   runs complete → `analytics_xmin_horizon_age_seconds` rises, lag rises, and after `COMMIT` every record
   arrives; none is missing.
3. **Permanent rejection.** Point a ClickHouse sink at a table without the `actor` column → the sink is
   `AUTO_PAUSED` with `SINK_SCHEMA_MISMATCH`, `GET /v1/namespaces/demo/alerts` shows
   `ANALYTICS_SINK_STUCK`; add the column, `POST …/actions {"action":"RESUME"}` → the same batch is
   delivered and nothing is skipped.
4. **Declared record disclosure.** Revoke the reveal grant of the `PII` sink's disclosure owner → the next
   plan auto-pauses the sink with `DISCLOSURE_REVOKED`; no further batch leaves.

## 5. Happy path — retention, archive and history export

Retention is available only after the principle-I amendment PR (T020-061) and ADR-0054 are accepted;
until then `retention set` is refused and this section is not part of the demo.

```sh
sagat workflow export-history <RUN> --mode STRUCTURE > before.json
sagat namespace retention set demo --hot-days 1 --archive REQUIRED --values METADATA_ONLY
# Test deployments move the database clock with the 028 test clock or wait one day; then:
sagat namespace retention status demo            # ELIGIBLE → … → PURGED counts
curl -s "$SAGAT_API/v1/runs/<RUN>" ...           # archive.state = ARCHIVED, still listed
curl -s "$SAGAT_API/v1/runs/<RUN>/events" ...    # 404 NOT_FOUND, details.reason = RUN_PURGED
sagat workflow export-history <RUN> --mode STRUCTURE > after.json
jq -r .digest before.json after.json             # identical digests; after.json has "source": "ARCHIVE"
```

## 6. Failure scenarios — retention

1. **Corrupted archive.** Overwrite one archive object before verification → the job is `BLOCKED`
   (`ARCHIVE_INTEGRITY`), `ARCHIVAL_BLOCKED` fires, the run keeps all its content.
2. **Crash after PUT.** `SAGAT_FAULT=archival_after_put_before_record` → the job resumes in `EXPORTING`,
   rewrites the same object name, verifies and purges once.
3. **Unsettled effect.** A run that ended `COMPENSATION_FAILED` is never selected, whatever its age.
4. **Erasure of a shared payload.** Erase a run whose input equals another retained run's input → the job
   reports `survivingSharedPayloads: 1`; the other run is intact.

Record the commands actually run and their outcomes in the feature's verification record; a step that
could not be run is reported as not run.
