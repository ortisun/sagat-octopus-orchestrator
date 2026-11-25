# Run list query language (draft, feature 020)

Draft contract for FR-020-007..009. Merged into `docs/reference/` and the OpenAPI descriptions of
`searchRuns`/`countRuns` at implementation (T020-012). Nothing here is implemented.

## Grammar (EBNF)

```
query        = [ expr ] [ order ] ;
expr         = term { "OR" term } ;
term         = factor { "AND" factor } ;
factor       = [ "NOT" ] ( "(" expr ")" | predicate ) ;
predicate    = attr cmp literal
             | attr [ "NOT" ] "IN" "(" literal { "," literal } ")"
             | attr "BETWEEN" literal "AND" literal
             | attr "STARTS_WITH" string
             | attr "CONTAINS" string            (* keywordList: any element equals *)
             | attr "MATCH" string               (* text: every token of the string matches *)
             | attr "IS" [ "NOT" ] "NULL" ;
order        = "ORDER BY" attr [ "ASC" | "DESC" ] ;
cmp          = "=" | "!=" | "<" | "<=" | ">" | ">=" ;
attr         = system | custom | "`" custom "`" ;
system       = PascalCase name from the system table below ;
custom       = /[a-z][a-zA-Z0-9_]{0,63}/ ;
literal      = string | integer | decimal | "TRUE" | "FALSE" ;
string       = "'" { char | "''" } "'" ;     (* UTF-8; '' escapes a quote *)
```

Keywords are case-insensitive; attribute names are case-sensitive. Whitespace is insignificant. No
functions, no arithmetic, no `LIKE`, no sub-queries, no comments. An empty query lists every run of the
namespace newest first (the order of `GET /runs`).

## Types and operators

| Type | Literal | Operators | ORDER BY |
|---|---|---|---|
| keyword | string | `= != IN NOT IN BETWEEN STARTS_WITH IS NULL` | yes |
| text | string | `MATCH IS NULL` | no |
| int | integer (i64) | `= != < <= > >= IN BETWEEN IS NULL` | yes |
| double | decimal or integer (finite) | `= != < <= > >= BETWEEN IS NULL` | yes |
| bool | `TRUE`/`FALSE` | `= != IS NULL` | yes |
| datetime | RFC 3339 string with offset | `= != < <= > >= BETWEEN IS NULL` | yes |
| keywordList | string | `CONTAINS`, `IN` (any element), `IS NULL` | no |

A literal of the wrong type is `VALIDATION_FAILED`/`QUERY_TYPE`. `MATCH` tokenizes with PostgreSQL's
`simple` configuration (lower-cased words, no stemming, G1092) and requires every token.

## System attributes

| Name | Type | Source | Gate |
|---|---|---|---|
| `RunId`, `WorkflowId`, `ParentRunId`, `RootRunId`, `RerunOf` | keyword (UUID) | `runs` (`id`, `workflow_id`, `parent_run_id`, `root_run_id`, `rerun_of`) | `run:read` |
| `ChainId` | keyword (UUID) | `chain_segments` (the chain's initial run id; `runs` has no chain column, *corrected 2026-11-24*) | `run:read` |
| `WorkflowName` | keyword | `workflows` metadata name | `run:read` |
| `WorkflowVersion` | int | version number | `run:read` |
| `Status` | keyword | `runs.status` (OpenAPI `Run.status` values) | `run:read` |
| `Origin` | keyword | `runs.origin` (`AD_HOC_STEP`, `AD_HOC_WORKFLOW`, `CONNECTION_PROBE`; null for an ordinary run, so `Origin IS NULL` selects those) | `run:read` |
| `StartedAt`, `FinishedAt` | datetime | `created_at`, `finished_at` | `run:read` |
| `Duration` | int (milliseconds) | `finished_at - created_at`, null while running | `run:read` |
| `CorrelationId`, `StartedBy`, `TraceId` | keyword | `runs` | `run:read` (exposed by `getRun` today) |
| `ArchiveState` | keyword | `runs.archive_state` | `run:read` |
| `BusinessId` | keyword | `runs.business_id` (018, built: `20270103000000_business_ids`) | `run:read` (as 018 exposes it) |
| `ScheduleId` | keyword (UUID) | `runs.schedule_id` (018, built: `20261230000200_schedule_run_provenance`) | `run:read` |

When 029 is enabled, the runs a search, count or saved view can return are first restricted by the 029
evaluator (folder visibility); a saved view never widens it. Custom attributes follow their registry
level (FR-020-009): `public` needs `run:read`; `internal`
also `payload:read-internal`; `pii` also the reveal grant and `reveal: ["pii"]` in the request.
Using an attribute in a predicate or `ORDER BY` without its right refuses the search. A requested column
without its right is answered with `{withheld: <level>}` values; `pii` values are returned only with the
grant, `reveal: ["pii"]` and the `run.search_pii` audit.

## Bounds (defaults / hard maxima; plan, Finite ceilings)

Query text 2 KiB / 4 KiB; predicates 16 / 32; `IN` values 64 / 256; nesting depth 3 / 4; string literal
256 bytes / 1 KiB; page size 50 / 200; statement budget 5 s / 15 s; count ceiling 10,000 / 100,000.
Bounds are checked on the AST before any SQL is prepared.

## Semantics

- `NOT` and `!=` exclude runs whose attribute is null (SQL three-valued logic is not exposed: a missing
  attribute never matches `=`, `!=`, `<` …; only `IS NULL` matches it).
- `OR` across different custom attributes is allowed; each custom predicate compiles to an `EXISTS` over
  `run_search_attributes` with the namespace prefix.
- `ORDER BY` a custom attribute orders nulls last in both directions; the tie-break is `RunId DESC`.
- Archived stubs (FR-020-035) match system predicates only; their custom attributes no longer exist.
- The cursor is an opaque, integrity-protected token of `(namespace, query digest, order, last sort
  value, last run id)`; any other use is `BAD_REQUEST`/`CURSOR_MISMATCH`.
- Results are a snapshot of committed state at statement time (READ COMMITTED); attribute values are
  transactional with the run (D1081), so a run is visible with its start values as soon as its `202`
  is returned.

## Examples

```
Status IN ('FAILED', 'PARTIAL') AND customerTier = 'gold' ORDER BY StartedAt DESC
orderTotal BETWEEN 100 AND 500 AND StartedAt >= '2026-10-01T00:00:00Z'
tags CONTAINS 'priority' AND NOT (region = 'eu-west' OR region IS NULL)
notes MATCH 'refund requested'
```

## `RunSelector` — unattended selection for 018 batch operations (co-owned with 018)

Application port `application::run_search::RunSelector`, the only way 018 (or any later feature)
selects runs by a query without a person waiting. It uses the same parser, compiler, bounds and gating
as `searchRuns`; differences are stated here. Changes need the agreement of both features.

| Aspect | Rule |
|---|---|
| Identity | Evaluated with the **job creator's current rights**, re-resolved at every round (membership, role, `payload:read-internal`, reveal grant). A right lost between rounds stops selection with `FORBIDDEN` and the reason of the lost right; 018 records it on the job. |
| Paging | Keyset pages by `RunId` ascending (UUIDv7, creation order), restricted to `StartedAt <= selection_bound_at`; the cursor is the last run id looked at, which 018 stores in `batch_jobs.select_cursor` (the job's selector fixes the query, so the cursor needs no query binding of its own); each page at most 500 runs; each round one page in one short read transaction. *Corrected 2026-11-24* from a `(StartedAt, RunId)` keyset, to match 018 as built (research Decision 020-006). |
| Snapshot | "Evaluated per round; fixed once materialized": a run is selected if it matches when its page is read. To include every run whose transaction began before job creation, 018 records the creating transaction's identity (`selection_horizon`, `pg_current_xact_id()`) and starts the first round only when `pg_snapshot_xmin(pg_current_snapshot()) > selection_horizon`, so every transaction that had an identity at creation has finished (the horizon rule of ADR-0053). *Corrected 2026-11-24:* the draft's `xmax` is wrong (it is the last completed identity plus one and lets an open transaction pass), as 018's migration `20270102000000_batch_operations` explains. |
| Bounds | No total ceiling of its own: 018's `maxItems` (up to 100,000) bounds the job. The count ceiling of `countRuns` does not apply. Query bounds (length, predicates, `IN`, depth) and the statement budget apply per page. |
| Preview | `preview(query, cap)` returns `count = min(matches, cap)`, `exact: matches < cap` and up to 20 run ids; 018 passes `cap = maxItems + 1` (at most 100,001) under the same statement budget; exceeding the budget is `QUOTA_EXCEEDED`/`QUERY_BUDGET_EXCEEDED`. |
| Classification | Predicates on `internal`/`pii` attributes need the creator's rights as in `searchRuns`. Selection returns run ids only, never attribute values. |
| Visibility | When 029 is enabled, the 029 evaluator decides which runs the creator can see, at every round, before classification gating. Draft test runs (022 `DRAFT_TEST`, `test_tree`) are never selected. |
| `pii` audit | A query with a `pii` predicate requires the creator's reveal grant at job creation and at every round, and writes one `run.search_pii` record per round (actor = creator, `on_behalf: batch-job/<id>`), never per run, because no value is returned. |
| Without 020 | 018 uses `RunFilter` (workflow, status, origin and its own structured filter); a job created with a query is refused `VALIDATION_FAILED`/`QUERY_UNSUPPORTED_OPERATOR` when the capability is off. |
| Purged runs | Stubs match system predicates; 018 actions that need content (rerun, reset) are refused per run with `CONFLICT`/`RUN_PURGED`. |
