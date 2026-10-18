# Legacy collection in-flight report fixture v1

This fixture covers two separately produced compatibility cases. The immutable
`6c44a0e49670cf19660792cd1518d241e6c99b3b` binary creates ten closed legacy
collection aggregates, leases the compensable `c10/0/reserve` forward task, and
exits without reporting it. A separately built current binary reopens the same
database. The consumer case selects whether the current binary reports the
retained ordinary success before collection drain, or starts the normal
`RUN_DATA_LIMIT` drain first while the original lease remains live.

The current phase uses the persisted worker membership, exact binding, worker ID,
session token, lease, and completion request. It creates a fresh JWT through the
current test identity provider. It does not provision, bind, or register the
worker again. The acceptance assertions require `SUCCEEDED` plus
`RUN_DATA_LIMIT`, no retained refused output, no report outbox row, exact receipt
replay, and the engine's real `line.release`
compensation task with the original target identity and key.

The no-argument default preserves the original report-before-drain case. Select
the independently named drain-first consumer and artifact namespace with:

```sh
CARGO_TARGET_DIR=/absolute/private/target \
SAGAT_TEST_DATABASE_URL='postgres://…/postgres' \
SAGAT_Q10C_OWNED_POSTGRES=1 \
scripts/test-legacy-collection-inflight-report-v1.sh --case drain-before-report
```

That case uses normal `Keeper` ownership and `Scheduler::tick` discovery. It
proves the first tick retains the original lease and requests cancellation, a
second tick waits for the live member, and the exact original success remains
`SUCCEEDED` without output before one real compensation reaches
`COMPENSATED`. It preserves the first stop and drain identity throughout and
does not seed a drain, rewrite a lease, register authority, or create a
reconciliation verdict.

Run both owner and least privilege API modes with an explicitly owned PostgreSQL
server and a private absolute Cargo target:

```sh
CARGO_TARGET_DIR=/absolute/private/target \
SAGAT_TEST_DATABASE_URL='postgres://…/postgres' \
SAGAT_Q10C_OWNED_POSTGRES=1 \
scripts/test-legacy-collection-inflight-report-v1.sh
```

The coordinator builds both test binaries before creating the retained lease.
The default uses `sagat-q10c-ordinary-before-drain-v1`; the selected drain-first
case uses `sagat-q10c-ordinary-drain-before-report-v1` and distinct log labels.
It writes the credential bearing receipt under a mode `0700` directory as an
atomic mode `0600` file, and removes the receipt and owned test database after
each phase. Logs and the nonsecret manifest contain hashes and process results,
never the lease or session values. Each case invokes a fresh old producer and
one exact current consumer in owner mode, then repeats both processes against a
fresh database as the least privilege runtime role.

These cases establish only the two ordinary `E > limit` report/drain orders and
public engine compensation semantics. They do not establish an external
provider effect, external compensation, exactly once behavior, Applied
reconciliation, `E <= limit < E + candidate`, cancellation precedence, a post
ACK process restart, output-dependent compensation, secret or blob policy, or
the remaining authority, race, and fault matrix. A passing run does not close
the full Q10c row or the open feature 014 release tasks.
