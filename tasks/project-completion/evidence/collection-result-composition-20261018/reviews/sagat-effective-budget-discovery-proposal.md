# Q9a dormant root collection effective-budget discovery proposal

Status: bounded design only; no repository edits, SQL execution, builds, formatting, tests, or commits. The accompanying one-file patch is unverified and needs root's genuine RED, separate review, and actual default/runtime-role validation. Parent agreed that discovery detects contradictions in metadata it reads, without cloning the complete authoritative verifier or promising an immediate corruption watcher for otherwise fitting Waiting runs.

Patch: `/tmp/sagat-effective-budget-discovery-proposal.patch`.
Repository: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion`.
Only proposed production file: `crates/infra-postgres/src/maintenance.rs`.

## Selected change

Extend the existing OPEN collection candidate disjunction with a root-only hint. Read the run's existing logical byte ledger and successful root scope result lengths, charging each owner whose root step has NULL output_digest. Select when the hint is over MAX_RUN_OUTPUT_BYTES, the usage row is absent/negative, more than MAX_NODES root successes are encountered, or one of the read joins/metadata facts is missing or contradictory. Preserve the current unentered PENDING arm, DRAINING arm, cancellation/admission/return/finish conditions, shard predicate, excluded-tenant predicate, old-first ordering and both per-shard/global limits.

This fixes the genuine frozen shape L=2,100,000, V=2,100,020, E=4,200,020 (>4,194,304) even when c10's member is RUNNING at full concurrency and no Return/admission is actionable. The query emits the active root owner c10, not one of the closed c0..c9 source owners. All accepted legacy owner node kinds (`forEach`, `legacyControl`) remain candidates. Every successful root owner contributes separately: no DISTINCT on result digest or byte length.

For valid fitting old histories the new disjunct is false, including E exactly equal to the limit. They do not permanently occupy the limited queue. Filtering is inside the existing per-shard candidate subquery, before ORDER BY/LIMIT; do not fetch the oldest arbitrary legacy runs and filter afterward. No backfill, historical row rewrite, cache flag, cursor, new table/index, new write, notification, grant, publication shape or nesting is added.

## Exact input parameters

1. `$1`: existing `i64::from(window.limit)`; shard-local and final output limit.
2. `$2`: existing `window.shards.indexes()` as smallint[]; only owned shards.
3. `$3`: existing excluded tenant UUIDs; current fairness boundary.
4. `$4`: checked i64 conversion of `sagat_application::execution::MAX_RUN_OUTPUT_BYTES`, currently 4,194,304.
5. `$5`: checked i64 conversion of `sagat_domain::workflow::MAX_NODES`, currently 10,000. Root-source subquery reads at most `$5 + 1`; count above `$5` dispatches to fail-closed verification rather than silently truncating the total.

The arithmetic uses PostgreSQL numeric (`SUM(bigint)` yields numeric and the comparison casts both limit and ledger), so the hint cannot silently wrap. `COALESCE(sum(...),0)` covers no legacy outputs; absent payload rows are independently detected, so NULL lengths are not silently treated as proof of zero charge. Only output length is used: no payload content column or JSON value is returned to Rust, no payload JSON parsing/hydration occurs, and no journal or definition body is added to this new hint.

## Join and corruption boundaries

The source set is every SUCCEEDED root scope for the same tenant/run, not a join filtered to only already-valid owners or matching namespaces. Source scope namespace contradictions are an explicit candidate condition. Root owners and payloads are LEFT JOINed; absent owner, absent result/payload, malformed result digest, wrong owner namespace/kind/node kind/status/routing, or a present owner output that disagrees with the scope result yields a candidate. The usage join is also LEFT, so missing usage is visible.

These checks are a conservative discovery hint only. No SQL total or source row is promoted into trusted effective bytes or drain evidence. The query deliberately does not duplicate deterministic owner identity, pinned definition shape, complete scope counts/drain metadata, finish/route journal, or aggregate body verification. A corruption solely in unexamined evidence on an otherwise fitting dormant run is not promised immediate discovery. Once dispatched, the existing shared verifier refuses bad evidence without mutation; a corrupt candidate can remain discoverable/report errors under existing fairness rather than being silently dropped. No new automatic corruption repair or starvation guarantee for permanently corrupt data is introduced.

## Authority and races

`collection_outputs::inspect` is the shared metadata/length verifier. `scope_frontier::header` calls it within the same repeatable-read transaction as run revision, stop and collection/member metadata. The production frontier chooses ExistingOutputs before Waiting/admission/nonfinal checkpoint, while preserving terminal/draining/prior-stop/actual member-failure precedence.

`primitives::validate_data_limit_evidence` recomputes through `effective_output_snapshot` and the same helper under the existing chain/own-run lock, scheduler fence and revision CAS. Query results never authorize a mutation. A stale candidate is re-read, and fitting current data cannot pass ExistingOutputs native validation. No full `RunTx::data()` overlay hydration is added.

## Existing indexes actually found

- `iteration_scopes_owner_uniq`: UNIQUE `(tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node)` from migration `20261029000000_collection_progress.up.sql`. Supports the same-run/root-range lookup and owner ordering; lifecycle is a filter, not an indexed key.
- `iteration_scopes_pkey`: PRIMARY KEY `(tenant_id, run_id, scope_id)` from that migration; alternate same-run prefix access.
- `iteration_scopes_open_owner_idx`: `(tenant_id, run_id, owner_path_nodes, owner_path_indices, owner_node) WHERE lifecycle IN ('OPEN','DRAINING')`, migration `20261205000000_bounded_execution_discovery.up.sql`. Useful for open owner metadata; does not cover SUCCEEDED root history and does not lead with shard or updated_at.
- `steps_pkey`: PRIMARY KEY `(tenant_id, run_id, instance_path_nodes, instance_path_indices, node)`, replaced by migration `20261027000000_scoped_instance_identity.up.sql`. Exact root owner probe.
- `payloads_pkey`: PRIMARY KEY `(tenant_id, digest)`, migration `20261009000100_execution.up.sql`. Exact aggregate payload length probe.
- `run_instance_usage_pkey`: PRIMARY KEY `(tenant_id, run_id)`, migration `20261029000000_collection_progress.up.sql`. One ledger probe.
- `runs_tenant_id_uniq`: UNIQUE `(tenant_id, id)` and `runs_pkey(id)`, migration `20261009000100_execution.up.sql`. Existing own-run join.
- Existing unchanged member path: `collection_iterations_actionable_idx` `(tenant_id,run_id,scope_id,lifecycle,iteration_index) WHERE lifecycle IN ('ADMITTED','CHECKPOINTED')` and `steps_pkey`; current return/failure condition remains as is.

No existing iteration_scopes index was found with `(run_shard(run_id), updated_at)` or a precomputed effective budget. The generic maintenance comment claiming one shard-first index probe does not describe this collection UNION's full physical cost. This patch preserves bounded returned queues and adds bounded per-owner history lookup, not a new constant-time discovery guarantee. Root may clarify that existing comment within the same file when integrating.

## Cost and validation limits

For each otherwise examined root OPEN scope: one usage row plus up to 10,001 successful root scope rows, corresponding exact owner rows and payload length probes. Payloads may share a digest; repeated references still count logically even if physical index/cache reads are reused. The query transfers only the existing StepRef metadata result. Database heap/TOAST metadata and index reads still cost work; no EXPLAIN, buffer count or physical-read claim has been measured here. PostgreSQL may reorder OR evaluation, so do not promise that ordinary actionable collections always avoid the added subquery. The outer scan can examine many fitting runs before finding a full candidate window, as with current semantic candidate filtering; the patch does not bound total database work by window.limit.

Root's first proof should be the frozen producer Scheduler::tick RED/GREEN draft, using normal Keeper renewal even if same-owner leases remain live. Run the fitting-delta case before global scheduler maintenance mutates its leased fixture. Also retain/establish fitting/equality exclusion with more old fitting owners than the candidate limit, owned-shard/tenant exclusion, equal-digest per-owner counting, and missing/contradictory joined metadata dispatch with authoritative no-write refusal. These are requested gates, not executed results. A method-length Clippy allowance or equivalent constant extraction may be needed when root applies the SQL; no formatter or compiler was run here.
