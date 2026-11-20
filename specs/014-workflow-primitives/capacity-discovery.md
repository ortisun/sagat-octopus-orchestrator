# B1d.1 — bounded read-only discovery of vacant child-root groups

Refs: T014-014..017/040/042, ADR-0037, capacity-eligibility.md D703–D705. This is the first
small discovery component, not Vacate/Acquire, a scheduler caller or nested publication.
The separate branch reuses the verified 014/033/034 baseline so PR56 can finish CI unchanged.

Scan only persisted ACTIVE CHILD_ROOT memberships, in strict (tenant, group, member) UUID order.
A new partial index covers that order without walking retained FINISHED history. A requested page size is in 1..256; a page consumes
up to that many memberships plus at most one lookahead. Materialize this bounded input before looking up
group/current-chain/current-head metadata or applying the owned-shard and excluded-tenant filters.
Do not filter first and then spend unbounded work searching for an eligible member.

Return candidate metadata only for VACANT groups whose exact current head lies in the requested
shards and whose tenant is not excluded. Read current head/revision from the member's own chain;
never redirect to another namespace or tenant. Keep terminal/paused heads as discovery hints:
Finish or Acquire eligibility belongs to its own later fenced transaction. Include observed
group revision/epoch, chain revision and run revision; none is authority or a permit.

The page cursor advances through consumed input even if every candidate was filtered out. Return
has_more based on the unconsumed lookahead; after an exhausted scan a future caller may wrap in a
later bounded pass. Concurrent inserts/updates can be seen on a later scan; do not promise a
frozen multi-page snapshot, starvation freedom, bounded wall time or 100k performance acceptance.
Reject an invalid page size or more than 256 tenant exclusions with a typed error.

Discovery uses no row/advisory locks, writes, receipts, audit or outbox events, and changes no tenant
capacity counters. Use the existing runtime SELECT grants. No tenant policy check can suppress
already admitted members, including policy zero. Emit the scheduler action span with closed kind
and bounded scanned/selected counts, without payloads or credentials. Missing/contradictory binding
metadata fails closed with a typed storage error. Current actor/head/run/group authority must be
revalidated under ADR-0037 order before any future write; this component grants none of it.

Required native tests: runtime role; HELD/RETIRED/FINISHED exclusion; exact current head; shard and
tenant isolation; keyset pages with an empty filtered page and store reconstruction with a retained cursor; held group
row lock does not block the read; invalid input; no metadata/audit/outbox/usage changes. Fixtures
seed dormant registry metadata through the owner and are not evidence of production group creation.
The eight canonical tasks remain open until the complete runtime/resource/release gates pass.


## Implemented checkpoint — 2026-10-27 (local date)

`CapacityDiscoveryStore` on `PgRunStore` returns typed observed bindings and revisions. A materialized
keyset input is bounded before vacancy/shard/tenant filtering; the continuation cursor records the
last consumed membership, including an empty filtered page. Group revision/epoch remain positive;
chain and run revision zero follow their existing schema contract. Migration20261225000000 adds
only the partial covering index. It adds no writer grants, counters, schema capability or caller.

Nine real PostgreSQL tests pass: current head after actual child continuation, equal ids in two
tenants, shard/exclusion boundaries, quota zero after a binding exists, pagination/reconstruction,
no row-lock wait, unchanged execution/audit/outbox/usage, retained tombstones and index reversal,
typed invalid input and closed span counts. The complete capacity-filter regression passes43/0/0
in111.05s (111.78s command); the separate SQL privilege audit passes42/0/0 in12.82s. Counts overlap:
the nine discovery cases are included in the43, not added to them. The fixture-owned registry rows
prove metadata behavior, not production group admission or quota reservation.

The first two native tests were seen failing against an explicitly temporary empty stub. The
first implementation then rejected a valid revision-zero chain (1pass/1fail); existing migration
20261102000000 establishes revision>=0, so the decoder was corrected and the positive test pins0.
The next7-case and9-case gates passed. A wider gate passed42/failed1 because a scoped-downgrade
race assumed that its migration ran before any later unrelated DDL. The new index DOWN can wait
on its own member relation first. The test now removes later migrations before opening its exact
scoped-migration Run→G barrier, asserts that schema version, and retains every original lock,
commit, refusal and preservation assertion. The isolated case and the full43-case gate pass;
production index DOWN was not given an unnecessary run lock just to satisfy a test oracle.

Three bounded reviews ended without an actionable finding: contract, implementation and the
specific migration-test isolation correction. These do not reopen the previously capped whole
nested protocol review. The owner preference for the existing single-model review remains in force.

Strict workspace/all-target Clippy passes187.06s; after the final test-only correction, focused
integration-target Clippy passes45.09s. Primitive contracts49, SDK registry2 and developer reference
checks pass; the full SDK runtime aggregate is deliberately ignored in that contract-only command,
not counted as a pass. Format, architecture (24 crates) and agent configuration also pass.
All six immutable-v0.2 profiles pass with normal migration to20261225000000, including real old
reports, original-effect compensation, old readers and dispatcher/callback cold cutovers. Their
frozen current source-manifest SHA256 is
`2839d05ec8694df3abf303ab3472d86921c0ae48e2acece656ca531be8db4a91`.
[Exact commands, original failures and source/binary identities](evidence/capacity-discovery-20261027/artifacts.json)
are retained separately. Runner wall time includes builds/cache waits; no performance ceiling or
specific PostgreSQL physical execution plan is inferred from these functional tests.

Reproduce the focused behavior with the standard private target, owned Docker fixture and
`SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_workflow_primitives capacity_finish::discovery --locked`.
Use filter `capacity_finish` for the wider43-case gate. B1d.1 discovery is implemented; Vacate,
Acquire, scoped-path discovery, group creation/join, tenant-unit accounting/backfill, actor wiring
and final resource/release gates remain separate. The eight canonical tasks stay unchecked.
