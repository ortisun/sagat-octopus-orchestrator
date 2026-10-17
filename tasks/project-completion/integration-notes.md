# Integration checkpoint — 2026-10-17

This is an implementation checkpoint, not a completed release gate.

## Reconciled boundaries

- Preserve all 100 migration files from the combined 014 baseline byte for byte.
- Allocate 017 migrations after that history (20261122–20261128); preserve its standalone branch.
- Preserve scoped protobuf field numbers; reserve managed capabilities for minor1.6 independently of scoped minor1.5. Default adapters keep ordinary workers at minor1.4 until scoped execution is enabled.
- Bind managed credential pins and all managed step/journal selections to root identities; reject unproven managed/structured or continuation composition at publication and typed construction.
- Keep provider reads outside transactions; repeat current role, lease, publisher ACL, binding and pool checks before disclosure and commit its audit first. Use binding → session (managed) → catalog → membership ordering, retain final grant locks through commit.
- Bind optional worker pools into durable OpenBao intents and apply the restrictions with activation. Omitted pools preserve existing policy; changed replay metadata conflicts.

## Executed checks

- `cargo test -p sagat-application --lib worker::tests:: --locked`:8passed,0failed; includes ordinary/managed protocol feature-gate matrix.
- `cargo test -p sagat-infra-ir --test database --locked`:8passed,0failed; includes publication composition boundary.
- `cargo check --workspace --all-targets --locked`: passed after integration fixes (dev profile,1m06s including shared-cache contention). This compiles test targets but does not execute them.
- `python3 scripts/check-architecture.py`: passed,24workspace crates, no dependency cycles.
- Canonical `worker.proto` compiled with `protoc` into a descriptor.
- Automated byte comparison against HEAD: all100preexisting migration files unchanged.
- Storage classification metadata:387entries,0duplicate(table,column)keys.
- Python and Go protobuf code regenerated; Studio OpenAPI types regenerated from the combined017/014contract. References and future016types still need integrated regeneration.

## Pending verification

Workspace all-target compilation exposed cross-feature interface mismatches; the final check passed after correcting them. Earlier failed attempts are not passing test evidence. The database composition RED fixture exposed the existing body-kind schema refusal; its assertion was corrected to accept the documented schema refusal and separately demand the mixed-root publication gate. Real PostgreSQL/OpenBao pool/authority races, scoped regressions, all-five-SDK runtime conformance and the final full gates remain pending.

Independent Astra xhigh static review found and drove fixes for pool bypass, stale ordinary worker membership/ACL authority, same-named scoped lease deadlines, binding/session deadlock order and protocol feature-gate negotiation. It ran no executable tests. A subsequent final review remains required after integration.

## Classification integration checkpoint

- New normal and private ad hoc versions pin interpreter rules 1; legacy migration defaults to 0.
- `integration_payload_classification` ran 8 PostgreSQL/HTTP tests successfully on the combined tree
  (`/tmp/sagat-completion-grants-http-green.log`): immutable pinning, byte preservation across the
  exact up/down DDL, grant rules/tenant/cascade, stale actor refusal, audit rollback, a concurrent
  256-grant ceiling, revocation blocked until an effective grant read transaction ends,
  repeated DELETE returning 404, no-store metadata, and oversized request refusal.
- This is focused integration evidence. It does not establish old-binary compatibility, full
  classified disclosure behavior or the final release gates. Classifier review findings remain
  under correction before value disclosure is mounted.

### Publication and SDK checkpoints

The production IR codec now runs the closed classification findings before the general workflow
schema, and validation includes its bounded report. Two codec tests first failed (SCHEMA instead
of CLASSIFICATION_NOT_APPLICABLE; a lowered declaration accepted) and passed after wiring
(`/tmp/sagat-completion-publication-{red,green}.log`). Full REST report validation remains pending.
Named-body report rows carry an optional `bodyName` to disambiguate repeated node identifiers.

SDK verification at `a74ec176` used the existing pinned runners and found no SDK defect:
TypeScript 130 tests, Python workflow 14, .NET builders 80, Java builder 24, and Go workflow
package passed. All five classification outputs matched the golden: 1,474 canonical carrier
bytes, SHA-256 `0700a0eb7dfff6baed1a3173e14f77e2a178a33db30d7b9cb4222dfd42dcb046`.
This focused proof does not replace the final complete protocol/runtime conformance matrix.

### Managed secret capability checkpoint

`integration_database_connections` passed all 3 top-level tests on the combined publication tree
(`/tmp/sagat-completion-database-integration-4.log`, 65.03 s), including its aggregated real
OpenBao lifecycle, database executor, pool revocation/provider-I/O races, output and blob cases.
Worker registration now requires persisted current membership with WorkerExecute; test worlds
persist their default worker identities. The aggregate test allocates an explicit 16 MiB test
thread stack and acquires a fresh lease immediately before the final revocation race. This
removes the earlier default-harness stack overflow and an expired-lease barrier wait; production
runtime stack/lease settings are unchanged. Final combined workspace gates remain pending.

### Studio precision checkpoint

The disclosure view now requests safe numeric parsing. A textual response containing
`9007199254740993` reproduced the rounded display in a failing component test; the corrected view
refuses numbers the browser cannot preserve and points the user to exact API/CLI representations.
Seven disclosure component tests, Studio typecheck and focused ESLint passed
(`/tmp/sagat-completion-ui-precision-{red,green,typecheck,lint}.log`). The server keeps its exact
numeric encoder. Final classifier review still has pending numeric-applicator and hidden-child
budget regressions; the production disclosure service remains unwired until these are corrected.

### Numeric validation and hidden-child budget checkpoint

The second security review closed indirect integer restrictions and hidden-child projection
budgets in `e5145923` (18 focused tests / 32 schema package tests). Two further exact repros
were added: `not`/`if` with `uniqueItems`, and an integer `multipleOf` divisor above the
dependency's exact compilation range. Both reproduced disclosure of mathematically invalid
values before the guards. All 20 focused classifier tests then passed
(`/tmp/sagat-completion-{unique-items,multiple-of}-{red,green}.log`). Numeric comparisons fail
closed when exact validation cannot be established; large integer values remain supported with
proven exact small divisors. This conservative boundary and the final service composition still
require the final integrated security/contract gates.

### Disclosure composition and deferred audit failures

The run disclosure service and US08 event read service are now mounted in both production and
real-API test factories. Root's combined PostgreSQL run passed 18 of 19 cases; the remaining result
case exposed a logical-output ledger defect: an `end` output created in the same transaction had
no preexisting projection row. A narrow fix and exact two-reference/replayed-report assertion are
under verification; this checkpoint does not claim the result case passed.

Two real PostgreSQL fault tests use `DEFERRABLE INITIALLY DEFERRED` audit triggers. The denial
commit first returned an internal storage error instead of the required typed refusal. Both run
and event denial paths now treat the entire denial audit transaction as best effort, including
commit/rollback failures, while a failed mandatory success commit still returns no values. Both
fault tests passed (`/tmp/sagat-completion-denial-commit-green.log`, 1.99 s). The initial success
control's expected error class was corrected from unavailable to the adapter's actual internal
classification; its refusal of values was already closed.

The actual authoring REST contract target passed 13/13
(`/tmp/sagat-completion-authoring-contract-green.log`). Its unrelated-operation digest baseline
changed only for approved 014 `getChain` pagination guidance (112f1e1d); the operation remains in
the checksum. API request unit tests passed 4/4 after endpoint-only unknown-field handling. Actual
REST audience and request boundary gates are still being finalized by the owning test stream.

### Authoring review corrections

Independent static review found reference-reachable annotations beneath unsupported applicators,
and two diff fallback errors involving explicit ancestors and secret inheritance. Commit
`b1d96a4b` reproduced the publication and diff regressions RED, then passed publication classification
14/14, semantic classification diff 14/14, publication 3/3 and carrier compilation 2/2 on its isolated
warmed target. Focused adapter Clippy passed. These fixes are merged; complete integrated gates remain.

### Python SDK integration gates

The combined Python SDK passed 261 tests in 22.23 s
(`/tmp/sagat-completion-python-tests.log`). Mypy passed all 38 checked files and Ruff passed after
renaming the chain page local variable (which had reused the run-page inferred type), sorting the
public export list, and applying the pinned formatter. This is SDK-local proof, not the all-five
runtime server conformance gate.

### Combined HTTP, result and browser checkpoint

At `a08bb21b` plus the narrow logical-output ledger fix, the combined target command passed
15 contract, 2 fault and 21 real PostgreSQL/HTTP integration tests
(`/tmp/sagat-completion-classification-combined-gates-3.log`). This includes the complete 96-cell
REST audience matrix, fixed selector/reveal/unknown/malformed errors without reflection, typed
1KiB refusal, private/no-store success and error responses, opaque tenant resolution with no foreign
debit, and exact namespace admission. The result regression now passes and confirms two logical
output references despite shared content, with no extra byte charge on an idempotent report replay.

The actual Chromium disclosure journey passed 1/1 (`/tmp/sagat-completion-disclosure-browser.log`):
viewer, developer, operator and revealer projections, forbidden canary absence in wire and DOM,
keyboard confirmation, PII absent from browser stores and cleared on close. Browser time 2.5 s;
real API/PG harness 7.66 s. The focused command used `SAGAT_STUDIO_NO_SANDBOX=1`, so the unrelated
script sandbox journey was explicitly excluded; this is not the full Studio gate or human visual
acceptance. The production Studio build passed 16.60 s before that run.

The cache benchmark report distinguishes D281's 16 MiB serialized schema weight from measured
compiled retention (~414.24MiB requested live heap, not RSS). A reviewed concurrent-miss accounting
fix and bounded compile admission are in progress. No actual 16 MiB heap guarantee is inferred.
