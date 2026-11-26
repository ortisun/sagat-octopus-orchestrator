# Draft contract — data runtime, data runner and data mode (FR-024-021..024, 028, 029; ADR-0058)

Revised 2026-11-26 against `develop` at `e5bda041` (Decisions 024-001, 024-007, 024-008, 024-013).
Three parts work together, and only the last one runs tenant code:

- the **data runner** (`apps/data-runner`, role `data-runner`, binding profile `data`): platform code
  that claims `sagat.data.transform` and `sagat.data.query`, resolves them, stages inputs, serves the
  attempt socket, uploads parts and reports. It holds a control-plane token and **no** container
  engine access, and receives no credential;
- the **data mode** of the sandbox runner (`apps/sandbox-runner`, an exclusive mode like the lake mode
  of Architecture decision 0079 rule 1): platform code that holds the container engine and **no**
  control-plane token, and starts a container only when the data runner asks it for an attempt;
- the **data container**: untrusted tenant computation (DuckDB SQL, or a Polars script).

Nothing in this contract runs in the control plane, the engine, the scheduler or the database
runner. Everything that feature 035's lake runner path already provides is reused, and the
differences are listed in Architecture decision 0058 decision 11.

## Images

- **Data runtime** — `runners/data/Dockerfile.runtime` pins by digest the Python line of design 046's
  dependency runtime (a glibc image, so a design 046 layer's native wheels load), installs from
  hash-verified archives only the DuckDB Python package with the `parquet` and `json` extensions baked
  in (no other extension present on disk) and the platform host program for the data contract. It
  carries no package catalog and no tenant-selectable package; for DuckDB tenants supply SQL text
  only. No package index configuration, no compiler, unprivileged user. Its digest and a CycloneDX
  SBOM are provenance (FR-005-002).
- **Polars (`python-data`)** — the same image. A Polars transform references a published script
  version of language `python-data` whose artifact is design 046's bundle version 5 (entry `main.py`,
  `dependencies` with ecosystem `pypi` and the version's own PEP 751 lock), resolved through the
  package gateway and checked by the namespace's supply-chain policy at publication; Polars, PyArrow
  and any library come from that version's signed read-only layer, never from the image (Owner
  decision 2026-11-26-162). A block found at lease refuses the step (`DATA_STATEMENT_DENIED`, reason
  `PACKAGE_BLOCKED`). The repository ships a reviewed starter lock for Polars and PyArrow under
  `runners/data/python-data/starter/` as a template an author copies; it is not a catalog and nothing
  enforces it.

The data runner offers `sagat.data.transform` and `sagat.data.query` (reserved prefix `sagat.data.`)
only after a probe attempt of the configured image through the data mode passes (FR-005-009
pattern), only with `data:execute` on its role, and only when the registry capability
`data-pipelines-v1` is recorded. It offers Polars only for versions whose layer the node's data mode
holds (design 046's rule: advertise an activity only once its layer is materialized; Gap 024-002).

## The attempt path (Architecture decision 0079, reused)

1. The data runner claims the task, calls `ResolveData`, and creates an attempt directory in the
   work volume it shares with the node's data-mode sandbox runner (`crates/infra-lake/src/staging.rs`,
   `AttemptDir`), named by an opaque random attempt id (never derived from a run or step id).
2. It stages each pinned part into `in/<alias>/` through `ReadBlob` (allowed by the lease's
   `step_lease_blobs` rows), checking the header is the blob asked for, the size and the SHA-256
   while reading, and removing a partial file on failure (the lake runner's whole-blob reader); the
   total is checked against the effective input bytes before any download.
3. It writes the request file (the transform or query spec below) into the attempt directory and
   binds the attempt's Unix socket there, admitting only the peer user `uid_for_slot(slot)` of the
   slot it chose in the data mode's slot range (the lake relay's `SO_PEERCRED` check).
4. It asks the data mode: `POST /v1/attempts/{attempt_id}` with `{"slot", "class", "deadline_ms"}`,
   plus, for Polars only, `"layer"`: the digest of a design 046 layer record the data mode itself
   materialized and verified. The request cannot name an image, a mount, a capability, a command, a
   limit or a user outside the data range. `GET …/wait` until `ended`; `DELETE …` ends it and returns
   `memory_killed`.
5. The host program connects to the socket, reads the request, runs it, streams the parts (output
   channel below) and the result line, and exits. The data runner stops the socket, ends the
   attempt, removes the directory after the container is gone (or after its label deadline), and
   reports.

The data mode's state lives in engine labels (attempt, slot, class, absolute deadline, memory limit);
it sweeps at start-up and every 30 s, drains on shutdown without killing running containers, and
answers every rejection with a closed body (Architecture decision 0079 rules 4 and 6 to 13).

## Mount profile (ADR-0058 decision 1; refines ADR-0013 for this profile only, as the lake profile did)

| Mount | Kind and options | Size mechanism |
|---|---|---|
| attempt directory (`in/` holds the staged inputs; the request file; the socket) | sub-path of the work volume, read-only, `ro,noexec,nosuid,nodev` | the data runner refuses to stage more than the effective input bytes; the directory is removed when the attempt ends |
| `/out` | tmpfs, `rw,noexec,nosuid,nodev`, `size=` the class's snapshot bytes (transform only); discarded with the container, so parts leave through the output channel below before the container exits | tmpfs pages are charged to the container's memory cgroup |
| `/spill` | tmpfs, `rw,noexec,nosuid,nodev`, `size=` the class's spill bytes, DuckDB `temp_directory` | same |
| `/opt/sagat/deps` (Polars only) | design 046's signed layer volume, read-only, executable (native extensions), `nosuid,nodev` where the engine applies them | fixed by the layer record's digest |
| everything else | read-only root from the image; no other mount | — |

Class memory = DuckDB `memory_limit` + `/out` size + `/spill` size + headroom; an author's limits
select the smallest class that covers them, and admission refuses limits that no configured class
covers or that leave DuckDB less than 256 MiB. A unit test over the container specification
generated by `crates/infra-sandbox/src/data.rs` MUST assert: every writable mount is tmpfs with
`noexec,nosuid,nodev` and a `size=`, every bind is read-only, the only executable mount is the signed
layer of a Polars attempt, and nothing else is mounted. Failure tests: executing a file written to
`/out` or `/spill` fails with `EACCES`; writing past `size=` fails with `ENOSPC`
(`DATA_OUTPUT_LIMIT` or `DATA_RESOURCE_EXHAUSTED`).

## Output channel (how parts leave a discarded tmpfs)

When the statement or script has finished, the host program inside the transform container streams
each file of `/out` to the data runner over the attempt socket, in order, as length-prefixed frames
(`{"part": i, "size": n}` then exactly `n` bytes), then writes the result line and exits. The data
runner treats the stream as untrusted: it refuses more parts than `maxParts`, a frame larger than
64 MiB or a total above the effective snapshot bytes, computes SHA-256 while it uploads each part with
`WriteBlob` (purpose `DATA_SNAPSHOT_PART`), and keeps no copy. A stream that ends early or breaks a
bound fails the attempt (`DATA_OUTPUT_LIMIT` or `WORKER_ERROR`); uploaded parts are then never
committed. The host's bounded, redacted log travels on the same socket and is kept as a script
step's log is (FR-005-007), never as step logs (Requirement 041-016). This replaces ADR-0013's "one
line out" for the data profile only (ADR-0058 decision 1); standard output and standard error of the
container are not read by the data runner, and the data mode keeps neither.

## Attempts per step

| | Transform attempt | Audit attempt |
|---|---|---|
| Starts | after inputs are staged and verified | after the transform attempt ended successfully and its parts were uploaded |
| Attempt directory `in/` | staged input parts, one directory per alias | the attempt's own uploaded parts, read back from the platform with `ReadBlob` into a new attempt directory, each verified against the SHA-256 the server measured at upload |
| `/out` | tmpfs `noexec,nosuid,nodev`, `size=` the class's snapshot bytes | absent |
| `/spill` | tmpfs `noexec,nosuid,nodev`, `size=` the class's spill bytes | tmpfs `noexec,nosuid,nodev`, small |
| Class | from the step's limits | `small` |
| Network | none (no interface but loopback; the socket is a file) | none |
| Root FS | read-only | read-only |
| Limits | memory, CPUs, PIDs from the class budget; wall time from the deadline | audit limits |
| Credentials | none | none |
| Capabilities | all dropped, `no-new-privileges`, default seccomp, the slot's unprivileged user | same |

## DuckDB settings applied by the host before any tenant SQL

```sql
SET enable_external_access = false;
SET allowed_directories = ['/attempt/in/', '/out/', '/spill/'];   -- transform; audit omits /out/
SET autoload_known_extensions = false;
SET autoinstall_known_extensions = false;
SET memory_limit = '<class memory minus /out, /spill and headroom>';
SET threads = <effective CPUs>;
SET temp_directory = '/spill';
SET max_temp_directory_size = '<effective spill>';
SET TimeZone = 'UTC';
SET lock_configuration = true;
```

Then one view per input alias over `read_parquet('/attempt/in/<alias>/*.parquet')` (the mount point
of the attempt directory is fixed by the data mode). A step source that is a 017 database result is
staged as `in/<alias>/result.json` and exposed as a table whose column types follow the result's
declared cell types (integer and decimal strings become `HUGEINT`/`DECIMAL`, timestamps
`TIMESTAMPTZ`, binary `BLOB`); a step source that is a Parquet blob is one part. These settings are
defence in depth; the container is the boundary (DuckDB's own guidance).

## Transform request (the request file, closed JSON, ≤ 1 MiB)

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
`engine: polars` the request carries `script` (the signed bundle version 5 artifact, verified by the
data runner as a script runner verifies any script) instead of statements; the script receives the
same `in/` and `/out` layout and an input JSON value, imports only from its layer at
`/opt/sagat/deps`, and cannot reach the network or a package index.

Result line (on the socket): `{"outputs":[{"index":0,"parts":[{"file":"part-0.parquet","rows":"1234"}]}]}`
or `{"failure":"DATA_TRANSFORM_FAILED"}` from the closed data failure set. No SQL error text, value
or file content is returned; the bounded log is redacted and retained under the script log quota.

## Audit request and result

The data runner builds audit SQL from the pinned declaration: row-count bounds
(`SELECT count(*)`), one generated `not_null` check counting rows with a null in any non-nullable
column, key uniqueness
(`SELECT count(*) FROM (SELECT key, count(*) c … GROUP BY key HAVING c > 1)`), accepted values, and
each custom check statement authored on the asset (one SELECT returning one row with one `BIGINT`
column `failures`). The audit container returns
`{"checks":[{"name":"unique_order_id","outcome":"FAILED","failing":"12"}]}`; the data runner adds
the SHA-256 of every part it staged, and the engine refuses the completion unless they equal the
committed parts (they are, by construction, the bytes read back from the platform). Outcomes are
`PASSED`, `FAILED`, `ERROR`; counts only, never rows.

## What the engine checks itself (not trusting the containers or the data runner)

Per part: blob row `UPLOADED` with this attempt's fence, measured size and SHA-256, magic bytes at
both ends, footer length ≤ 1 MiB, footer not encrypted, schema (names, order, physical/logical
types, repetition) equal to the pinned contract, row count consistent with the reported one; per
output: part count and total bytes within ceilings. Row-level audit outcomes are taken from the
audit container (G1249).

## dataQuery

One attempt with no `/out`; inputs are the snapshots (or step sources) pinned at first lease. The
host runs the single statement and encodes its result into the 017 typed-cell JSON within the
effective row limit and the 256 KiB inline bound (`DATA_OUTPUT_LIMIT` above it), sent on the socket;
the engine validates it with the 017 result validator (a reusable function) and completes the step
with it as inline output. The output is a 016 carrier whose platform-derived schema is annotated
with the highest `x-sagat-classification` level among the pinned input columns. DuckDB types map to
cells as follows:

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
| Image or runtime unavailable, probe failed; the data mode refused the attempt (`ceiling`, `slot`, `class`, `deadline`, `directory`) or was `unavailable` | `DATA_RUNTIME_UNAVAILABLE` | yes |
| Input part missing, digest mismatch, object store error | `DATA_INPUT_UNAVAILABLE` | yes |
| A `from` input resolved to a quarantined snapshot | `DATA_INPUT_QUARANTINED` | no |
| Statement not a single query / script refused / a Polars package blocked at lease (`PACKAGE_BLOCKED`) | `DATA_STATEMENT_DENIED` | no |
| Tenant SQL or script error | `DATA_TRANSFORM_FAILED` | no |
| A `dataQuery` result column of an unsupported type | `DATA_TYPE_UNSUPPORTED` | no |
| Memory (`memory_killed`), PIDs or spill exhausted | `DATA_RESOURCE_EXHAUSTED` | no |
| Output above bytes or parts ceiling | `DATA_OUTPUT_LIMIT` | no |
| Engine footer/schema check failed, or audited digests differ from the parts | `DATA_CONTRACT_VIOLATION` | no |
| Audit failed / audit errored | `DATA_AUDIT_FAILED` | no / yes |
| Lease deadline or wall time | `DATA_TIMEOUT` | yes |
| Cancellation | `DATA_CANCELLED` | no |
| Pointer moved since first lease (`onStale: fail`) | `DATA_PUBLISH_STALE` | no |
| Asset paused or deleted at completion | `DATA_ASSET_NOT_ACTIVE` | no |
| Unexpected host failure, a broken socket conversation | `WORKER_ERROR` | yes |

Every code above is a `worker`-origin entry of `sdks/error-catalog.json` with this retryable flag
(Decision 024-015).
