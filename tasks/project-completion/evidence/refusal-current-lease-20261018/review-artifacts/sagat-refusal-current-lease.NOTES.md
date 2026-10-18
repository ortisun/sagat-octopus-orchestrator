# Current refusal lease draft notes

- Repository read baseline: `/Users/marcelo/Documents/Projects/sagat-octopus-project-completion` at `d96b943763c93eff68ac4fa145d4c45c43a7d327`.
- Base source SHA-256: `7017ac479064e0a94b4dd6af5af46f66be56267b51857c22a5fd39d1223c1dbe` for `tests/integration/workflow-primitives/refusal_secret_versions.rs`.
- Sealed patch SHA-256: `1e2f03b508dfabba49585b14a8c55af1c7ffc552bc4c65b7f36fd15c7d089406` for `/tmp/sagat-refusal-current-lease.patch`.
- Repository path changed by the patch: `tests/integration/workflow-primitives/refusal_secret_versions.rs` only.
- Temporary artifacts written: `/tmp/sagat-refusal-current-lease-draft/tests/integration/workflow-primitives/refusal_secret_versions.rs`, `/tmp/sagat-refusal-current-lease.patch`, and this notes file.

## Exact added ranges in the draft mirror

- Lease and audit witness records: lines 518-544.
- Lease, step, checkpoint, binding-byte, pool, membership and session readers: lines 546-654.
- Snapshot normalization oracles: lines 656-673.
- Independent testcase `a_scanned_output_refusal_rechecks_the_original_lease_after_path_drop`: lines 996-1480.

## Oracle coverage

The new case uses the real OpenBao 2.7.1 provider, one public managed secret, a public 17-member depth-one collection, sixteen genuine 250,000-byte completions and the original seventeenth public lease. It normalizes the public worker binding to both advertised activities before leases, observes zero initial revocations, and then narrows the binding to `line.reserve` while the exact runtime-role report PID is blocked on the full-only checkpoint restore query.

The witnesses check the complete pre-report execution snapshot, exact checkpoint rows, full scoped receipt key/address, exact original token hash/attempt/generation/status/holder/cancellation/output/report facts, database-clock lease freshness, one advertised session, binding rows and physical byte total, binding pools, Worker membership, and the complete managed secret catalog. After the public replacement they check nested 1/1 revocation, the exact `worker.bound` audit and trace/time bounds, the computed binding quota delta, removal of only the identified session, native expiry/update-time as the only target-step row delta, and unchanged catalog/membership/pools.

After releasing only the owned gate, the original report must return gRPC `Aborted` with `STALE_ATTEMPT`. The raw post-mutation snapshot and checkpoint rows must remain byte-equal, which excludes execution, refusal, receipt, audit, outbox, telemetry-head, payload, logical-output and reported-lease effects. A fresh store/cache journal verification uses only a fixed boolean assertion. The expired lease is not retried.

## Scope and execution

No production source hook or helper outside the assigned module was needed. The existing full snapshot, exact runtime PID/ungranted relation-lock observers, full scoped receipt construction, and `version_row` catalog alias were reusable as required.

No tests, builds, formatter, containers, database mutation, commits, or external writes were run. No runtime result is claimed. Repository changes visible in the worktree were pre-existing root-owned legacy fixture work and were not modified.
