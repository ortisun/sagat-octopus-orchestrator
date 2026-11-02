# Draft contract — data runtime and sandbox profile (FR-024-021..024, 028, 029; ADR-0058)

The data runtime is a feature-005 runtime profile served by `apps/sandbox-runner`. The runner is
platform code; the containers it starts run tenant computation and are untrusted. Nothing in this
contract runs in the control plane, the engine, the scheduler or the database runner.

## Images

- **SQL data runtime** — `runners/data/Dockerfile.runtime` pins by digest a Python 3.12 slim base and
  installs, from hash-verified archives only, the DuckDB Python package with the `parquet` and `json`
  extensions baked in (no other extension present on disk) and the platform host program. It carries
  no tenant-selectable package: tenants supply SQL text only, so ADR-0057 decision 4 does not apply.
  No package index configuration, no compiler, unprivileged user. Its digest and a CycloneDX SBOM are
  provenance (FR-005-002).
- **Polars (`python-data` runtime)** — `runners/data/python-data/Dockerfile.runtime`, owned by this
  feature, pins a Python 3.12 base by digest and installs only the wheels of
  `runners/data/python-data/catalog.json` (023's `catalog.schema.json`, `language: python-data`,
  upstream `pypi`; only if O1242 is accepted): Polars and PyArrow at exact versions with their
  reviewed transitive closure, one SHA-256 per wheel and target, licence and image-build advisory
  evidence. The artifact mechanism is 023's (FR-023-027, 023 D1229): bundle v4 with `entry` `main.py`,
  `lockDigest` of a hash-pinned requirements lock that matches catalog entries exactly, and
  `runtimeCatalog` equal to this image's catalog digest; the runner refuses a mismatch
  (`SCRIPT_SANDBOX_UNAVAILABLE`) and puts only locked packages on the import path. Re-pinning follows
  D1270 (previous image kept 90 days; G1255).

The runner advertises `sagat.data.transform` and `sagat.data.query` (reserved prefix `sagat.data.`)
only after a start-up probe of the configured image passes (FR-005-009 pattern), and only with
`data:execute` on its role.

## Mount profile (ADR-0058 decision 1; supersedes ADR-0013's "no mounts" for this profile only)

| Mount | Kind and options | Size mechanism |
|---|---|---|
| `/in` | bind of a per-attempt directory the runner created and filled, `ro,noexec,nosuid,nodev` | the runner refuses to stage more than the effective input bytes; the directory is deleted when the attempt ends |
| `/out` | tmpfs, `rw,noexec,nosuid,nodev`, `size=<effective snapshot bytes>` (transform only); discarded with the container, so parts leave through the output channel below before the container exits | tmpfs pages are charged to the container's memory cgroup |
| `/spill` | tmpfs, `rw,noexec,nosuid,nodev`, `size=<effective spill bytes>`, DuckDB `temp_directory` | same |
| everything else | read-only root from the image; no other mount | — |

Container memory = DuckDB `memory_limit` + `/out` size + `/spill` size + headroom; admission refuses
limits that leave DuckDB less than 256 MiB. A unit test over the container specification generated
by `crates/infra-sandbox` MUST assert: every writable mount is tmpfs with `noexec,nosuid,nodev` and a
`size=`, every bind is read-only and `noexec,nosuid,nodev`, and nothing else is mounted. Failure
tests: executing a file written to `/out` or `/spill` fails with `EACCES`; writing past `size=`
fails with `ENOSPC` (`DATA_OUTPUT_LIMIT` or `DATA_RESOURCE_EXHAUSTED`).

## Output channel (how parts leave a discarded tmpfs)

When the statement or script has finished, the host program inside the transform container streams
each file of `/out` to the runner over standard output, in order, as length-prefixed frames
(`{"part": i, "size": n}` then exactly `n` bytes), then writes the result line and exits. The runner
treats the stream as untrusted: it refuses more parts than `maxParts`, a frame larger than 64 MiB or a
total above the effective snapshot bytes, computes SHA-256 while it uploads each part with `WriteBlob`
(purpose `DATA_SNAPSHOT_PART`), and keeps no copy. A stream that ends early or breaks a bound fails
the attempt (`DATA_OUTPUT_LIMIT` or `WORKER_ERROR`); uploaded parts are then never committed. This
extends ADR-0013's "one line out" for the data profile only (ADR-0058 decision 1).

## Containers per attempt

| | Transform container | Audit container |
|---|---|---|
| Starts | after inputs are staged and verified | after the transform container streamed its parts, exited successfully and its parts were uploaded |
| `/in` | staged input parts, read-only bind, one directory per alias | the attempt's own uploaded parts, read back from the platform with `ReadBlob` into a new runner-staged directory (read-only bind), each verified against the SHA-256 the server measured at upload |
| `/out` | tmpfs `noexec,nosuid,nodev`, `size=` effective snapshot bytes | absent |
| `/spill` | tmpfs `noexec,nosuid,nodev`, `size=` effective spill bytes | tmpfs `noexec,nosuid,nodev`, small |
| Network | none (no interface but loopback) | none |
| Root FS | read-only | read-only |
| Limits | memory, CPUs, PIDs, wall time from the effective limits | audit limits |
| Credentials | none | none |
| Capabilities | all dropped, `no-new-privileges`, default seccomp, unprivileged user | same |

The runner verifies each staged input part's size and SHA-256 while reading it through `ReadBlob`
(a stream that does not end cleanly is discarded). Total staged bytes are checked against the
effective input ceiling before any download. The containers receive their spec on standard input
and return exactly one bounded result line on standard output; standard error is a bounded,
redacted log (FR-005-007 retention).

## DuckDB settings applied by the host before any tenant SQL

```sql
SET enable_external_access = false;
SET allowed_directories = ['/in/', '/out/', '/spill/'];   -- transform; audit omits /out/
SET autoload_known_extensions = false;
SET autoinstall_known_extensions = false;
SET memory_limit = '<container memory minus /out, /spill and headroom>';
SET threads = <effective CPUs>;
SET temp_directory = '/spill';
SET max_temp_directory_size = '<effective spill>';
SET TimeZone = 'UTC';
SET lock_configuration = true;
```

Then one view per input alias over `read_parquet('/in/<alias>/*.parquet')`. A step source that
is a 017 database result is staged as `/in/<alias>/result.json` and exposed as a table whose column
types follow the result's declared cell types (integer and decimal strings become `HUGEINT`/
`DECIMAL`, timestamps `TIMESTAMPTZ`, binary `BLOB`); a step source that is a Parquet blob is one
part. These settings are defence in depth; the container is the boundary (DuckDB's own guidance).

## Transform spec (standard input, closed JSON, ≤ 1 MiB)

```json
{
  "kind": "transform",
  "engine": "duckdb",
  "inputs": [{"alias": "orders", "parts": 3}],
  "outputs": [{
    "index": 0,
    "statement": "SELECT order_date, count(*) AS orders FROM orders GROUP BY 1",
    "parameters": [{"type": "date", "value": "2026-10-25"}],
    "columns": [{"name": "order_date", "type": "date", "nullable": false},
                {"name": "orders", "type": "int64", "nullable": false}],
    "maxBytes": 268435456, "maxParts": 8
  }]
}
```

The host prepares each statement, refuses anything that is not a single query, and writes
`COPY (<statement>) TO '/out/<index>/' (FORMAT parquet, …)` split into parts of at most 64 MiB. For
`engine: polars` the spec carries `script` (the signed `python-data` artifact, verified by the
runner as for any script) instead of statements; the script receives the same `/in` and `/out` layout
and an input JSON value; it cannot reach the network or a package index.

Result line: `{"outputs":[{"index":0,"parts":[{"file":"part-0.parquet","rows":"1234"}]}]}` or
`{"failure":"DATA_TRANSFORM_FAILED"}` from the closed data failure set. No SQL error text, value or
file content is returned; the bounded log is redacted and retained under the script log quota.

## Audit spec and result

The runner builds audit SQL from the pinned declaration: row-count bounds
(`SELECT count(*)`), one generated `not_null` check counting rows with a null in any non-nullable
column, key uniqueness
(`SELECT count(*) FROM (SELECT key, count(*) c … GROUP BY key HAVING c > 1)`), accepted values, and
each custom check statement authored on the asset (one SELECT returning one row with one `BIGINT`
column `failures`). The audit container returns
`{"checks":[{"name":"unique_order_id","outcome":"FAILED","failing":"12"}]}`; the runner adds the
SHA-256 of every part it mounted, and the engine refuses the completion unless they equal the
committed parts (they are, by construction, the bytes read back from the platform). Outcomes are
`PASSED`, `FAILED`, `ERROR`; counts only, never rows.

## What the engine checks itself (not trusting the containers)

Per part: blob row `UPLOADED` with this attempt's fence, measured size and SHA-256, magic bytes at
both ends, footer length ≤ 1 MiB, footer not encrypted, schema (names, order, physical/logical
types, repetition) equal to the pinned contract, row count consistent with the reported one; per
output: part count and total bytes within ceilings. Row-level audit outcomes are taken from the
audit container (G1249).

## dataQuery

One container with no `/out`; inputs are the snapshots (or step sources) pinned at first lease. The
host runs the single statement and encodes its result into the 017 typed-cell JSON within the
effective row limit and the 256 KiB inline bound (`DATA_OUTPUT_LIMIT` above it); the engine validates
it with the 017 result validator (a reusable function) and completes the step with it as inline
output. The output is a 016 carrier whose platform-derived schema is annotated with the highest
`x-sagat-classification` level among the pinned input columns. DuckDB types map to cells as follows:

| Cell type | DuckDB types |
|---|---|
| boolean | BOOLEAN |
| integer | TINYINT…HUGEINT, UTINYINT…UBIGINT |
| decimal | DECIMAL |
| float | FLOAT, DOUBLE (finite) |
| text | VARCHAR |
| timestamp | TIMESTAMPTZ |
| date | DATE |
| binary | BLOB |
| json | JSON |
| unsupported (`DATA_TYPE_UNSUPPORTED`, no partial result) | TIMESTAMP, TIME, INTERVAL, UUID, LIST, STRUCT, MAP, UNION, ENUM |

## Failure mapping

| Condition | Code | Retryable |
|---|---|---|
| Image or runtime unavailable, probe failed | `DATA_RUNTIME_UNAVAILABLE` | yes |
| Input part missing, digest mismatch, object store error | `DATA_INPUT_UNAVAILABLE` | yes |
| A `from` input resolved to a quarantined snapshot | `DATA_INPUT_QUARANTINED` | no |
| Statement not a single query / script refused | `DATA_STATEMENT_DENIED` | no |
| Tenant SQL or script error | `DATA_TRANSFORM_FAILED` | no |
| A `dataQuery` result column of an unsupported type | `DATA_TYPE_UNSUPPORTED` | no |
| Memory, PIDs or spill exhausted | `DATA_RESOURCE_EXHAUSTED` | no |
| Output above bytes or parts ceiling | `DATA_OUTPUT_LIMIT` | no |
| Engine footer/schema check failed, or audited digests differ from the parts | `DATA_CONTRACT_VIOLATION` | no |
| Audit failed / audit errored | `DATA_AUDIT_FAILED` | no / yes |
| Lease deadline or wall time | `DATA_TIMEOUT` | yes |
| Cancellation | `DATA_CANCELLED` | no |
| Pointer moved since first lease (`onStale: fail`) | `DATA_PUBLISH_STALE` | no |
| Asset paused or deleted at completion | `DATA_ASSET_NOT_ACTIVE` | no |
| Unexpected host failure | `WORKER_ERROR` | yes |
