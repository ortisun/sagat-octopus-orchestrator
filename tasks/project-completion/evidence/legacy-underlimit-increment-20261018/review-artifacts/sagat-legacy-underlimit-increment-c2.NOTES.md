# Q10c under-limit increment C2 TMP delta

## Delta identity

- C2 is an exact two-file delta against the sealed, unformatted C1 mirror at `/tmp/sagat-legacy-underlimit-increment-draft/`.
- C1 patch remains unchanged at SHA-256 `7b64876c6fb61658278c23de6a65ecb6a34ad27fe206b9efb9f3838545c52d30`.
- C2 patch: `/tmp/sagat-legacy-underlimit-increment-c2.patch`.
- C2 patch SHA-256: `483aea4768727e91c25711be95895d260c9c5f7fb62857e6df41ff3d0f5d9d70`.
- C2 patch size: 8,697 bytes, 152 lines.
- Frozen repository source pin remains `5ad34370e9bec4ebfe5aab6e8b802f6a670c73f8`; both owned paths remained byte-identical to that source in the shared worktree during drafting.

| Owned C2 source | C1 SHA-256 | C2 SHA-256 | C2 lines | Delta + / - |
| --- | --- | --- | ---: | ---: |
| `tests/integration/workflow-primitives/legacy_collection_inflight_report_v1.rs` | `38b7cfb4e7473ac752e304ccb3c598b0be0bd610926365fa7b835768d38e99d7` | `7d475655465e16f69515673de205ed8559e5bda555676f285b51a637e625740a` | 3905 | 81 / 5 |
| `tests/fixtures/legacy-collection-inflight-report-v1/README.md` | `0b215ff8417344f1c53e878d194523b300fd0699e7dd8fd4edab9dba514fac6c` | `6fca15fa413a762bc5b8d04ce4899326c6f5a8543417c714afb1ab91202b0d31` | 111 | 6 / 5 |

## Corrected review units

1. **Terminal retained references.** A dedicated fingerprint records every nonnull run, step, collection scope, member, and checkpoint output digest with its durable identity. It includes every step node, so a nonnull `reserve~c` compensation output cannot be filtered away. A newly created compensation row with a null output no longer changes this fingerprint. The value is captured before the report and compared immediately after terminal compensation. The original broad `output_reference_fingerprint` and all original three-case assertions remain unchanged.
2. **Real compensation output.** The under-limit terminal branch queries the exact `c10/0/reserve~c` row and requires `output_digest IS NULL`. Existing row existence, status, event count, saga outcome and target identity assertions remain in place.
3. **Complete continuation footprint.** Every call to `under_limit_execution_footprint` now includes the complete `telemetry_pending` row, all complete `scope_sagas` rows, and all complete `scope_saga_entries` rows for the exact tenant/run. `to_jsonb` retains every field, including telemetry heads, saga timestamps and sequences. Aggregates use the actual remaining primary-key order and preserve SQL null when there are no rows. The definitions were read from pinned migrations `20261015000100_telemetry_projection.up.sql` and `20261112000000_scope_compensation.up.sql`; the complete-row snapshot form was cross-checked against the pinned `byte_ceilings.rs` transaction snapshot.
4. **README truthfulness.** The report accepts the original live lease and clears it through recorded success. The later drain is described as preserving that success and its original compensation target.

## Constraints and root review

- Only the consumer and fixture README are present in `/tmp/sagat-legacy-underlimit-increment-c2-draft/`.
- No repository, production, schema, migration, grant, flag, registration, support, coordinator, lockfile, spec, task or evidence path was changed.
- No formatter, compiler, shell parser, test, SQL, container, architecture gate, commit or subagent was run.
- Root should apply C1 and then C2, format the combined tree, inspect the formatter diff, and obtain the requested fresh bounded C2 review before executing gates.
- This C2 delta changes test oracles and documentation only; it adds no acceptance claim beyond C1.
