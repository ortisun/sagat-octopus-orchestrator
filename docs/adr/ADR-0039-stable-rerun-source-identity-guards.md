# ADR-0039: Stable rerun source identity guards

Status: **Accepted scoped correction** | Date: 2026-10-18.

Refines ADR-0030 and ADR-0037's foreign-run identity boundary. Refs: T014-014..017/040,
FR-014-012–015, T017-012/024. This is one bounded correction discovered in B0; it does not
accept the nested-capacity protocol or enable nested runtime.

## Context

A rerun creates an unrelated root and references its existing source through runs.rerun_of.
The INSERT's foreign-key KEY SHARE check previously followed managed namespace Secrets quota,
active-run admission and payload retention. Terminal credential cleanup instead locks its
source run FOR UPDATE before Secrets quota. Existing managed report receipt writers also
retain their own run FOR UPDATE before the catalog. These opposing edges must be addressed
without weakening their aggregate mutation guards.

## Decision

After checking for an existing idempotency receipt, RunStore::create takes exact tenant/source
KEY SHARE before retained private-definition admission, managed admission, active-run quotas,
payload retention or new run insertion. Retain this guard in the creation transaction through
commit/rollback. Select only source identity; do not read mutable source business state or
upgrade the guard. The existing foreign key remains in place. A missing source returns the
existing MissingReference store error (application NotFound), including cross-tenant absence.
Receipt replay retains its current authorization checks and does not acquire this new guard.

Current production rerun writers create a new root (parent=None, depth=0, root=self). Current
child writers set rerun_of=None. This mutual exclusion is a **writer invariant**, not a database
constraint or a NewRun type guarantee. The source may itself be a child or historical segment;
its identity guard does not authorize its state or transfer its chain, parent or group ownership.
A future writer combining rerun ancestry and child ancestry requires a new ordering review.

Keep terminal cleanup FOR UPDATE SKIP LOCKED. While a rerun holds source KEY SHARE, cleanup
skips that source; it must not wait holding another admission guard. Full cleanup may subsequently
visit provider operations with separate catalog locks, so the focused race exercises native pin
release and the positive retention control subsequently executes full cleanup.

## Consequences and alternatives

The guard advances an existing FK wait to the creation entry. No source run is mutated, no new
scheduler authority is granted, and no external provider I/O is added to the transaction. Source
writers may delay rerun admission; cancellation rolls back the transaction and its retained guard.
The existing source FK already prevents deletion after successful creation. No migration, new
API field, SDK fork, error code, group lock or external-effect guarantee is introduced.

Weakening only cleanup to NO KEY UPDATE would leave other source run FOR UPDATE → Secrets writers.
Moving source checks outside the owning transaction leaves the late FK wait. Sorting unrelated
run IDs or globally changing aggregate lock modes would broaden the protocol without proof.

## Acceptance evidence

`tasks/project-completion/evidence/rerun-source-identity-20261018/` retains the exact test source,
production diff, raw logs, command exits/times and source hashes. Unmodified production failed
the ordered runtime-role/real PostgreSQL/OpenBao test: Secrets probe unavailable while waiting
on source, PostgreSQL deadlocks 0→1, rerun succeeded and native cleanup returned Unavailable.
This is an observed deadlock; no counter or other source-writer cycle is claimed as reproduced.

Required positive controls: same barriers after correction; cleanup skips pinned source;
missing reference retains error class and atomic counts; cancel while source-blocked rolls back;
retry same key creates one new rerun; full cleanup later releases original pin; new rerun can
claim, resolve current credentials, finish and eventually release its own pin. Preserve original
live/manual-review/current/grace retention assertions. Record actual gate results before acceptance.

### Executed verification

Final full connections3/3,27.36s (51.44s command including compile); segment rerun1/1,1.95s.
Final strict locked workspace/all-target Clippy passes2.87s, format4.11s and architecture24crates.
Three bounded fresh plan/code review cycles; final no concrete issues. The stronger credential
control first hit an INT4/i64 fixture decoding error; retained log/source and explicit test-only
bigint casts distinguish it from the earlier actual production deadlock RED. No fullworkspace
test/SDK58/100k/Compose/advisory/mixed-writer/group acceptance is inferred.
