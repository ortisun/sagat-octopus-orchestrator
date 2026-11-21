# Draft contract — database engines (FR-024-001..013)

Draft for review; merged into the canonical contracts by T024-008. Unchanged 017 rules apply to
every driver: revision pinned at admission, two-phase capability resolution, one statement, one
attempt, closed failure codes, no driver or provider message in any output, fresh session per
attempt with no idle credential-bearing pool.

## Driver catalog

| Driver id | Executor (reserved activity) | Operations | Auth modes | Network profile | Deadline ceiling |
|---|---|---|---|---|---|
| `postgresql` | database runner, private deployment (`sagat.database.postgresql`) | query, execute | password | private CIDR/port | 30 s |
| `mysql` | database runner, private deployment (`sagat.database.mysql`) | query, execute | password | private CIDR/port | 30 s |
| `mariadb` | database runner, private deployment (`sagat.database.mariadb`) | query, execute | password | private CIDR/port | 30 s |
| `sqlserver` | database runner, private deployment (`sagat.database.sqlserver`) | query, execute | sql-login | private CIDR/port | 30 s |
| `bigquery` | database runner, warehouse deployment (`sagat.database.bigquery`) | query, execute | service-account-key | public endpoint | 600 s |
| `snowflake` | database runner, warehouse deployment (`sagat.database.snowflake`) | query, execute | key-pair | public endpoint (PrivateLink as operator network) | 600 s |

DuckDB is not a driver: DuckDB queries are `dataQuery` data nodes executed by the sandbox data
runtime ([data runtime](data-runtime.md#dataquery)), so ADR-0036 decision 1 and the 017 result view
are unchanged. Both runner deployments use the existing `database-runner` role.

A driver appears in `listDatabaseDrivers` only when enabled server-side, its release gate passed
(for `bigquery`/`snowflake` the real-service suite, O1241) and at least the server capability is on;
runner availability is reported separately by the probe. An executor registers only drivers its
deployment enables; a 1.6 runner registers only `postgresql`.

## Settings (non-secret, closed objects)

| Driver | Fields (all others refused) |
|---|---|
| `mysql`, `mariadb` | `driver`, `host` (bare DNS name or IP), `port` (1–65535), `database` (1–64), `tls` (canonical 017 `DatabaseTls`: `verify-full` with optional CA PEM ≤ 64 KiB, or `development-plaintext`), `secretName`, `limits` |
| `sqlserver` | `driver`, `host`, `port`, `database` (1–128), `encryption` (`strict`, `verify-full` or `development-plaintext`; optional CA PEM; optional `hostNameInCertificate`), `secretName`, `limits` |
| `bigquery` | `driver`, `projectId` (`^[a-z][a-z0-9-]{4,28}[a-z0-9]$`), `location` (1–64, `[A-Za-z0-9-]`), `maximumBytesBilled` (decimal string, 1..1 TiB), `secretName`, `limits` |
| `snowflake` | `driver`, `account` (`^[A-Za-z0-9_.-]{1,255}$`, organisation-account identifier), `endpoint` (`public` or `privatelink`), `warehouse`, `role`, `database`, `schema` (identifiers, 1–255), `secretName`, `limits` |

URLs, DSNs, connection strings, user names and passwords in settings are refused. `limits` uses
the 017 `DatabaseLimits` object; `timeoutMs` is bounded by the driver's ceiling.

## Credentials (OpenBao KV v2 values, closed JSON)

| Auth mode | Value | Codec rules | Secret-bearing fields scanned in output |
|---|---|---|---|
| password, sql-login | `{"username","password"}` | 017 codec (username ≤ 63 bytes for PostgreSQL, ≤ 128 for SQL Server, ≤ 80 for MySQL/MariaDB); no NUL | `password` |
| service-account-key | Google service-account key JSON | `type` must be `service_account`; `client_email`, `private_key_id`, `private_key` (PKCS#8 PEM) required; `token_uri`, if present, must equal `https://oauth2.googleapis.com/token`; `universe_domain`, if present, must equal `googleapis.com`; size ≤ 16 KiB | `private_key` (PEM and DER forms), `private_key_id` |
| key-pair | `{"user","privateKeyPem","passphrase"?}` | PKCS#8 RSA ≥ 2048 bits, encrypted or not; `user` 1–255 | `privateKeyPem` (PEM and DER), `passphrase` |

Endpoints, audiences and token URIs are fixed by the driver and the deployment, never read from a
credential. Signed JWTs (BigQuery token exchange, Snowflake key-pair) live only in runner memory for
the attempt and are never logged, persisted or returned.

## Network

Two deployments of the same runner binary keep the profiles apart (D1243): the **private**
deployment has no public egress; the **warehouse** deployment's Kubernetes NetworkPolicy allows only
DNS, the engine and TCP 443 to non-private ranges or operator-listed private-endpoint networks, and
no route to private customer networks. Each deployment registers only its profile's drivers.

- **Private CIDR/port** (postgresql, mysql, mariadb, sqlserver): 017 `NetworkPolicy` unchanged —
  operator allowlist narrowed by namespace, port check before DNS, every answer validated, address
  pinned for the session and the cancellation connection, TLS verifies the configured name.
- **Public endpoint** (bigquery, snowflake): host form fixed per driver —
  `bigquery.googleapis.com` and `oauth2.googleapis.com`; `<account>.snowflakecomputing.com` or
  `<account>.privatelink.snowflakecomputing.com`. The deployment config lists allowed hosts (exact or
  suffix); namespace policy can only narrow that list. Resolution happens inside the connector; each address must be public unless it lies in an
  operator-listed private-endpoint network; loopback, link-local, metadata, multicast, reserved and
  platform state addresses are always refused. No redirects, no environment proxy, response bodies
  bounded (64 KiB for metadata calls; result pages by the result limits).

## Statements and query-mode enforcement

| Driver | Placeholders | `query` accepts | `execute` accepts | Query-mode enforcement (in addition to the allowlist, which is not a boundary) |
|---|---|---|---|---|
| mysql / mariadb | `?` | one SELECT (CTE, set operations) | one INSERT / UPDATE / DELETE (RETURNING on MariaDB only) | `START TRANSACTION READ ONLY`, rolled back; temporary-table DML stays possible, so DDL is refused and the role must lack `CREATE TEMPORARY TABLES` |
| sqlserver | `@P1`… | one SELECT | one INSERT / UPDATE / DELETE / MERGE (OUTPUT allowed) | explicit transaction always rolled back; `SET XACT_ABORT ON` |
| bigquery | `?` (positional) | one SELECT | one INSERT / UPDATE / DELETE / MERGE | dry run first; execution only if the dry run reports a SELECT statement type — enforced only after the contract suite verifies the field (G1241); until then the allowlist plus the customer's IAM are the only control |
| snowflake | `?` (bindings "1".."n") | one SELECT | one INSERT / UPDATE / DELETE / MERGE | none the SQL API exposes: `MULTI_STATEMENT_COUNT=1`, administrative roles refused, but a custom role's write grants are not detectable — the allowlist plus the customer's grants are the only control, and the Studio says so |

Refused everywhere: multiple statements, DDL, `GRANT`/`REVOKE`, transaction and session control
(`USE`, `SET`, `ALTER SESSION`), `CALL`/`EXEC` of procedures, `LOAD DATA`/`COPY`/`BULK INSERT`,
`SELECT … INTO`, explicit locking, `EXPORT DATA` (BigQuery), `PUT`/`GET` (Snowflake).

## Privileged identity refusal (probe and every attempt)

| Driver | Refused when |
|---|---|
| mysql / mariadb | the current user holds `SUPER`, `FILE`, `PROCESS`, `SHUTDOWN`, `CREATE USER`, `RELOAD` or `GRANT OPTION` globally (`information_schema.USER_PRIVILEGES`) |
| sqlserver | `IS_SRVROLEMEMBER` is 1 for `sysadmin`, `securityadmin` or `serveradmin`, or `IS_MEMBER('db_owner')` is 1 |
| snowflake | `CURRENT_ROLE()` is `ACCOUNTADMIN`, `ORGADMIN`, `SECURITYADMIN`, `SYSADMIN` or `USERADMIN` |
| bigquery | not checkable with a scoped key; least-privilege IAM is documented as the customer's control (G1242) |

## Parameter binding

| 017 type | mysql / mariadb | sqlserver | bigquery | snowflake |
|---|---|---|---|---|
| null | NULL | NULL (typed by placeholder) | typed NULL (declared type required) | `TEXT` null |
| boolean | `TINYINT` 0/1 | `bit` | `BOOL` | `BOOLEAN` |
| integer (≤ 40 chars) | `BIGINT`, else `DECIMAL(65,0)` text | `bigint`, else `decimal(38,0)` | `INT64`, else `BIGNUMERIC` | `FIXED` |
| decimal | `DECIMAL` text | `decimal(38,s)` if it fits, else refused | `BIGNUMERIC` | `FIXED` |
| float | `DOUBLE` | `float` | `FLOAT64` | `REAL` |
| text | `utf8mb4` string | `nvarchar` | `STRING` | `TEXT` |
| timestamp | `TIMESTAMP` (session UTC) | `datetimeoffset` | `TIMESTAMP` | `TIMESTAMP_TZ` |
| date | `DATE` | `date` | `DATE` | `DATE` |
| binary | `VARBINARY` | `varbinary` | `BYTES` | `BINARY` (hex) |
| json | `JSON` text (MySQL), text (MariaDB) | `nvarchar` | `JSON` | `TEXT` (author applies `PARSE_JSON`) |

A value that does not fit its target fails with `DATABASE_TYPE_UNSUPPORTED` before dispatch.

## Result cells (normative mapping)

| Cell type | mysql / mariadb | sqlserver | bigquery | snowflake |
|---|---|---|---|---|
| boolean | `BIT(1)` | `bit` | `BOOL` | `BOOLEAN` |
| integer | `TINYINT`…`BIGINT` signed/unsigned, `YEAR` refused | `tinyint`…`bigint` | `INT64` | `NUMBER(p,0)` |
| decimal | `DECIMAL` | `decimal`, `numeric`, `money`, `smallmoney` | `NUMERIC`, `BIGNUMERIC` | `NUMBER(p,s>0)` |
| float | `FLOAT`, `DOUBLE` (finite) | `real`, `float` (finite) | `FLOAT64` (finite) | `FLOAT` (finite) |
| text | `CHAR`, `VARCHAR`, `TEXT`, `ENUM`, `SET` (utf8mb4 session); MariaDB `JSON` | `char`, `varchar` (decoded by collation), `nchar`, `nvarchar`, `ntext` | `STRING` | `VARCHAR` |
| timestamp | `TIMESTAMP` (session `+00:00`) | `datetimeoffset` | `TIMESTAMP` | `TIMESTAMP_TZ`, `TIMESTAMP_LTZ` |
| date | `DATE` (zero dates refused) | `date` | `DATE` | `DATE` |
| binary | `BINARY`, `VARBINARY`, `BLOB` | `binary`, `varbinary`, `image` | `BYTES` | `BINARY` |
| json | MySQL `JSON` | — | `JSON` | `VARIANT`, `OBJECT`, `ARRAY` |
| unsupported (`DATABASE_TYPE_UNSUPPORTED`) | `DATETIME`, `TIME`, `YEAR`, `BIT(n>1)`, spatial | `datetime`, `datetime2`, `smalldatetime`, `time`, `uniqueidentifier`, `xml`, `sql_variant`, `hierarchyid`, spatial, `vector` | `DATETIME`, `TIME`, `INTERVAL`, `RANGE`, `GEOGRAPHY`, `ARRAY`, `STRUCT` | `TIMESTAMP_NTZ`, `TIME`, `GEOGRAPHY`, `GEOMETRY`, `VECTOR`, `FILE` |

Cast in SQL to obtain a supported type. Column order and duplicate names are preserved; `affectedRows`
is a decimal string (BigQuery `numDmlAffectedRows`; Snowflake `number of rows inserted/updated/deleted`).

## Probes

Fixed, no caller SQL: `SELECT 1` (mysql, mariadb, sqlserver, snowflake — the Snowflake probe resumes
the configured warehouse and consumes its credits, stated in the Studio), a BigQuery dry run of
`SELECT 1` in the project/location (no bytes billed). Each probe also runs the privileged-identity
check. (The data runtime has its own start-up probe in the sandbox runner, not a connection probe.)

## Cancellation

MySQL/MariaDB `KILL QUERY <id>` on a second session to the pinned address; SQL Server TDS attention;
BigQuery `jobs.cancel`; Snowflake statement cancel by handle. Cleanup bounded by one second; a
cancelled write is still `OUTCOME_UNKNOWN`
unless the engine reported its outcome first.

## Warehouse identity and lookup

- BigQuery job ID: `sagat_` + base32 of HMAC-SHA256(deployment key, tenant ‖ run ‖ node ‖ attempt),
  truncated to 52 characters; jobs are created with `jobCreationMode=JOB_CREATION_REQUIRED` so a
  job exists to look up; `requestId` is not relied upon (15-minute window).
- Snowflake `requestId`: UUID derived from the same HMAC; resubmission uses `retry=true`.
- After a transport error, timeout or 5xx on a write, the runner looks up up to 3 times within
  30 s and the lease: BigQuery `jobs.get` (ID, location; `DONE` without `errorResult` = success);
  Snowflake resubmission with the same `requestId` and `retry=true`, or status by handle. Known
  success completes the step (with `affectedRows`); known failure fails it; anything else is
  `OUTCOME_UNKNOWN` and the manual-review record carries the opaque identity, never SQL.
- Labels/query tags: fixed `sagat_origin=workflow`; no tenant, run, user or asset identifiers.

## New closed failure code

`DATABASE_COST_LIMIT` — BigQuery refused the job under `maximumBytesBilled`. Non-retryable. All
other outcomes use the existing 017 `DatabaseFailure` codes.
