# Architecture review of design 042 (first draft)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: 540154c80cb36cc585d18c5952f7c0abce77d430
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output was relayed by the coordinator in condensed form, grouped by subject across the three reviewers; this record lists the subjects attributed to this reviewer, in the relay's words, with the answer of the second draft. The relay itemises 11 of the 13 Blocker and Major findings and 2 of the 8 Nits; the rest were not itemised and are covered only where they coincide with an item below. Counts are the reviewer's own.

## Blocker

1. The 20,000-evaluation ceiling bounds neither memory, bytes nor time; `environment` re-parses every step output per evaluation; `map` over a large output builds GiBs; `take`/`window`/`skip` evaluate nothing; the inline input count is unchecked.
   *Answer:* one port call that builds the environment once, stage by stage in a blocking task with a 1 s deadline; running ceilings per stage (input and stage elements 1,000, element 256 KiB, stage 4 MiB) failing at the first crossing with one code each; the 20,000 number withdrawn (FR-042-006, FR-042-007, section 3, ADR-0087 decisions 2-3, D042-017 to D042-019, T042-7, T042-8, T042-9).
2. Blob sources: `materialize` is bounded by the node's `maxItems` and the entry and restore checks require the manifest to equal the stored items, which a chain changes.
   *Answer:* a chained manifest under `$collectionChainSource` with source and output count and digest and the chain digest, read by both checks; `materialize` bounded by the chain input ceiling; output ceilings checked before the store; old readers return `None` and fail closed (FR-042-010, section 3, ADR-0087 decision 6, D042-020, T042-45, T042-46).
3. The capability gate in `convert::*` would also run in `parse_stored`, so switching it off would make stored definitions unreadable.
   *Answer:* the switch moves to `DefinitionService` at publication beside `lake.enabled`; the codec always decodes; a `parse_stored` test with the switch off (FR-042-003, section 1, ADR-0087 decision 8, D042-021, T042-47).

## Major

4. `onError` is not taken by a collection that fails at entry (`stop_with_code_routing(..., None)`).
   *Answer:* the run fails with the typed code, as for a failing `items`; routing to `onError` is gap G042-008; CUJ 1 uses `has()` (FR-042-008, ADR-0087 decision 4, D042-022).
5. The "one transaction" claim is false: `items` is evaluated after `drop(tx)`; entry reruns on `SourceUnavailable`, a fence or revision conflict and the quota `Waiting` path.
   *Answer:* the output is recorded once by the entry transaction and evaluations before a failed commit are discarded; S1 changes the scheduler's collection entry and adds no loop; the quota re-entry is backed off and counted, and T042-9 measures the worst case and `scheduler_tick_seconds` (FR-042-009, section 3, ADR-0087 decision 5, D042-023, G042-012, T042-52).
6. Stage variables `item`/`index` would shadow the iteration's under nesting.
   *Answer:* `element` and `position`; `item`, `index`, `iteration`, `carry`, `bodyResult` refused in a stage (FR-042-005, D042-026, T042-6).
7. SDK shape: `forEach`/`map` return the workflow builder, `map` is already the alias, and Rust's `CollectionOptions` is a public struct without `#[non_exhaustive]`.
   *Answer:* operators in each SDK's collection options or an additive overload, built from an `ops` factory; Rust gets new methods and leaves the struct unchanged (FR-042-028, section 3 table, D042-027, T042-10).
8. Telemetry and closed sets: no S1 counter, new codes absent from `WorkflowPrimitiveRejectionCode::ALL`, the AsyncAPI and `docs/sdk-compatibility.md`; no leak test.
   *Answer:* `operator_elements_total{surface="collection"}` in S1, one new code joined to those sets, and a no-value-leak test (FR-042-025, T042-49).
9. ADR and approval wording disagree with the owner's acceptance.
   *Answer:* both ADRs say accepted (OD-2026-11-13-17) with a Revision section; the design, section 11, the roadmap row and D042-016 agree. T042-4 is not ticked: rule R9 of the checker refuses a tick in a doc that is not Approved; a dated note says it is done.
10. (M9) A window whose run 022 buffers: the design says `STARTED`, 022's D022-018 says "stays sealed".
    *Answer:* `STARTED`, because the run exists; 022's text to be aligned (FR-042-019, section 8 rule 2, D042-036, G042-011, T042-3).
11. (M10) The apply path inserts the delivery row and then writes the outcome on it.
    *Answer:* the delivery row is inserted once, last, with its final outcome, and never updated (FR-042-014, section 5, D042-035, T042-15).

## Nit

12. `distinct` by digest of serialised bytes disagrees with `==`.
    *Answer:* a canonical form (FR-042-011, D042-025, property test in T042-7).
13. Parameter ranges for `skip`, `take` and `window`, and `window {count: 0}`.
    *Answer:* ranges stated and `window {count: 0}` refused at publication (FR-042-004, T042-5).

Counts: Blocker 3, Major 10, Nit 8
