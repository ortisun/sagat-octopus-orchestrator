# PostgreSQL target adapter

T017-014 implementation checkpoint; the runner process and durable probe admission remain separate
T017-015/016 work. A passing adapter test does not mean SQL execution is enabled in Studio.

## Connection and authority

The isolated executor receives an admitted connection, literal typed parameters and the authorized
credential capability. It has no OpenBao management token or Sagat database credentials. The
credential JSON is a closed object with `username` and `password`; fields must be nonempty UTF-8,
username is at most 63 bytes without control characters, and password must not contain NUL.
The shared codec rejects duplicate/unknown fields and redacts its diagnostic and Debug output.

The operator supplies allow/deny CIDRs and an explicit port allowlist. Namespace CIDRs and ports can only narrow that scope. Port denial occurs before DNS or a socket is opened. Each list is bounded to 256 entries; empty allowlists deny access. Every
DNS answer (at most 16) must pass both lists and the built-in metadata/reserved-address denial.
The selected IP is pinned for the connection and cancellation socket; TLS verifies the original
host name or IP. An empty allowlist denies access. The deployment must put platform stores and
service addresses in the denylist and apply the corresponding network policy. Private CIDRs are
possible explicitly. Loopback and plaintext require the explicit development profile. There is
no environment/pgpass connection-string inheritance, DNS retry, proxy or TLS downgrade.

TLS uses a caller-selected public CA bundle from the admitted immutable config, or system roots,
with rustls certificate/hostname verification. Secrets are never used as a connection URL.
The first adapter uses password authentication over verified TLS (development fixtures are the
only plaintext exception). SCRAM channel binding is not required in this delivery.

A fresh PostgreSQL session is used for each attempt and closed before normal completion returns.
There is no idle pool or credential cache. Distributed managed leasing enforces per-connection,
namespace and tenant capacity; a process semaphore additionally bounds live sockets. Thus idle
lifetime is zero, within the configured idle ceiling. Warm-session reuse is a future performance
optimization that requires tenant/version keys, reset and rotation tests first.

## Statement and transaction boundary

`query` accepts one SELECT/query statement, including read-only CTEs and set operations. `execute`
accepts a top-level INSERT, UPDATE or DELETE, including RETURNING. Multiple statements, DDL,
transaction/session control, COPY, procedural code, SELECT INTO, explicit locking and unsupported
syntax are rejected. The AST parser has a recursion ceiling of 64 and statement bytes are bounded.
The conservative parser may reject otherwise valid PostgreSQL syntax; no fallback bypass exists.

Every operation uses a prepared statement and typed positional bindings. Integer parameters that
fit signed 64-bit use INT8; larger integers and decimals bind as NUMERIC text without conversion to
floating point. Null uses PostgreSQL type inference. Expressions have already been evaluated and
are rejected at this adapter boundary. Query executes in a READ ONLY transaction. Execute asks to
commit only after every result cell and output size has been validated.

Login roles with superuser, create-role, create-database, replication, bypass-RLS or inherited
server-file/program privileges are refused. READ ONLY and an AST allowlist do not sandbox installed
functions/extensions or SECURITY DEFINER code. The target operator must restrict those grants and
target-server egress; Sagat does not create/grant itself a target database account.

Client/connect/lease deadlines and startup statement/lock/idle-in-transaction timeouts are finite.
Cancellation sends a PostgreSQL cancellation request to the same pinned address with the same TLS
policy, with a one-second cleanup ceiling, then drops the socket. There is no automatic retry.
A failure after dispatching a write is conservatively `OUTCOME_UNKNOWN`; inspection/reconciliation
is required before repeating it. Cancellation is never represented as undoing external effects.

## Retained result format

```json
{
  "columns": [{"name":"amount","type":"decimal"}],
  "rows": [[{"type":"decimal","value":"12345678901234567890.001230"}]],
  "affectedRows": "1",
  "truncated": false
}
```

Column order and duplicate names are preserved; the column type describes the PostgreSQL type,
while a null cell is always `{"type":"null","value":null}`. Types are boolean, integer decimal
string, decimal string, finite float, text, timestamp RFC3339, date ISO, binary padded base64, and
JSON value. TIMESTAMPTZ is supported; timestamp-without-zone, arrays, ranges, UUID, extension types
and nonfinite numbers are explicitly unsupported in this first adapter. Cast intentionally where
needed. NUMERIC preserves its scale, sign and precision up to the shared 16384-character ceiling.
Affected rows are also a decimal string. Overflow fails with `RESULT_LIMIT`; no partial successful
result or implicit truncation is returned.

A decrypted wire-frame guard rejects oversized frame headers before the PostgreSQL client can
allocate the body. A separate aggregate wire budget bounds startup metadata and notices. Rows are
consumed incrementally and encoded through a writer limited to actual JSON bytes, including base64
and escaping overhead. Typed conversion cannot bypass the output ceiling. Process/container memory
and CPU limits remain required; these bounds are not a promise of zero-copy memory use.

The driver and SQL parser normally log SQL at DEBUG and untrusted notices at INFO. The adapter drives the
connection message stream without formatting notices, and Sagat's log layer drops the driver
and parser targets even with TRACE enabled, including records bridged from `log`. OTel exports only Sagat
spans; the adapter span skips all inputs. Public error categories never include server diagnostics.

The canonical output schema is [database-result.schema.json](database-result.schema.json). The server-side validator also checks UTF-8 byte limits, row/column alignment, column/cell type consistency, lossless value syntax and u64 affected-row bounds.

Inline completion requires the configured result validator and credential codec. It snapshots the
immutable credential of the exact attempt/generation and its frozen limits, closes the transaction,
reads OpenBao, and then repeats the current machine/session/binding/lease/cancellation fences before
persisting anything. Provider failure refuses new completion. An already committed identical report
replays its durable receipt without another provider read. Rotation and secret deletion cannot replace
the credential used for this scan.

The codec scans raw bytes and every decoded JSON string/key, including duplicates, plus decoded
base64 binary content. Input bytes and recursion are bounded; substring search is linear. This covers
the canonical encodings; it is not an exfiltration guarantee against arbitrary SQL transformations.
A detected credential is never stored as output. Query attempts fail with `SECRET_IN_OUTPUT`;
execute attempts wait on the operator with run status `MANUAL_REVIEW` and `OUTCOME_UNKNOWN`.
Failure to retain a completed write due to quota likewise requires review, with no automatic retry.

Managed blob uploads/commits, result disclosure and production runner wiring remain closed/pending
at this inline-result checkpoint (T017-017). No complete result-delivery guarantee is claimed yet.

## Evidence and sources

Real PostgreSQL 16 fixtures cover prepared injection-shaped text, large integers/decimals, null,
binary/JSON/date/timestamp, duplicate names, READ ONLY rejection of a SECURITY DEFINER write,
restricted-role/authentication failures, row/frame limits, nonfinite numeric rejection, query and
write deadlines, cancellation, capacity release, and SQL/credential/notice log canaries. A separate
TLS fixture verifies valid private trust, wrong hostname, and untrusted CA. Unit cases exercise
mixed DNS answers, namespace narrowing, metadata/platform denial, nested write syntax and framing.
See `verification.md` for commands and results.

Driver versions: `tokio-postgres = 0.7.18`, `sqlparser = 0.63.0`, pinned in Cargo.lock.
Official references used and dependency sources inspected:

- [tokio-postgres Config](https://docs.rs/tokio-postgres/0.7.18/tokio_postgres/config/struct.Config.html)
- [Prepared typed queries](https://docs.rs/tokio-postgres/0.7.18/tokio_postgres/struct.Client.html)
- [PostgreSQL transaction modes](https://www.postgresql.org/docs/current/sql-set-transaction.html)
- [PostgreSQL TLS verification](https://www.postgresql.org/docs/17/libpq-ssl.html)
- [sqlparser](https://docs.rs/sqlparser/0.63.0/sqlparser/)
