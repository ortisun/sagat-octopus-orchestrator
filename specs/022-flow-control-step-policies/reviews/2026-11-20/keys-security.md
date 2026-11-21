# Security review: feature 022 key visibility (T022-026..029)

Reviewer: `security-reviewer` (read-only), 2026-11-20. Diff `b3351f2a` against `466c151e`, specs excluded. Judged against
FR-022-022, FR-022-057, NFR-022-002, NFR-022-006, D022-055/056, the `listConcurrencyKeys` contract and the constitution.
The reviewer ran nothing. (Report as returned; the reviewer's closing file list is omitted.)

## Summary

- **Blocker:** none.
- **Major (3):** 1. `keyLabel` goes to every reader, which breaks FR-022-057. 2. The cursor carries the full digest and is
  not signed, so any later fix to the label can be bypassed. 3. The new route is missing from the canonical OpenAPI, so
  the cross-tenant battery never tests it.
- **minor (2):** 1. No test covers the tenant and namespace filters in the new SQL. 2. A crafted cursor returns a 500
  instead of a 400.
- **nit (4).** The SQL tenant boundary, authorization order, injection, runtime-role grants and error text are clean.

## Major 1: `keyLabel` is shown to every `run:read` / `ops:read` caller, against FR-022-057

Where: `crates/api-http/src/dto_flow_control.rs:33` (`Run.concurrency`) and `:65` (listing): `key_label: Some(...)`
unconditionally; rationale in D022-055 item 4.

FR-022-057: the label goes only to a caller who may disclose, at its level, every location the key reads; everyone else
gets `null`. The code gives it to every holder of `run:read` or `ops:read`. Facts: an unannotated input field is `pii`
(`Level::UNMARKED = Level::Pii`, `crates/domain/src/classification.rs:41-42`); `pii` disclosure is granted per subject
and namespace, never by role (`PayloadRevealPii`, `identity.rs:314-315`); `viewer` holds `RunRead` and `OpsRead` but not
`PayloadReadInternal`; publication does not refuse `secret` keys or warn on `pii` keys yet (T022-104 open,
`FLOW_CONTROL_KEY_SECRET`/`_PII` still `PENDING` in `tests/contract/flow-control-step-policies/ir.rs:263-264`).

Failing scenario: with the switch on and the capability activated, a workflow declares `concurrency: {group:
"customers", key: "input.email", limit: 5}` (or a `secret` field). An operator without the reveal grant starts a run with
`input.email = guess`, reads its `keyLabel` and compares with the listing's or other runs' labels: a match confirms the
guessed value. A viewer links runs that share a customer value.

Mitigation: both switches are off by default and need fleet activation; the task text names the gap. Why Major: T022-026
is ticked done for 0.3.0 and nothing stops an operator from turning the switch on before T022-104.

Smallest fix (fail closed): emit `null` unless the caller holds the namespace's pii reveal grant until T022-104 can
classify the key's locations; T022-104 then relaxes per location. Alternative: refuse to start with the switch on until
T022-104 (owner's call). Tests: a viewer and a developer without a reveal grant get `keyLabel: null` in both reads; a
subject with the grant gets the label.

## Major 2: the cursor carries the full 32-byte digest and is not signed, which defeats any label redaction

Where: encoder `crates/application/src/flow_control_keys.rs:112`; decoder accepts any digest `:120-141`; order on
`key_digest` `crates/infra-postgres/src/flow_control.rs:480`; cursor emitted `dto_flow_control.rs:93`.

Today it adds nothing (the label is shown). Once Major 1 or T022-104 lands: (a) direct: with `?limit=1` every page but
the last returns the full digest of a key, so its label, even where the body says `null`; (b) bisection: the caller knows
pool, scope, group and the exact `oldestWaitSince`, forges cursors with digest D and watches whether the key still
appears after D; about 256 requests recover a digest bit by bit, the last key included.

Smallest fix: sign the cursor (HMAC-SHA-256 under a separate purpose of a server key, constant-time compare) and refuse an
unsigned or altered one with 400; encrypt it if `null`-label callers can page. Amend D022-055 item 2. Test: a cursor with
one digest byte flipped returns 400; a cursor from a `null`-label response does not contain the digest.

## Major 3: the route is live but missing from the canonical OpenAPI, so the cross-tenant battery never tests it

Route registered `crates/api-http/src/lib.rs:478`; absent from `specs/001-platform-foundation/contracts/openapi.yaml` and
from the charged-route list of `docs/runbooks/api-rate-admission.md:23-60`. The battery
(`tests/contract/security-multitenancy/rest/battery.rs:13-14`) is driven by that document; nothing compares the router's
routes with it. The route's own test (`rest.rs:249-263`) compares status codes only, without body equality, and the victim
namespace has no keys. No code defect (a foreign namespace is a 404 through the same path as a missing one,
`api_admission.rs:274-296` → `authz.rs:46-77`); an evidence gap against NFR-022-002 on a task marked done.

Fix: add `listConcurrencyKeys` and `Run.concurrency` to the canonical OpenAPI now (or a battery entry until T022-060); a
fitness check that every route registered in `lib.rs` has an OpenAPI operation; the route in the runbook's list.

## minor 1: no test covers the tenant and namespace filters of the new SQL

The code is correct (`flow_control.rs:475`, `QUEUED_RUNS_SQL`, the lateral's tenant predicates; digests separated per
tenant and namespace). Every `rest.rs` test has keys in one namespace of one tenant; deleting `s.namespace_id = $2` or
both predicates would pass. Test: keys in `payments`, `billing` and another tenant's namespace; each listing shows only its
own keys and `queuedRuns`; hand-mutate line 475.

## minor 2: a crafted cursor returns a 500 and an error-level log instead of a 400

`flow_control_keys.rs:131` accepts any instant `time` can represent (±9999 years); PostgreSQL's `timestamptz` starts at
4713 BC. `?cursor=k1.-300000000000000000.ORDINARY.RUN.<64 hex>.6163636f756e7473` decodes, PostgreSQL rejects the bind
(22008), the store returns `Internal`, an error is logged and the client gets 500. No leak, but any `ops:read` caller can
generate 5xx at will. Fix: refuse instants outside PostgreSQL's range in `decode` (`us >= 0` suffices: no row predates
1970); add the case to `a_cursor_it_did_not_write_is_refused`.

## nit

1. The canary checks response bodies only (`rest.rs:58`); NFR-022-002 also covers logs, spans, metrics and audit. Do not
   count it as log or span evidence.
2. `key_consistency` returns `Ok(vec![])` on any 42P01 or 42703: a renamed column makes every `assert_consistent` pass
   while checking nothing. Prefer an explicit `to_regclass('concurrency_holders') IS NULL` check.
3. `run_permit` (`let Ok(Some(state)) = row.try_get(...) else { return Ok(None) }`) swallows a decode error; follow
   `selected()` (`execution.rs:2514-2523`).
4. Each page computes both correlated subqueries for every live key of the namespace and sorts before `LIMIT`; bounded by
   the token bucket and the live-keys ceiling, not a real DoS; revisit with T022-105.

## Classes with no finding (checked)

- Authorization and admission: `OpsRead` checked in `admit` before the namespace is resolved (403 whether or not it
  exists), checked again in the service; `OpsRead` held by admin, developer, operator, viewer and by no machine role; group
  and cursor validation before admission, as `list_alerts`; `RunBody` built only in API handlers, `POST /runs` returns a
  record without `concurrency`, no webhook or worker protocol carries the field.
- Tenant boundary: every new statement binds tenant and namespace; the lateral is tied to `runs.tenant_id` and the run id;
  `RUN_SELECT`'s other users bind the tenant; a foreign namespace's timing is the same as every namespace route's.
- Secrets and personal data: no raw value reaches this layer; no new metric labels, audit records or tracing fields; the
  access log records `MatchedPath`, not the query; error messages are constants.
- Least privilege: `sagat_app` has SELECT on the four tables; the read is `REPEATABLE READ READ ONLY` with no row locks;
  the lateral uses the primary keys; no new SECURITY DEFINER function.
- Injection and parsing: production SQL is `concat!` of literals and the macro, every value bound; group checked by
  `ResolvedGroup::parse` and the CHECK; every cursor part checked; test-only `AssertSqlSafe` strings built from constants.
- Races: the new reads are snapshots that write nothing; the new suites use held locks and `pg_stat_activity`, not sleeps.
