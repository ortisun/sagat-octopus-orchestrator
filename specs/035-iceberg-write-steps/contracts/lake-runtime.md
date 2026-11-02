# Lake runtime contract (draft, feature 035)

The `lake` deployment of `apps/database-runner` (cargo feature `lake`, image target `lake-runner`) runs
Iceberg writes, Iceberg destinations and object-storage destinations. The Rust runner owns the lease,
the credentials, the record files and the only network route. A pinned Python program (the *host*)
owns Iceberg semantics through PyIceberg, inside a per-attempt sandbox container. Decisions:
D035-026, D035-030, D035-032, D035-033, D035-036 and, after review round 1, D035-048..050,
D035-061..064 in [research.md](../research.md).

## Image

- `runners/lake/Dockerfile` stage `lake-runtime`: Python 3.12 slim base pinned by digest;
  `pyiceberg[pyarrow,s3fs,gcsfs,adlfs,hive,glue,sql-postgres]==0.12.0` and its dependency closure
  from `runners/lake/requirements.lock` (`--require-hashes --no-deps`); pip and setuptools removed
  after installation; no compiler; the host program at `/opt/sagat/lake/` read-only; an SBOM
  published with the image; `pip-audit` scheduled in CI as well as on every lock change (G035-023).
- `infra/docker/Dockerfile` target `lake-runner`: the `database-runner` binary built with
  `--features lake` in its own `cargo build` invocation (D035-058); `USER 10001`; read-only root
  filesystem. The runtime image is a separate artifact used by the sandbox container.

## Isolation (D035-048)

Each attempt runs in its own container started by `apps/sandbox-runner` with the `lake` profile of the
feature-005 driver (ADR-0013):

- own user, pid, mount and network namespaces; no network interface but loopback; a distinct UID per
  slot; read-only root; all capabilities dropped; `no-new-privileges`; the default seccomp profile;
  core dumps disabled; `PR_SET_DUMPABLE=0` set by the host as its first action;
- cgroup `memory.max` (the attempt's memory budget), `pids.max`, CPU quota, wall-clock deadline;
- mounts: the runtime image's `/opt/sagat/lake` (read-only), the attempt directory
  `/work/<attempt-id>` (read-write, size-limited) holding `relay.sock` and `relay.token` (mode 0400,
  owned by the container's UID); nothing else (no runner token, no other attempt, no container engine
  socket);
- environment: `PATH`, `HOME`, `TMPDIR`, `AWS_EC2_METADATA_DISABLED=true`; no credential variable.

The credential-holding lake runner never receives the container engine's socket: it asks the sandbox
runner to start, stop and kill the container. Their co-location (one pod sharing an `emptyDir`, or a
node-local socket) is fixed by spike T-isolation in increment I0, whose acceptance tests are: from
inside the container, reading the runner's token file, another attempt's directory, the pod network,
the metadata service and the engine socket all fail (G035-022).

## One attempt

1. The runner leases a task whose activity is `sagat.database.iceberg.write`,
   `sagat.database.iceberg.revert`, `sagat.database.destination.iceberg`,
   `sagat.database.destination.object_storage`, `sagat.database.lake.probe` or
   `sagat.database.lake.browse`, and calls `ResolveDatabase` (two-phase, fenced; both
   `catalogSecret` and `storageSecret` compared, pinned and audited).
2. It creates `/work/<attempt-id>/` and streams the record sources into `in/`: inline records as
   `inline.ndjson`; each blob, read with `ReadBlob` under the lease, as `blob-<n>.<ndjson|parquet>`; a
   017 database result as `result.json`. It checks the total against the effective limit while
   streaming; going over fails `ICEBERG_LIMIT_EXCEEDED` before any container starts.
3. It opens the relay socket, writes a fresh per-attempt token to `relay.token`, and asks the sandbox
   runner for a `lake` container running `python3 -I -X utf8 /opt/sagat/lake/host.py`. The start
   request carries no credential.
4. The host authenticates on the relay socket with the token and reads **one JSON request line**,
   served by the lake runner over that socket, so credentials never transit the sandbox runner:

```json
{
  "protocol": "sagat.lake.v1",
  "operation": "icebergWrite | icebergRevert | objectWrite | probe | browse",
  "effectKey": "8f1c…:save-orders",
  "firstAttemptStartedAt": "2026-10-27T18:02:11.204Z",
  "snapshotProperties": {"sagat.effect-key": "…", "sagat.run-id": "…", "sagat.step-id": "…",
                         "sagat.workflow": "orders-sync", "sagat.workflow-version": "12"},
  "connection": {"catalog": {…pinned settings…}, "storage": {…}, "warehouseLocations": ["s3://lake/"],
                 "credentials": {…resolved values…}},
  "relay": {"socket": "/work/<attempt-id>/relay.sock", "token": "…"},
  "table": {"namespace": ["sales"], "name": "orders"}, "ref": "main",
  "mode": "append", "overwrite": null, "upsert": null,
  "inputs": [{"file": "in/blob-0.parquet", "format": "parquet"}],
  "mapping": [{"field": "/amount", "column": "amount"}], "ignoreUnknownFields": false,
  "limits": {"maxCommitRetries": 4, "maxDecodedBytes": "17179869184", "maxMetadataBytes": "67108864",
             "deadlineAt": "2026-10-27T18:12:11Z"}
}
```

5. The host writes **progress lines** (`{"heartbeat": true}`, at most one per second) and exactly
   **one result line** on the same socket, then exits 0:
   `{"receipt": {…}, "relayDenials": 0}` or
   `{"failure": {"code": "ICEBERG_SCHEMA_MISMATCH", "column": "amount", "reason": "SCALE_OVERFLOW"}, "relayDenials": 0}`.
   The codes come from the closed set in [errors.md](errors.md).
6. The runner parses the result line as typed fields and **rebuilds** the receipt
   ([receipts.schema.json](receipts.schema.json), no unevaluated properties). Anything else fails the
   attempt as `LAKE_HOST_PROTOCOL`. The host's standard error is never stored or logged: the runner
   keeps a closed exception class and a line count (D035-062). The runner then reports
   `CompleteStep` or `FailStep`, audits relay denials, kills the container and removes the attempt
   directory.

A lease loss, cancellation, deadline, missing heartbeat or `memory.max` kill ends the container. The
next attempt's lookup resolves whatever it already sent (D035-030).

## The relay (D035-049)

- **Socket**: the lake runner serves `/work/<attempt-id>/relay.sock` and accepts only connections that
  present the attempt's token. An in-container forwarder (part of the pinned host program) turns it
  into loopback ports: an HTTP `CONNECT` proxy and one TCP port per non-HTTP destination (Hive URIs,
  JDBC catalog database).
- **Policy**: each `CONNECT host:port` and TCP mapping must match an endpoint declared on the pinned
  connection, or an official regional endpoint derived from its storage type and region. The runner
  then applies `crates/infra-database/src/network.rs`: resolve once, check every address (including
  the IPv6 metadata addresses added by D035-061), connect to the checked address, and refuse
  loopback, link-local, metadata and platform state endpoints. Vended and catalog-supplied endpoints
  are allowed only if they match. TLS is end to end; the relay never terminates it.
- **TLS over the TCP relay**: verified against the declared host name (psycopg
  `host=<name> hostaddr=127.0.0.1 sslmode=verify-full`; Thrift TLS `server_hostname`).
- **Limits of the relay** (G035-021): it sees host names, not buckets, so bucket confinement to
  `warehouseLocations` is enforced by the host and by credentials scoped to the declared prefixes.

## Host semantics

- **Configuration hygiene** (D035-050): from REST `/v1/config` and `LoadTable.config`, accept only
  `warehouse`, `prefix` and the vended credential keys of the declared storage type; a vended region
  never changes the endpoints derived from the connection's declared region. Pin `py-io-impl`
  and the FileIO implementation. Refuse remote signing unless the connection declares it. Pass GCS and
  Azure credentials explicitly, and fail closed when any are missing.
- **Compatibility** (FR-035-011): refuse format v3, a non-Parquet default file format, partition
  transforms PyIceberg cannot apply, locations outside `warehouseLocations`, and metadata files above
  `maxMetadataBytes`; check that the ref exists.
- **Lookup and commit**: D035-030, steps 1–6. Every commit carries `snapshotProperties` on each
  snapshot it produces, and `commit.retry.num-retries` is set to 0 on the loaded table object in
  memory, never committed. T-commit-once proves exactly one request per search.
- **Modes**:
  - `append`: `Table.append` with streamed batches.
  - `overwrite`, scope `filter`: check that every row satisfies the filter, then
    `Table.overwrite(df, overwrite_filter=…)`.
  - `overwrite`, scope `batchPartitions`: `Table.dynamic_partition_overwrite(df)`.
  - `overwrite`, scope `table`: `Table.overwrite(df, overwrite_filter=AlwaysTrue())`.
  - `upsert`: split rows by delete marker, then stage the upsert of the remaining rows (joined on the
    keys) and the deletion of the marked keys in **one** `Table.transaction()`, so one commit.
    PyIceberg 0.12.0 may not expose `upsert` on the transaction object; if it does not, the host
    composes the same copy-on-write changes from `overwrite`, `delete` and `append`.
    T-upsert-one-commit proves a single commit. Duplicate keys are refused before any write.
- **Decoding bounds** (D035-063): decoded bytes, row-group size, manifest count and metadata size are
  checked while reading; going over fails `ICEBERG_LIMIT_EXCEEDED`.
- **Conflict validation** (FR-035-017): after a conflict, read the commits since `H`. Never retry
  blindly if any of them added or deleted data in the overwrite scope or touched the upserted keys:
  re-read, re-plan and re-validate, within `maxCommitRetries`.
- **Revert** (FR-035-021, D035-056): find the step's commit by its effect key. Refuse with
  `ICEBERG_REVERT_BLOCKED` if a later snapshot on the ref removed or rewrote its added files, or if a
  removed file no longer exists. Otherwise commit the inverse file changes as a new commit carrying the
  revert's effect key.
- **Object write**: the host encodes Parquet to `out/object.parquet`. The runner computes
  `sagat-records-sha256` over the canonical NDJSON form of the mapped records (D035-064) and uploads
  the file (D035-036, D035-061). The runner encodes NDJSON itself, without a container.
- **Probe**: authenticate, load the warehouse configuration and list one namespace page. For object
  storage, run the double-create check and try one write outside the prefix, warning if it is allowed
  (G035-021). Returns per-check results only.
- **Browse**: one bounded page of namespaces or tables, or one table's schema, partition spec,
  identifier fields, refs and format version. Never data or file locations.
