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
