# Legacy collection in-flight report fixture v1

This fixture covers one compatibility case. The immutable
`6c44a0e49670cf19660792cd1518d241e6c99b3b` binary creates ten closed legacy
collection aggregates, leases the compensable `c10/0/reserve` forward task, and
exits without reporting it. A separately built current binary reopens the same
database and reports the retained ordinary success before any current scheduler
tick.

The current phase uses the persisted worker membership, exact binding, worker ID,
session token, lease, and completion request. It creates a fresh JWT through the
current test identity provider. It does not provision, bind, or register the
worker again. The acceptance assertions require `SUCCEEDED` plus
`RUN_DATA_LIMIT`, no retained refused output, no report outbox row, exact receipt
replay before scheduler advancement, and the engine's real `line.release`
compensation task with the original target identity and key.

Run both owner and least privilege API modes with an explicitly owned PostgreSQL
server and a private absolute Cargo target:

```sh
CARGO_TARGET_DIR=/absolute/private/target \
SAGAT_TEST_DATABASE_URL='postgres://…/postgres' \
SAGAT_Q10C_OWNED_POSTGRES=1 \
scripts/test-legacy-collection-inflight-report-v1.sh
```

The coordinator builds both test binaries before creating the retained lease. It
uses a new `sagat-q10c-ordinary-before-drain-v1` artifact namespace, writes the
credential bearing receipt under a mode `0700` directory as an atomic mode `0600`
file, and removes the receipt and owned test database after each phase. Logs and
the nonsecret manifest contain hashes and process results, never the lease or
session values.

This case establishes only ordinary report before drain and public engine
compensation semantics. It does not establish an external provider effect,
external compensation, exactly once behavior, Applied reconciliation, the other
report/drain order, cancellation precedence, a post ACK process restart,
output-dependent compensation, or the remaining authority, race, and fault
matrix. A passing run does not close the full Q10c row or the open feature 014
release tasks.
