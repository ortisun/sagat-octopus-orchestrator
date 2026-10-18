# Q10c: immutable-old ordinary report before current drain

Bounded fixture over production base `6c44a0e49670cf19660792cd1518d241e6c99b3b`; root executes the real archived producer and current consumer in separate processes, preserving original worker authority. No deployment is inferred.

## Attempts retained

- `preflight`: strict integration Clippy failed on one unused import and one constant assertion; no behavior test was run by that command. Both lint-only issues were removed; the actual SQL effective-total assertion remains.
- `executed`: strict Clippy passed. The coordinator's immutable-old owner producer passed, but its current consumer failed before migration/report on a fixture expectation that `TaskLease.tenant_alias` equals the receipt alias. Both old/current service implementations deliberately leave that field empty because tenant authority is implicit in authenticated identity. This is a test-contract error. Native inputs before/after: 1124 each.
- `wire-contract-correction`: root changes only that assertion to require empty, preserving the receipt alias and actual authenticated original tenant/session/lease checks. The immutable-old owner producer passed and the current consumer passed all report/refusal/replay/drain checks, then failed on another fixture expectation: Run status was COMPENSATING after the real compensation task was offered, rather than the asserted CANCELLING. No engine change. Native inputs before/after: 1124 each.

The `review-artifacts` directory retains exact C1, C2, final C3 patches, plan, draft notes, hashes and reconciliation. Private handoff receipts/session/lease values are not copied into evidence. Each attempt retains its own source hashes.

## Scope

Ten genuine closed old aggregates produce L=2,100,000 and V=2,100,020: E=4,200,020 exceeds 4 MiB. The still-leased original eleventh member reports ordinary known success through the current public gRPC transport before any current scheduler tick. Intended assertions cover no retained refused output, exact receipt and replay, durable discovery, real compensation identity/key, closed-source preservation and fresh native journal reconstruction.

This is not Applied reconciliation, drain-first order, a post-ACK third-process restart or unknown-COMMIT recovery. It does not model an external provider business effect or prove its undo. The full Q10c row, all eight canonical014 tasks and production nesting activation remain open.

## Final acceptance

`compensation-contract-correction` passes the complete coordinator in 155.46 s. All four phase logs were independently inspected: immutable-old owner producer 1/1 (12.19 s), current owner consumer 1/1 (1.44 s), immutable-old runtime producer 1/1 (8.35 s), current runtime consumer 1/1 (1.09 s). Actual API roles are postgres/postgres and sagat_app/sagat_app respectively. Producer exits are reaped before current consumer launch, and both binaries are built before lease creation.

Final source asserts exact COMPENSATING Run and saga phase before real compensation completion and COMPENSATED only after it. The original scope, target identity/key, reported lease, exact receipt hash/scope/resource/command, immutable closed source witnesses, no refused output and stable replay all pass. Fresh store/cache verifies the journal; this is not a third OS process after ACK. Owned DBs and private phase receipts were cleaned by the coordinator.

Strict integration Clippy passes (39.12 s), cargo fmt check passes (6.16 s), architecture passes (0.61 s). All 1124 native input hashes remain identical before/after final gates. Archived producer rustfmt check and coordinator bash syntax check also passed. Preserved raw patch/harness evidence is not represented as source whitespace-clean. The current source five-file diff is separately checked before commit.
