# ADR-0087: Collection operators are applied once at scope entry and the snapshot is the record

**Status:** Accepted (2026-11-13, OD-2026-11-13-17: the owner's "Aceito tudo"); revised the same day after review, before any code (see Revision)
**Decided by:** owner, on the agent's proposal (OD-2026-11-13-17, accepting the text of commit df83eed9); the revision of 2026-11-13 is the agent's, answering the architecture, security and correctness reviews, and is put to the owner with the design
**Refines / supersedes:** refines ADR-0032 (additive IR v1, one capability at a time, the frozen expression limits) and ADR-0030 (bounded expressions, scoped instances); builds on ADR-0003. Supersedes none.
**Enforced by:** proposed checks of design 042, slices S0 and S1: the IR contract fixtures (T042-6), the `parse_stored` test with the switch off (T042-47), the property test of the chain (T042-7), one test per ceiling (T042-8), the blob tests with `verify_run` (T042-45), the race and panicking-evaluator tests (T042-48), the no-value-leak test (T042-49), the old-reader test (T042-50), the unchanged-entry test (T042-51), and the six-language scenarios `collection-operators`, `collection-operators-runtime` and `collection-operators-limits` (T042-11). None exists yet; until they do, this rule is review only.

## Context

A `forEach` of feature 014 evaluates `items` once and snapshots the membership (`iteration_scopes`,
`collection_scope_items`). The collection entry of the scheduler (`crates/scheduler/src/passes.rs`, from about line
1094) reads the run data under the fence, drops that transaction, evaluates `items`, and calls `enter_collection`, which
opens its own transaction and checks the fence. An inline list is stored as one snapshot payload of at most 256 KiB; a
blob source is materialised bounded by the node's `maxItems` and stored as a `SourceManifest` (blob, count, items
digest) that `enter_collection` and the restore check against the stored items. A failing `items` stops the run with a
typed code; the collection's `onError` is not taken at entry.

There is no way to drop, project, flatten, deduplicate or group items in the definition. The owner asked for
reactive-stream operators and decided that release 0.3.0 carries them on collections (design 042, O042-001, O042-002).

The forces:

- **Replay never re-decides.** Membership is read from the snapshot; whatever shapes the list must be what the snapshot
  records.
- **No code in the engine.** Principle V and AGENTS rule 7 leave only the bounded expression language: deterministic, no
  clock, no I/O, no object literal, limits frozen by ADR-0032.
- **The present evaluator parses per call.** `JsonEvaluator`'s `environment` (`crates/infra-ir/src/evaluator.rs`, lines
  33-49) parses the run input, the step outputs and the state for every expression evaluated. Per element, that is the
  run's data parsed a thousand times.
- **A count of evaluations bounds neither bytes nor memory.** `map {select: "steps.big.output"}` over many elements
  builds a list of the step output repeated; `flatMap` grows a list; `take`, `skip` and `window` evaluate nothing.
- **The codec runs on read.** `parse_stored` reaches the same `convert::*` functions as `parse`.
- **The schema is closed.** A reader that does not know a field refuses the document.

## Decision

1. A `forEach` MAY declare `operators`, one to eight stages from the closed set `filter`, `map`, `flatMap`, `skip`
   (0 to 1,000), `take` (0 to 1,000), `distinct`, `window {count}` (1 to 1,000), in any order. Every expression MUST be
   of the existing language, validated at publication against the existing limits; the language MUST NOT be extended.
   Stage expressions see `element`, `position` and the variables `items` sees; a stage that names `item`, `index`,
   `iteration`, `carry` or `bodyResult` MUST be refused at publication.
2. The chain MUST be evaluated by one evaluator port call that builds the run's environment once and applies every
   stage, stage by stage, off the async runtime in a blocking task with a cooperative 1-second deadline checked between
   elements. An error MUST name the first failing stage, then the first failing position, with the operator, and never a
   value.
3. Ceilings MUST be checked while each stage's output is produced and fail at the first crossing with
   `COLLECTION_OPERATOR_BUDGET`: the chain's input at most 1,000 elements (the 0.3.0 published collection ceiling,
   OD-2026-11-13-1; a blob source is materialised with that bound), each stage's output at most 1,000 elements, one
   element at most 256 KiB, one stage's output at most 4 MiB, and the deadline. An expression error, a `filter` value
   that is not a boolean or a `flatMap` value that is not a list MUST fail with `EXPRESSION_ERROR`. After the chain, the
   node's `maxItems` (`ITERATION_LIMIT`) and the inline snapshot ceiling (`RUN_DATA_LIMIT`) apply as today. Every
   ceiling MUST be checked before `enter_collection`, so the store never refuses a chained entry with an untyped error.
4. A chain that fails at entry MUST fail the run through the path a failing `items` takes. The collection's `onError` is
   not taken; routing entry failures to it is a follow-up for all collections.
5. The output MUST be recorded once, by the transaction that enters the scope. An evaluation whose entry does not
   commit is discarded; the next entry evaluates again over the same immutable data and gets the same output. A
   storage-quota wait is re-entered only after a back-off. Once the entry has committed, nothing MUST evaluate the chain
   again: not a replay, a retry, a restart, a reconciliation or a read. No table and no journal event type is added.
6. An inline source's output MUST be stored as today's snapshot array. A blob source's output MUST be stored per item
   with a chained manifest (`$collectionChainSource`: blob, source count and digest, chain digest, output count and
   digest), and `enter_collection` and the restore MUST check the stored items against the output count and digest.
7. `distinct` MUST compare a canonical form: integral doubles within plus or minus 2^53 as integers, `-0` as `0`, object
   keys sorted by code point, independent of `serde_json` features.
8. The field is additive in IR v1. The codec MUST always decode it; the deployment switch `collection-operators` MUST be
   checked by the definition service at publication, as `lake.enabled` is, and MUST NOT make a stored definition
   unreadable when off. An operator outside the set, or one only a trigger has, MUST be refused at publication.
9. Publishing a chained `forEach` requires every replica of the fleet to run a binary that knows the field; an older
   binary refuses such a definition and such a blob scope and MUST NOT misread them.
10. The six SDKs MUST emit the same IR for the same chain without breaking a present caller and pass the same scenarios.

## Considered options

1. **Apply once by one port call under running ceilings, record in the snapshot** — no new state, determinism for the
   reason collections already have it, and every resource bounded where it is spent.
2. **A count of evaluations as the only ceiling** (the accepted first text) — bounds neither bytes nor memory.
3. **The capability check in the codec** (the accepted first text) — switching it off would make stored definitions
   unreadable, since `parse_stored` uses the same functions.
4. **A journal event per stage** — records intermediate lists nobody needs.
5. **Engine-run virtual activities** — a task, a lease and an attempt per stage for a pure function of one input.
6. **A sandboxed function per stage** — a worker round trip and nondeterminism to police.
7. **Lazy stages fused across the chain** — `take` would limit a `flatMap`, at the cost of a harder error and ceiling
   rule.

## Consequences

- Good: no migration, no new pass or loop; an author removes a preparatory activity; every failure is typed.
- Bad: `map` cannot build an object; a chain cannot see the body's results; the chain input is 1,000 elements where
  `maxItems` validation accepts 10,000, so a `filter` cannot reduce a larger blob; `take` does not rescue an over-ceiling
  `flatMap`; the wall-time ceiling depends on the machine (a backstop, counted apart); an entry failure does not reach
  `onError`.
- Compatibility: definitions without the field are unchanged and cost the same. A binary that predates the field
  refuses a stored definition that has it, so the whole fleet runs 0.3.0 before publication, as the upgrade procedure
  already requires.
- Rollback: switch publication off; stored definitions stay readable and existing scopes run from their snapshots.
- Nested collections (design 039) inherit the rule: one application per scope; `item` and `index` stay free for the
  enclosing iteration.

## Revision (2026-11-13, after review, before any code)

The owner confirmed this revision on 2026-11-13 (OD-2026-11-13-19: "Confirmar a revisão").

The owner accepted the text of commit df83eed9. The architecture, security and correctness reviews of commit 540154c8
found that it left memory, bytes and time unbounded, broke the blob-source checks, put the switch where it would make
stored definitions unreadable, promised `onError` that the scheduler does not take at entry, and described the entry as
one transaction. Decisions 2, 3, 4, 5, 6, 8 and 9 were rewritten and decision 7 added; the 20,000-evaluation ceiling
and the code `COLLECTION_OPERATOR_FAILED` were withdrawn. The chosen option is unchanged: apply once, the snapshot is the
record. The reports are in `docs/designs/042-stream-operators.reviews/2026-11-13/`.

## Evidence

None yet. Design: `docs/designs/042-stream-operators.md` (FR-042-004 to FR-042-012, SC-042-003 to SC-042-005, Technical
Plan section 3); registers: `specs/042-stream-operators/research.md` (D042-010, D042-016 to D042-028, G042-001,
G042-008 to G042-010, G042-012 to G042-014).
