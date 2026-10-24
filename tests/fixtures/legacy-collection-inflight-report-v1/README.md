# Legacy collection in-flight report fixture v1

This fixture covers four separately produced compatibility cases. The immutable
`6c44a0e49670cf19660792cd1518d241e6c99b3b` binary creates ten closed legacy
collection aggregates, leases the compensable `c10/0/reserve` forward task, and
exits without reporting it. A separately built current binary reopens the same
database. The consumer case selects whether the current binary reports the
retained ordinary success before collection drain, reports a success whose
increment alone crosses the run limit, starts the normal `RUN_DATA_LIMIT` drain
first, or commits a public cancellation before the normal cancelled drain starts
while the original lease remains live.

The current phase uses the persisted worker membership, exact binding, worker ID,
session token, lease, and completion request. It creates a fresh JWT through the
current test identity provider. It does not provision, bind, or register the
worker again. The acceptance assertions require `SUCCEEDED` with the applicable
data-limit or cancellation stop, no retained refused output, no report outbox
row, exact receipt replay, and the engine's real `line.release`
compensation task with the original target identity and key.

The no-argument default preserves the original report-before-drain case. Select
the independently named under-limit increment consumer and artifact namespace
with:

```sh
CARGO_TARGET_DIR=/absolute/private/target \
SAGAT_TEST_DATABASE_URL='postgres://…/postgres' \
SAGAT_Q10C_OWNED_POSTGRES=1 \
scripts/test-legacy-collection-inflight-report-v1.sh --case under-limit-increment-before-drain
```

That case closes ten old source values whose JSON bodies are 200,000 bytes and
whose aggregate arrays are 200,002 bytes. The current production overlay reads
an effective retained total of 4,000,020 bytes, below the 4 MiB run limit, while
the exact retained 250,000-byte candidate would raise it to 4,250,020 bytes.
Before the report, normal discovery excludes `c10`, explicit advance waits, and
an exact fenced native `existing_outputs` drain is rejected without changing
the execution. The successful report accepts the original live lease, records
the data-limit stop and fixed refusal receipt, and does not retain the candidate.
Normal current discovery then starts the `stopped_for_run_data_limit` drain and
preserves the recorded success and original compensation target. Completion of
the real `line.release` task permits the run to reach `COMPENSATED`. The final replay returns the stored
original acknowledgement and receipt.

Select
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

Select the independently named cancellation-first consumer and artifact
namespace with:

```sh
CARGO_TARGET_DIR=/absolute/private/target \
SAGAT_TEST_DATABASE_URL='postgres://…/postgres' \
SAGAT_Q10C_OWNED_POSTGRES=1 \
scripts/test-legacy-collection-inflight-report-v1.sh --case cancellation-before-report
```

That case sends the public `CANCEL` operation as the retained `alice` principal
in namespace `payments`, with the actual pre-cancellation journal version and a
stable idempotency key. It proves the public stop, audit, HTTP trace and receipt;
the normal cancellation drain and live-member wait; current-projection replay
of the public operation; the exact original successful report and receipt; and
one real compensation with cause `RUN_CANCELLED`. The final run and collection
are `CANCELLED`; the completed saga records `COMPENSATED`.

Run both owner and least privilege API modes with an explicitly owned PostgreSQL
server and a private absolute Cargo target:

```sh
CARGO_TARGET_DIR=/absolute/private/target \
SAGAT_TEST_DATABASE_URL='postgres://…/postgres' \
SAGAT_Q10C_OWNED_POSTGRES=1 \
scripts/test-legacy-collection-inflight-report-v1.sh
```

The coordinator builds both test binaries before creating the retained lease.
The default uses `sagat-q10c-ordinary-before-drain-v1`; the under-limit increment
case uses `sagat-q10c-ordinary-under-limit-increment-v1`; the selected drain-first
case uses `sagat-q10c-ordinary-drain-before-report-v1`; and the selected public
cancellation case uses `sagat-q10c-ordinary-cancellation-before-report-v1`. Each
has distinct log labels.
It writes the credential bearing receipt under a mode `0700` directory as an
atomic mode `0600` file, and removes the receipt and owned test database after
each phase. Logs and the nonsecret manifest contain hashes and process results,
never the lease or session values. Each case invokes a fresh old producer and
one exact current consumer in owner mode, then repeats both processes against a
fresh database as the least privilege runtime role.

These cases establish only the two ordinary `E > limit` report/data-limit drain
orders, one ordinary `E <= limit < E + candidate` report-before-drain order, the
public-cancellation-first order, and public engine compensation semantics. They
do not establish an external provider effect, external compensation, exactly
once behavior, Applied reconciliation, a post ACK process restart,
output-dependent compensation, secret or blob policy, or the remaining
authority, race, and fault matrix. A passing run does not close the full Q10c
row or the open feature 014 release tasks.

## Explicit only-live-work frontier

`--case live-work-frontier-red` adds a separate immutable-old control. Immediately
after genuine ordinary setup (L2100000,V2100020,E4200020), it proves the original
uncancelled leased c10[0]/reserve is the only active member: no stop, failure,
pending Return, checkpoint or free admission slot. Old public advance_collection
actually returns Waiting without changing execution/source witnesses. The fixed
marker observed_legacy_live_work_waiting=true precedes an intentionally failing
DrainStarted assertion. Exactly one failure/exit101 is required per role; a setup
error, timeout or different result is not that RED. No private receipt is published
and no current consumer runs for either RED database.

`--case live-work-frontier` uses the existing ordinary producer and unchanged
19-field q10c-ordinary-before-drain-v1 handoff. The new current wrapper verifies the
same actual frontier and makes public advance_collection its first Run execution
operation, requiring exactly DrainStarted. Admission/member/execution inventories
must remain unchanged apart from cancellation of the original live lease. It then
reuses the accepted original report, replay, real compensation and verification
lifecycle. The existing discovery-tick drain-before-report test remains separate.

Original selectors/formats/role meanings stay intact. New phases have a300-second
process deadline and overall2400-second supervisor plus5-second termination grace.
Cleanup uses a fresh strict single-new-database proof, checked safe receipt/RED
identity and bounded administration; ambiguous or failed identity/hash caching
refuses DROP. Private receipt and .receipt.UUID.tmp files are removed before DB
administration, after active phases are terminated/reaped. A published/failed-cache
receipt cannot be treated as a pre-receipt failure. Actual source/restoration/
executable/role/count/cleanup results belong in project-completion evidence; these
commands alone are not passing results.

```sh
scripts/test-legacy-collection-inflight-report-v1.sh --case live-work-frontier-red
scripts/test-legacy-collection-inflight-report-v1.sh --case live-work-frontier
```

Use only the repository-declared healthchecked Compose fixture and an exclusive
private CARGO_TARGET_DIR outside checkouts. This adds §11.4(c) observation only;
CAS, equality/precedence, OS restart, rollout, provider undo and nesting remain
separate. Source certification/scoped Finish production activation stays closed.

## Original report versus stale frontier

`--case report-frontier-cas` reuses the ordinary immutable-old producer and its
unchanged private19-field session/lease/request handoff. No registration, repoll,
renewal or rebuilt completion occurs. With E4200020 and a live original reserve,
a dedicated owner transaction locks the existing telemetry row. The real public
refusal report must be observed waiting there while it already owns the run;
a role-correct one-connection scheduler then reads the old frontier and is
observed waiting behind that report at the exact native drain run-lock query.
A pinned, distinct API-role observer checks both edges in fresh autocommit reads.

Only then does the holder roll back. Original success must commit and deliver its
ACK; stale public advance must return exact native Primitive RevisionConflict with
the independently persisted report revision and no drain effect/metric. Fresh
store/cache advancement must select stopped_for_run_data_limit, preserving the
original stop. The accepted original-target/key compensation, held nonfinality,
terminal COMPENSATED state, original-ACK replay and fresh verification follow.
Stronger existing under-limit lifecycle witnesses also apply to this ordinary CAS
case, with ordinary totals. The refused-report counter advances once; only the
fresh committed drain increments the primitive rejection counter.

The new90-second proof budget does not cancel cleanup: holder and actor handles
remain owned outside it. Holder begins cancel-safely; identities include backend
start to prevent PID reuse. Owner cleanup observes owner holder; API-role observer
observes same-role report/scheduler and is released before API-pool closure. Every
error releases the row barrier, joins or cancels identified actors, checks server
transaction closure and bounds pool shutdown before surfacing failure. Emergency
termination targets only a verified owned identity and always fails acceptance.

```sh
bash scripts/test-legacy-collection-inflight-report-v1.sh --case report-frontier-cas
```

This selector has300s native phases and the existing2400s whole supervisor with5s
termination grace. All seven selectors must be rerun after shared lifecycle edits.
This establishes only original-success/frontier CAS and fresh recovery, with no
production/producer/schema/grant/SDK change or broader recovery/nesting guarantee.
