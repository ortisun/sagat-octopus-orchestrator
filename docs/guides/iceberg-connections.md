# Connect an Apache Iceberg catalog and test it

An Iceberg connection tells the platform which catalog and which object store a lake step writes to, and with which
credentials. **Saving a connection proves nothing.** The probe is how you find out whether the platform can reach the
catalog, find the warehouse and use the storage, before a workflow depends on it (FR-035-006, D035-150).

What the platform serves today is a **REST catalog over S3-compatible storage**, appending (`GET …/database-drivers`
lists exactly that, and lists Iceberg only on a deployment that has the lake on). The domain accepts other catalogs and
stores in a connection document, and no runner opens them yet, so the Studio and the probe do not offer them.

## Before you start

- The deployment runs the lake (`SAGAT_LAKE_ENABLED`), and a lake runner is bound to **both** activities,
  `sagat.database.iceberg.write` and `sagat.database.lake.probe`
  ([the runbook](../runbooks/database-runner.md) has the order of the rollout). Without a runner that offers the probe, a
  probe waits its 30 seconds and the run ends with no checks, as any run does that nobody served.
- You may manage connections (`database:manage`) and read secrets; to start a probe you also need `database:use`,
  `database:adhoc` and `run:start`, and to read it `run:read`.
- The credentials are OpenBao secrets of the namespace, written once and never shown again. A REST catalog with OAuth2 takes
  `{"clientId": "…", "clientSecret": "…"}` (a bearer token takes `{"token": "…"}`), and S3 takes
  `{"accessKeyId": "…", "secretAccessKey": "…"}`. A connection that lets the catalog hand out storage credentials
  (`vended`) needs no storage secret. The connection carries the **names** of the secrets and never a value; a URL that
  carries user information or a query credential is refused.

## Create the connection

In the Studio, **Connections → New connection**, choose Iceberg and fill the REST and S3 sections (the form can create the
secrets in OpenBao first). Over REST, `PUT /namespaces/{namespaceId}/connections/{name}`:

```json
{
  "requestId": "0b6f4c1e-5c7a-4f0e-9a43-2f1d6a7e9b10",
  "expectedRevision": 0,
  "status": "ACTIVE",
  "config": {
    "driver": "iceberg",
    "catalog": {"type": "rest", "rest": {
      "uri": "https://catalog.example/api", "warehouse": "analytics",
      "auth": {"type": "oauth2", "tokenUri": "https://catalog.example/oauth/token"},
      "accessDelegation": "none", "remoteSigning": false
    }},
    "storage": {"type": "s3", "s3": {"region": "eu-west-1", "pathStyle": false}, "credentials": "static"},
    "warehouseLocations": ["s3://lake/warehouse/"],
    "allowedModes": ["append"],
    "catalogSecret": "lake-catalog",
    "storageSecret": "lake-storage",
    "limits": {}
  }
}
```

`warehouseLocations` are the only places a write may put data files; a catalog that places the warehouse elsewhere is
refused, not followed.

## Test it

In the Studio, **Probe / status** on the connection, then **Start probe**. Over REST,
`POST /namespaces/{namespaceId}/connections/{name}/probe` with an `Idempotency-Key` and a body of `{}` (the current
revision) or `{"revision": 3}`. The answer is a run; read what it found with
`GET /namespaces/{namespaceId}/connections/{name}/probe`.

- **One probe at a time** for a connection and a revision. Another start is a `409` and makes no run; a retry of the same
  `Idempotency-Key` is the run that is already there. A probe that ended, in any way, holds nothing.
- A connection that does not exist, is paused, or is not an Iceberg connection is a `404`, as for any connection, and with the
  lake off a probe is refused at the start.
- The probe **writes nothing**: it does not create or change a table, a namespace or an object.

## Read the result

A probe runs three checks in order. Each is `PASSED`, `FAILED` with a closed code, or `SKIPPED`.

| Check | What it does | A failure says |
|---|---|---|
| `CATALOG_AUTH` | Authenticates to the catalog (the OAuth2 token included) and reads its configuration. | `DATABASE_AUTHENTICATION_FAILED`, `DATABASE_TLS_FAILED`, `DATABASE_DESTINATION_DENIED` (the egress policy refuses that host), `CATALOG_UNAVAILABLE`, `DATABASE_TIMEOUT` |
| `WAREHOUSE` | Asks the catalog for one page of namespaces, and checks that the warehouse it names is inside your `warehouseLocations`. | `DATABASE_AUTHENTICATION_FAILED`, `CATALOG_UNAVAILABLE`, `ICEBERG_LOCATION_NOT_ALLOWED`, `DATABASE_TIMEOUT` |
| `STORAGE_ACCESS` | For each location, looks for an object, or for one listed key, with the storage credentials, and checks that the bucket exists. | `DATABASE_AUTHENTICATION_FAILED`, `DATABASE_TLS_FAILED`, `DATABASE_DESTINATION_DENIED`, `STORAGE_UNAVAILABLE`, `DATABASE_TIMEOUT` |

- **A failed check is the answer, not a fault of the platform.** The probe run is `SUCCEEDED`, `failureCode` is the code of the
  first failed check, and the checks after it are `SKIPPED`.
- **`SKIPPED` is not confirmed.** A check that did not run (an earlier one failed, or the connection uses vended storage
  credentials, which nothing can try without a table) is never shown as passed.
- **A connection is confirmed only when every check passed**, for the revision that was tested, at the time it was tested.
  The Studio says `Confirmed: every check passed for revision N.` only then, and says nothing of a probe that has not ended.
  A newer revision is not confirmed by an older probe.
- **No checks never means a healthy connection.** A probe the platform could not run (the step failed, with a code such as
  `LAKE_SANDBOX_UNAVAILABLE` or `DATABASE_TIMEOUT`) has no checks, and a probe whose result can no longer be read says
  `DATABASE_RESULT_UNAVAILABLE`.

## What a passed probe does not tell you

It tells you the platform could authenticate, find the warehouse and reach the storage within the locations you declared.
It does not tell you that a table exists, that the credentials may **write** to it, or that a write will commit: that is
what a write step finds out, and the first run of one is the real test.

## When it does not work

| You see | Probably |
|---|---|
| The probe run ends after about 30 seconds with no checks | No runner offers the probe (the runner's binding or version), so nobody took it: [the runbook](../runbooks/database-runner.md). |
| `404` on the start | The lake is off, or the connection is missing, paused or not Iceberg. |
| `409` on the start | A probe of this connection and revision is already in flight; read it with the `GET`. |
| `CATALOG_AUTH` `DATABASE_DESTINATION_DENIED` | The operator's egress policy refuses the catalog's host or address. |
| `WAREHOUSE` `ICEBERG_LOCATION_NOT_ALLOWED` | The catalog's warehouse is outside `warehouseLocations`: fix the connection, not the catalog. |
| `STORAGE_ACCESS` `STORAGE_UNAVAILABLE` | The bucket is not there, or the storage cannot be reached. A bucket the credentials may not ask about (they can be scoped to a prefix) is not a failure. |
