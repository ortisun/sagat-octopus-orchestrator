# Correctness review of the build: feature 041, `e7b5ac9f..24787c78` (T041-33, T041-31, T041-3, T041-4)

Reviewer: `correctness-reviewer` (`.claude/agents/`), 2026-11-19, read-only, given the diff and the spec set, not the
author's conclusions. Answers: `resolution.md`.

**Summary.** No Blocker. Four Majors: one latent CPU denial of service in the domain, three in how the spike is
measured or read. Several test-honesty gaps.

## Major

- **M1. `canonical_fields` is cubic on colliding keys and nothing bounds the fields of a record**
  (`crates/domain/src/step_logs.rs`). 20,000 fields of one key fit in one record and cost ~1.3·10¹² comparisons; P5
  runs before T2, so any `worker:execute` token can send it. Test: 20,000 copies must finish within a timeout. Fix: a
  set of taken names and a next suffix per key; bound the fields considered.
- **M2. The report's reason for not raising O041-026 does not follow from its figures.** T041-31 asks whether more than
  half of the p99 excess is wait on the tenant row; the report compares the mean wait (~0.25 ms) with the p99 excess
  (17 ms at 1 × 16). The sampled wait (≥2.25 s at 1 × 16) could explain the whole tail if it sits in the slowest
  completions; the sampler also undercounts. Attribute waits per completion; until then O041-026 is undecided.
- **M3. The driver almost never lets a final batch be refused for a ticket.** The permits are released when `append`
  returns and the same task re-takes them with no `.await` in between, so "0.00% of finals" in every row, (e)
  included, is an artefact. Add a yield (or a simulated round trip) before admission and rerun (a) and (e).
- **M4. The fan-out "pass" was measured almost without load** (69 transitions/s, no commit waits): raise the load until
  waits appear, then record the result.

## Minor

- **m1.** The cut test never steps back (é is 2 bytes and 8,192 is even); prefix one ASCII byte.
- **m2.** The counting-rule property checks the plan against its own counters; count `keep` and compare.
- **m3.** Every pair runs off before on, then a', then chunk rows on a growing database; alternate or report the spread.
  The 8 × 128 p99 verdict is not established; the 1 × 16 failure holds whatever the order.
- **m4.** `cap.min(half_pool.max(cap))` is always `cap`; in the run that needed 17 tickets the cap stayed 17.
- **m5.** S leaves out lock-timeout transactions, biasing S p99 and the ticket count down.
- **m6.** Departures from T041-31: the transaction of (d) held 14.1 s, not a minute; "commit time of other writers"
  reported as the completion p99; T9 always writes `ceiling_reason = 'attempt_bytes'`; gaps not split around
  `ceiling_sequence`. State them.
- **m7.** The migration test inserts ceilings of value 1 only (not the 1 GiB that the value-range correction is about),
  and does not test `quota_limits_tenant_scoped` or the `step_log_bytes <> 0` guard.
- **m8.** Only the `list` path of D041-066 is tested; `view` has none.

## Notes

- **N1.** Once a drop count saturates at 2^53 the counting rule no longer holds; limit the stated rule to below it.
- **N2.** The `ceiling > stream.last_sequence` branch of the gap split and `met.is_none() || first_met.is_some()` are
  always one way; harmless.
- **N3.** `deleteRunLogs` declares no `400` or `422`.
- **N4.** `AppendStepLogs` falls into the discovery class until T041-7; negligible while it answers `UNIMPLEMENTED`.
- **N5.** The spike's quota-retry match on `constraint() == "tenant_log_quota_exceeded"` may not match the finalizer;
  cannot happen at 2^50.
- **N6.** The 1 × 16 rate mostly measures added latency (a closed loop); a cheaper ingest transaction alone will not
  reach 90%. Put O041-036 in those terms.

## Checked and found sound

`validate_batch` against P4 and contracts.md §1; `plan_batch` against data-model.md §8 T5 and T6 (duplicates, the high
mark never lowering `last_sequence`, the attempt allowance in sequence order, one marker on the first attempt ceiling,
text plus 128 bytes a row with the marker for the tenant, an empty batch charged nothing, `FINAL_ONLY` sticky);
`canonical_double` against ECMAScript `Number::toString`; JSON escaping, key order and the 32-key / 2,048-byte bound;
the four quota resources against the database CHECKs, `quota_limits_tenant_scoped` without `tenant_step_log_bytes`,
`NOT VALID` then validated, the down guard, the oracle in three files; usage, permissions and audit; the proto against
contracts.md §1, `skip_debug` and canary tests in three crates, `UNIMPLEMENTED` in seven implementations; the four
REST operations `x-planned`; no task box ticked, T041-31's evidence honest.
