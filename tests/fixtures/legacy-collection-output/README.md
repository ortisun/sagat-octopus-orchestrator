# Immutable-old collection output fixtures

The coordinator `scripts/test-legacy-collection-output-view.sh` archives immutable
`6c44a0e49670cf19660792cd1518d241e6c99b3b`, overlays only its test module,
builds old/current executables sequentially, restores every archived source byte,
and records source, migration, toolchain and executable hashes.

Selectors are closed:

- `output-view` (default): the existing seven-run Q5 matrix. Its old `Flow`
  constructor always uses `sagat_app`; owner/runtime refers to the current reader.
- `nonfinal-return`: a separate old producer and current consumer in each actual
  API/store role (`postgres` fixture owner, then `sagat_app`). Four positive test
  processes, each exactly one passing test.
- `nonfinal-return-red`: two isolated old tests, one per actual API/store role.
  Each must exit 101 with exactly one failing test and the explicit observed
  old `Checkpointed { index: 0 }` marker. These are expected failures, not passing
  Rust tests. Their databases are never consumed by current code.

## Nonfinal Return contract

Ten old root scopes c0–c9 each process two items at concurrency one. Twenty public
worker completions retain JSON strings of exactly 105,000 bytes. Each old aggregate
is 210,003 bytes; equal aggregate digests still count once per owner reference.
The next scope c10 has two items. Its first compensable work publicly succeeds
with JSON `1`; its Return is PENDING and item 1 remains unadmitted. The positive
producer stops before any additional old scheduler transition.

Actual stored logical output L = 2,100,001 bytes, virtual legacy owner output
V = 2,100,030 bytes, E = 4,200,031 > 4,194,304. All individual values fit 256 KiB.
Current normal frontier advancement must select `DrainStarted` before that
nonfinal Return can checkpoint. It records first FAILED/RUN_DATA_LIMIT,
`existing_outputs` audit evidence and a truthful public FAILED drain hint.

Ordinary member drain cancels the Return, releases the slot and plans exactly one
`line.release` compensation. Its lease targets the original work/path/key and
its input includes `/steps/work/output = 1`. A held compensation prevents early
finalization. Public successful completion leads to Run COMPENSATED and scope/
owner FAILED with RUN_DATA_LIMIT, without item-1 admission, a c10 checkpoint,
owner output, new payload or ledger backfill. Native DRAINED settlement increments
`completed_count` to **1**; the original design snapshot's terminal value 0 was
incorrect. Initial and first-DrainStarted counts remain 0.

The private `legacy-nonfinal-return-frontier-v1` receipt has exactly 20 fields:
`format`, `source_commit`, `case`, `role`, `database_name`, `database_url`,
`runtime_database_url`, `tenant`, `other_tenant`, `namespace`, `run`,
`workflow_version`, `scope_id`, `scheduler_owner`, `handoff_run_version`,
`closed_source_cutoff`, `forward_step_id`, `forward_target_key`,
`producer_witness_sha256`, `closed_source_sha256`. It contains fixture database
access details; directories are 0700 and receipts 0600. No old worker credential
is handed off. Logs retain safe identities and receipt hashes only.

## Reproduction

Use this repository's healthchecked PostgreSQL Compose service on a dedicated
loopback port/project. Do not reuse another project's service. Set
`SAGAT_TEST_DATABASE_URL` to that owned fixture's admin URL,
`SAGAT_Q5_OWNED_POSTGRES=1`, and an absolute private `CARGO_TARGET_DIR` outside all
checkouts. Set `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`,
`CARGO_INCREMENTAL=0` and `CARGO_BUILD_JOBS=3` for the local resource profile.

```sh
SAGAT_Q5_CASE=nonfinal-return-red bash scripts/test-legacy-collection-output-view.sh
SAGAT_Q5_CASE=nonfinal-return bash scripts/test-legacy-collection-output-view.sh
SAGAT_Q5_CASE=output-view bash scripts/test-legacy-collection-output-view.sh
```

New fixture phases have a 300-second process deadline; their entire coordinator
has a 2,400-second deadline plus five-second termination grace. The old driver
has a 256-transition bound and the current drain a 32-transition bound. Database
administration has connection/statement/lock and process deadlines. Cleanup
terminates and reaps active phases before dropping only the single new test
database proven created after that phase started. Ambiguous ownership or cleanup
failure produces an error and retains evidence for inspection.

This is narrow pending-nonfinal-Return upgrade acceptance. It does not establish
other frontier shapes, concurrent/CAS controls, performance bounds, rollout,
provider-side undo or general nested capacity. Source certification and scoped
Finish production activation remain closed. Canonical project tasks stay open
until their complete gates pass. Captured outcomes belong in project-completion
evidence; a command listed here alone is not a passing result.


## Free-admission frontier

`free-admission` and `free-admission-red` are separate closed selectors. All eleven
owners use plain legacyLine, without compensation/effect declarations. Exactly20
old public successes retain105000 bytes each; ten aggregate references retain
210003 bytes each. L=2100000,V=2100030,E=4200030>4194304.

After success20, the old producer stops generic ticking. Exact normal calls
checkpoint c9 index1, finish c9, then enter c10. Handoff has Run WAITING, c10 OPEN,
two captured indices/values [[0,1],[1,2]], zero progress/members/reservations/tasks.
Its separate RED calls old normal advance once more: actual Admitted{index:0}
emits observed_legacy_free_admission_admitted_zero=true and intentionally fails
the expected DrainStarted assertion. Each such Rust process must exit101 with
one failure; its database never becomes a current-consumer input.

Current must return exactly DrainStarted, DrainFinished (Run/scope/owner FAILED
with original RUN_DATA_LIMIT), then AlreadyFinished after fresh verification.
No admission, task, Return, checkpoint, compensation, new payload or source repair
is allowed. Exact stop/drain/finish journals, native outboxes, original trace,
existing_outputs audit and immutable Run/source/byte inventories are asserted.
This adds only free-admission acceptance; other frontiers/CAS/rollout remain open.

The private legacy-free-admission-frontier-v1 receipt contains exactly18 fields:
the nonfinal format's20 minus forward_step_id and forward_target_key. Its case is
free-admission, owner free-admission-role and head exactly closed_source_cutoff+1.
It uses the same private file, ownership, deadline and credential-first cleanup
rules. Actual results are retained in project-completion evidence; commands alone
are not passing results.

```sh
SAGAT_Q5_CASE=free-admission-red bash scripts/test-legacy-collection-output-view.sh
SAGAT_Q5_CASE=free-admission bash scripts/test-legacy-collection-output-view.sh
```

## Exact4MiB positive admission control

`free-admission-equal-limit` uses the same genuine old public20-completion and
c9checkpoint/finish→c10entry boundary, without another old call. First19 JSON
values retain105000 bytes; the last retains102137. Nine aggregate references are
210003bytes and the last207140: L2097137,V2097167,E4194304 exactly. Actual payload
contents/digests and every checkpoint/owner identity are verified. The separate
private18-field format is legacy-free-admission-equal-limit-v1, with matching case;
it inherits the same role/ownership/hash/private-file/cleanup contract.

Current first public advance must be Admitted{index:0}. It creates exactly one
original item0 READY instance/member, one held slot/reservation, the two native
admission/release events and one task.ready outbox. Charged/admitted counters
advance once. Exact historical source/output/payload/audit/receipt inventories
remain unchanged; no stop, drain, item1 or Return is created. Fresh verification
has no durable effects. The full free-target source hash legitimately changes
with admission counters; closed source and the exact permitted target delta are
checked separately. This control stops after valid admission and owned DB cleanup;
it proves admission progress, not workflow completion. No worker poll/effect occurs,
and no observed wire idempotency key is claimed. Current strict > already permits
equality; no artificial failing RED or production change is introduced.

```sh
SAGAT_Q5_CASE=free-admission-equal-limit bash scripts/test-legacy-collection-output-view.sh
```

Run all six closed selectors after shared fixture edits. Actual outcomes belong
in project-completion evidence. Parent quota/frontier/CAS/rollout/nesting and
canonical task acceptance remain separate.

## Actual failed-member precedence over excess

`failed-member` adds one positive control. After the same genuine old20-success
boundary, old public advancement admits c10[0]/legacyWork. The already registered
worker polls that exact lease and submits nonretryable Q5_MEMBER_FAILURE through
real FailStep, then replays the exact request. ACK is FAILED/WAITING. No additional
old collection transition occurs. The six-event suffix after c9 routing is entry,
admission, release, lease, failure and empty routing: head=cutoff+6. Run has no stop;
c10 is OPEN with one terminal FAILED/routed/no-output entry, one ADMITTED member,
one held slot/reservation and no Return. L2100000/V2100030/E4200030 remain exact.

The dedicated legacy-failed-member-frontier-v1 receipt has the same closed18
non-capability fields, case failed-member and observed head. Its stable source
adds the original failed step/reported hash, exact attempt-scoped canonical receipt,
original member identities and five admission/claim/failure events. Changing active/
completed/drain fields are checked through exact projection deltas. Worker session,
lease and request never cross the handoff or enter logs/evidence.

Current must select DrainStarted for that actual member failure before the excess
check, then Drained0, DrainFinished and AlreadyFinished after fresh verification.
The first custom-code FAILED stop remains unchanged. Exact +2/+1/+2 journal and
1/1/2 outbox writes preserve original trace/scope/identity; capacity settles and
uncreated Return reservation retires. There is no data-limit audit, new step,
checkpoint/output/item1/compensation or source repair. This proves real failed-member
selection and first-stop preservation, without fabricating a prior FAILED Run over
an empty OPEN scope. Existing Q10 cancellation-before-report covers literal prior
CANCELLED stop precedence and is rerun separately. No artificial RED or production
change is introduced; both implementations already prioritize actual failure.

```sh
SAGAT_Q5_CASE=failed-member bash scripts/test-legacy-collection-output-view.sh
```

Run all seven Q5 selectors plus the existing Q10 cancellation-before-report gate
after this increment. Actual results belong in project-completion evidence. General
CAS/restart/rollout/nesting and canonical task acceptance remain separate.
