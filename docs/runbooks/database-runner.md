# Managed PostgreSQL runner

The `database-runner` executable performs customer SQL in a separate process. The
execution engine issues scoped capabilities and retains results; OpenBao credentials
stay behind that API. The runner needs a `database-runner` workload identity bound
to its configured namespaces and `sagat.database.postgresql`. It needs no Sagat
PostgreSQL credential or OpenBao token.

## Start

Build with `cargo build -p sagat-database-runner --bin database-runner --locked`.
Set `SAGAT_DATABASE_RUNNER_CONFIG=/etc/sagat/database-runner.json`. Mount that
operator-owned file and the short-lived workload token file read-only; use a token
agent to replace the latter. Keep the replica name stable across restarts and unique
among live replicas. New namespace registrations require a restart. Policy and token
contents are reread for subsequent attempts/calls; publish files by atomic rename.

Example (replace namespace UUID, endpoints and network ranges with deployment values):

```json
{
  "grpcUrl": "https://engine.example.internal:50051",
  "tokenFile": "/run/sagat/worker-token",
  "caFile": "/etc/sagat/engine-ca.pem",
  "name": "database-runner-0",
  "maxConcurrent": 4,
  "pollIntervalMs": 250,
  "rpcTimeoutMs": 5000,
  "developmentLoopback": false,
  "deployment": {
    "allowCidrs": ["10.32.0.0/16"],
    "allowPorts": [5432],
    "denyCidrs": ["10.16.0.0/16"],
    "limits": {
      "maxStatementBytes": 16384, "maxParameters": 64,
      "maxParameterBytes": 65536, "maxRows": 1000,
      "maxResultBytes": 1048576, "timeoutMs": 10000,
      "connectTimeoutMs": 5000, "poolSize": 4, "idleTimeoutMs": 30000
    }
  },
  "namespaces": [{
    "id": "00000000-0000-0000-0000-000000000001",
    "slug": "payments",
    "allowCidrs": ["10.32.4.0/24"],
    "allowPorts": [5432],
    "limits": {
      "maxStatementBytes": 16384, "maxParameters": 64,
      "maxParameterBytes": 65536, "maxRows": 1000,
      "maxResultBytes": 1048576, "timeoutMs": 10000,
      "connectTimeoutMs": 5000, "poolSize": 2, "idleTimeoutMs": 30000
    }
  }]
}
```

The deny ranges must cover platform state services in your deployment. CIDRs and
ports are intersected with namespace policy; an empty allow scope denies access.
All target sessions close after one attempt. `poolSize` bounds live sessions per
namespace/connection, including older revisions; there is no warm credential pool.
The engine also enforces distributed claim limits. Align server catalog limits with
operator policy to avoid leasing tasks that a narrower runner limit will refuse.

For local fixtures only, `developmentLoopback=true` permits numeric loopback HTTP
to the engine and an explicitly configured plaintext loopback target. Production
TLS remains verified. The Docker target, development overlay, production Helm profile
and workload-auth bootstrap are documented in [OpenBao deployment](openbao.md).
`SAGAT_DATABASE_RUNNER_NAME` can override the configured name with the stable pod name;
the same length/character and namespace constraints apply.

### The lake profile (feature 035)

An optional `lake` section turns on the runner's lake profile. **Absent means off.** Present, it is validated whole when
the runner starts, and a value outside its bounds stops the runner: nothing is defaulted that is a decision about security.
**Today the section is read and validated and nothing serves lake activities yet** (the attempt driver is built in slices,
`specs/035-iceberg-write-steps/research.md` D035-123); do not rely on it to write to a table.

```json
"lake": {
  "workRoot": "/work",
  "sandboxSocket": "/run/sagat/lake-sandbox.sock",
  "sandboxUid": 10002,
  "slots": 8,
  "relayAllowCidrs": ["0.0.0.0/0", "::/0"],
  "relayDenyCidrs": ["10.96.0.0/12", "10.244.0.0/16"]
}
```

- `workRoot` and `sandboxSocket` are absolute paths (no `..`). `sandboxUid` is the user the sandbox runner runs as: the
  runner believes only a socket whose other end is that user. It is neither root nor one of the containers' users
  (`40000` to `44095`).
- `slots` is how many containers' users there are, 1 to 4096; the lake runner picks the slot of an attempt.
- `relayAllowCidrs` are the ranges the relay may connect to for a destination's catalog and storage. **An empty list
  reaches nothing and is refused**, except with `developmentLoopback`.
- `relayDenyCidrs` are the ranges it may never connect to, and a deny range wins over an allow range. **The relay itself
  refuses loopback, link-local, metadata and other special-use addresses, and no private network.** A tenant declares the endpoints of its connection, so the
  platform's own state (PostgreSQL, the broker, the secret store, the Kubernetes API) and your pod and service networks must
  be listed here, exactly as `denyCidrs` is for the SQL runner. It is required and non-empty, except with
  `developmentLoopback`.
- Optional, with the defaults of the contract: `maxBytes` (1 GiB, staged input of one attempt), `maxRecords` (10 million),
  `maxCommitRetries` (4, at most 16), `maxDecodedBytes` (16 GiB), `maxMetadataBytes` (64 MiB), `hostStartSecs` (30),
  `killLimitSecs` (20) and `largeAboveBytes` (none).

**The identity of a lake runner carries the profile `lake`** (D035-136). A dedicated runner registers only for the managed
activities of the profile its binding carries, whatever its patterns cover, so a runner bound to `*` is still of one profile
and a stolen lake token cannot register as a SQL runner. Bind it with the profile named, and with the activity it serves:

```sh
curl -X PUT "$SAGAT/namespaces/$NAMESPACE_ID/worker-bindings/$RUNNER_SUBJECT" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H "Content-Type: application/json" \
  -d '{"activities": ["sagat.database.iceberg.write"], "profile": "lake"}'
```

A binding made without `profile` is `database`, which is what every SQL runner has always been; a replacement that leaves
`profile` out keeps the one in force, and an identity has **one profile in the whole tenant**: binding it with another profile in a second
namespace is refused (409) until its bindings elsewhere are removed. Removing the last binding of an identity forgets its profile, so bind it again
with the profile named: the audit record of every binding says which profile came to be in force. During a rolling upgrade an older replica
ignores the profile, so bind a lake runner to the exact activity, as above, and not to `*`, until every replica has the profile. Changing the profile closes the sessions of the identity that offer the other profile's
activity (the answer counts them in `revoked`). The registration of a `database` runner for the lake activity, of a `lake`
runner for the SQL activity, of both, or of an ordinary activity beside either, is refused as not bound.

**The credentials of a lake step** (D035-141). A lake runner that has leased its step asks `ResolveDatabase` for the connection and
its secrets. Nothing is resolved unless the session offers the lake activity **and** its binding is of the profile `lake` alone
(the same check the lease makes, repeated where the secret leaves). The answer carries the stored connection in `iceberg` and a
`credentials` map with `catalog` and `storage`: only the ones the connection names (a catalog that delegates the storage credentials
sends no `storage`), and nothing of SQL. The server reads the two secrets from OpenBao one after the other, catalog then storage, and
hands them over only if neither changed while it read the other; then it pins each version under its role for the attempt, and
writes one audit record `secret.resolved` for each (`... generation 1: catalog <name> v<n>` and `... storage <name> v<n>`),
never the value. A second attempt reads what is in force then; the same attempt, asked again, gets the versions it pinned. The pins go with the
terminal retention of the run, all roles together. **The process that serves the worker gRPC needs `SAGAT_LAKE_ENABLED=true`** too:
without it the step is not found, as any lake node is on a process that has the lake off. The runner that carries out the attempt
is not built yet, so a deployment should not rely on this to write to a table.

**The completion of a lake step** (D035-142). The runner reports `CompleteStep` with the receipt of the write. The server reads it into typed fields, holds it to the step (the
effect key, the ref, the mode, that a commit has a snapshot and that an empty batch is not accepted), and keeps the receipt it wrote again: never the bytes the runner sent. A completion
that no resolve preceded, or a receipt that is not the step's, is refused (`INVALID_ARGUMENT`), leaves the step as it was and is audited once for the run; a lake step cannot upload a
blob. A step whose lease lapsed waits for an operator (`LEASE_EXPIRED`), and a commit whose receipt the run has no room to keep waits with an unknown outcome. **Rotating a secret does not
cut an attempt that is in flight**: the attempt keeps the version it pinned until its lease ends.

## Observe and recover

- Standard `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` and `http/json` configure traces.
  Attempt spans carry trace, tenant, namespace and run identifiers, without content.
- Scrape `/metrics` on `SAGAT_DATABASE_RUNNER_METRICS_ADDR` (loopback port 9092 by
  default). Counters distinguish execution outcomes, registration/poll failures,
  report retries and unacknowledged attempts. `database_connection_seconds` measures DNS policy,
  socket/TLS and target authentication with a fixed outcome label. Never expose this listener publicly.
- Check the durable run journal for outcome and audited result access. A failure
  code is a category, not a raw PostgreSQL/OpenBao diagnostic.
- `DATABASE_DESTINATION_DENIED`: inspect operator and namespace CIDRs/ports and
  platform-state exclusions. Never broaden policy based solely on a hostname.
- `DATABASE_CAPACITY_EXHAUSTED`: align policy limits or wait for existing sessions;
  SQL has not been dispatched on this attempt.
- `DATABASE_CREDENTIAL_UNAVAILABLE`: inspect engine/OpenBao health and scoped
  workload authentication. There is no local-secret fallback.
- `OUTCOME_UNKNOWN` or an expired write attempt: inspect the target using the
  business idempotency/reconciliation key before approving another attempt.
  A process restart or cancellation cannot prove rollback of a committed write.
- SIGTERM/SIGINT requests cancellation and a bounded 30-second report drain.
  A completion acknowledgment can be lost after the engine commits; identical
  terminal reports receive the original durable receipt and never execute SQL again.

## Reproduce the process test

With Docker available:

```sh
cargo build -p sagat-database-runner --bin database-runner --locked
cargo test -p sagat-tests --test integration_database_runner --locked
```

For a custom target directory, set `SAGAT_DATABASE_RUNNER_BIN` to the executable.
The fixture launches real PostgreSQL/OpenBao and a separate runner with a cleared
environment. It covers typed and blob results, writes, lost completion acknowledgments,
heartbeat loss, namespace capability refusal, shared local pool limits, namespace
fairness, policy reload, graceful shutdown and TRACE canaries. T017-024 tracks further
crash/restore tests, including process interruption during active writes/reporting.
