# Research: stream operators (design 042)

Registers of the design `docs/designs/042-stream-operators.md`. Append-only: numbers are never reused or renumbered; a
gap is closed by evidence, not by deletion. Measurements are added as dated sections.

## Decisions

| Id | Decision | Date |
|---|---|---|
| D042-001 | Operators are declared data; every predicate, key and projection is an expression of the existing bounded language, evaluated by the engine. No user code. | 2026-11-13 |
| D042-002 | One vocabulary on two surfaces (trigger, `forEach`), behind two capabilities. Proposed; depends on O042-001. | 2026-11-13 |
| D042-003 | Chain state is PostgreSQL rows per chain and key; tables and types say "operator chain", because "Streams" is feature 026's word (ADR-0068). | 2026-11-13 |
| D042-004 | Time is the database clock; an element's time is `received_at`. Proposed; depends on O042-004. | 2026-11-13 |
| D042-005 | The order of a key's elements is the order of application under the key lock, recorded as a position. | 2026-11-13 |
| D042-006 | A window is sealed by a fenced pass and is not sealed while a never-attempted pending event received at or before its deadline could still match. | 2026-11-13 |
| D042-007 | A sealed window starts one run under a key derived from its recorded identity; a chain without a window keeps `start_key`. | 2026-11-13 |
| D042-008 | Trigger chains have a canonical stage order; collection chains may be in any order. | 2026-11-13 |
| D042-009 | No operator paces a run start; that is feature 022's. | 2026-11-13 |
| D042-010 | A collection chain is applied once at scope entry and the snapshot is the record. | 2026-11-13 |
| D042-011 | The chain is stored on the trigger row, so that an old dispatcher over-delivers rather than loses. | 2026-11-13 |
| D042-012 | The owner answered O042-001 to O042-009 with "Aceitar todas" (OD-2026-11-13-15): every recommendation is the decision. D042-002 and D042-004 are therefore decided, not proposed. | 2026-11-13 |
| D042-013 | Release 0.3.0 carries S0 (collection half) and S1 only; the trigger fields enter the IR schema with S2, in a later release. | 2026-11-13 |
| D042-014 | A sealed window's start is a run creator through feature 022's one admission port; a run that 022 buffers makes the window `STARTED`; a full buffer leaves it `SEALED`; chain rows are locked before the semaphore and rate rows (ADR-0088, decision 4). | 2026-11-13 |
| D042-015 | The proposed ADRs are ADR-0086 (operator chains on triggers) and ADR-0087 (collection operators); 0084 and 0085 belong to design 041, 0088 to feature 022. | 2026-11-13 |
| D042-016 | The owner accepted ADR-0086 and ADR-0087 ("Aceito tudo", OD-2026-11-13-17). Both were revised after the reviews of the same day, before any code; each ADR says what changed. | 2026-11-13 |
| D042-017 | The collection chain is evaluated by one evaluator port call that builds the run's environment once, stage by stage, in a blocking task with a 1-second cooperative deadline. | 2026-11-13 |
| D042-018 | The collection chain is bounded by running ceilings checked while each stage is produced: input and stage elements at most 1,000, one element at most 256 KiB, a stage at most 4 MiB, wall time 1 s; all `COLLECTION_OPERATOR_BUDGET`. Stage errors are `EXPRESSION_ERROR`. The first draft's 20,000-evaluation ceiling and its `COLLECTION_OPERATOR_FAILED` code are withdrawn (never put to the owner). | 2026-11-13 |
| D042-019 | With operators present the chain's input is bounded by 1,000 elements, the ceiling published for collections in 0.3.0 (OD-2026-11-13-1); a blob source is materialised with that bound. | 2026-11-13 |
| D042-020 | A blob source with operators stores a chained manifest under `$collectionChainSource` (blob, source count and digest, chain digest, output count and digest); entry and restore check the output. An inline source keeps today's snapshot array. | 2026-11-13 |
| D042-021 | The `collection-operators` and `trigger-operators` switches are checked by `DefinitionService` at publication, as `lake.enabled` is; the codec always decodes the fields. | 2026-11-13 |
| D042-022 | A collection chain that fails at entry fails the run through the path of a failing `items`; `onError` is not taken (G042-008). | 2026-11-13 |
| D042-023 | The chain's output is recorded once by the entry transaction; evaluations whose entry does not commit are discarded; a storage-quota wait is re-entered after a per-replica back-off and counted. | 2026-11-13 |
| D042-024 | Errors name the first failing stage, then the first failing position, with the operator; never a value. | 2026-11-13 |
| D042-025 | `distinct` compares a canonical form: integral doubles within plus or minus 2^53 as integers, `-0` as `0`, keys sorted by code point, independent of `serde_json` features. | 2026-11-13 |
| D042-026 | Stage variables are `element` and `position`; `item`, `index`, `iteration`, `carry`, `bodyResult` are refused in a stage. The body's `index` is the position in the output. | 2026-11-13 |
| D042-027 | SDKs carry the chain in the collection options (TypeScript, C#), a variadic (Go), a keyword argument (Python), overloads (Java) or new methods (Rust), built from an operator factory; no present call breaks. | 2026-11-13 |
| D042-028 | Release 0.3.0: publishing a chained `forEach` requires the whole fleet on 0.3.0, as the upgrade procedure already requires; an old reader refuses, never misreads. | 2026-11-13 |
| D042-029 | Trigger chains hold element values in a deletable, classified, quota-counted column of their own table, deleted when the window's run starts or the window is discarded; never in `payloads`, which is append-only. | 2026-11-13 |
| D042-030 | Trigger keys and `distinct` values are stored as HMAC-SHA-256 under a per-tenant key that stays in the database; the API names keys and windows by random surrogate ids. Refines O042-008; put to the owner with T042-44. | 2026-11-13 |
| D042-031 | A new key beyond `maxKeys` (and a mark beyond `maxMarks`) ends that event's delivery to that trigger at once, `FAILED`/`KEY_LIMIT`, audited once per chain and minute with a count, retryable by an operator; marks do not hold a key slot. Refines O042-009; put to the owner with T042-44. | 2026-11-13 |
| D042-032 | Window flush and retry need `run:start` and `run:operate`, discard needs `run:operate`; the caller is recorded as the starter of the run. | 2026-11-13 |
| D042-033 | A window is sealed no earlier than its deadline plus `admissionGraceMs` (at least the admission statement timeout), the backlog rule is scoped by tenant and namespace and counts only events with `attempts = 0`, and holds at most `maxHoldMs`. | 2026-11-13 |
| D042-034 | A key may have several open windows; an element joins the one covering its `received_at`; a deadline that has elapsed is never extended. `emit` order is `(received_at, source, event_id)`. | 2026-11-13 |
| D042-035 | The delivery row is inserted once, last in the apply transaction, with its final outcome; it is never updated and is deleted with the inbox row. A chain of only stateless stages takes no key row. Supersedes the order of D042-005 for `emit`; the application order remains the order of `skip` and `take` counts. | 2026-11-13 |
| D042-036 | A window whose run 022 buffers is `STARTED`, since the run exists; 022's D022-018 says "stays sealed" and is to be aligned (G042-011). | 2026-11-13 |
| D042-037 | Lock order: chain row, key row, window rows, then the admission port's rows, then quota and usage rows; the purge takes the chain row first. | 2026-11-13 |

## Gaps

| Id | Gap | State |
|---|---|---|
| G042-001 | `map` cannot build an object; the language has no object literal. | Open; a stated limit, kept by O042-005 |
| G042-002 | Whether seal and run start can share one transaction. | Open; T042-2 |
| G042-003 | A workflow with a `forEach` cannot have triggers (`published_collection_definition`), so a window cannot feed a collection yet. | Open; belongs to feature 014 or 039 |
| G042-004 | The Studio does not know the `forEach` node. | Open |
| G042-005 | The dispatcher is one queue for all tenants (G88 of feature 004); a key flood is bounded per event, not isolated. | Open; measured by T042-31 |
| G042-006 | Nothing enforces that every scheduler replica knows `trigger-operators` before it is enabled. | Open; waits for the instance registry |
| G042-007 | One hot key is serial by construction. | Open; reported by T042-33 |
| G042-008 | A collection that fails at entry stops the run; its `onError` is not taken (as for a failing `items` today). | Open; a follow-up of its own for feature 014's collections |
| G042-009 | The wall-time ceiling depends on the machine and could fail on a starved replica a chain another would finish. | Open; counted as `wall_time`, a defect to explain when it fires |
| G042-010 | Beyond plus or minus 2^53 `distinct` may keep an integer and a double that `==` calls equal. | Accepted; documented |
| G042-011 | Feature 022's D022-018 says a window whose run 022 buffers "stays sealed"; 042 records it `STARTED`. | Open; T042-3 aligns the 022 text |
| G042-012 | The storage-quota re-entry back-off is a per-replica hint; a lost hint costs one more evaluation. | Accepted |
| G042-013 | `take` after `flatMap` does not rescue a `flatMap` output that crosses a ceiling (stages are materialised). | Accepted; documented |
| G042-014 | The chain input ceiling (1,000) is below the 10,000 elements validation accepts for `maxItems`. | Open; waits for the T014-042 measurement deferred by OD-2026-11-13-1 |

## Owner questions

All nine were answered by the owner on 2026-11-13 ("Aceitar todas"), recorded as OD-2026-11-13-15.

| Id | Question | Decision | State |
|---|---|---|---|
| O042-001 | Which surface: events, collections or both | (c) Both, one vocabulary, two capabilities | Answered, OD-2026-11-13-15 |
| O042-002 | What goes in release 0.3.0 | (b) The collection operators only (S0 and S1); S2 to S5 in a later release | Answered, OD-2026-11-13-15 |
| O042-003 | Operators on `waitEvent` | (a) Not in the first delivery; `filter` as a follow-up | Answered, OD-2026-11-13-15 |
| O042-004 | Time base: arrival only, or also event time | (a) Arrival only | Answered, OD-2026-11-13-15 |
| O042-005 | Object construction in `map` | (a) Keep the language as it is | Answered, OD-2026-11-13-15 |
| O042-006 | The first operator set and the deferred list | The nine, and the deferred list, accepted | Answered, OD-2026-11-13-15 |
| O042-007 | The published numbers | As listed in the design | Answered, OD-2026-11-13-15 |
| O042-008 | Showing a key: digest only, or the key | (a) Digest unless the caller may reveal personal data | Answered, OD-2026-11-13-15 |
| O042-009 | A new key beyond `maxKeys` | (a) Fail, retry, dead-letter | Answered, OD-2026-11-13-15 |

## 2026-11-13 — Review of the first draft

The architecture, security and correctness reviewers read the design at `540154c8` (counts: architecture Blocker 3,
Major 10, Nit 8; security Blocker 2, Major 7, Nit 6; correctness Blocker 1, Major 12, Nit 10). Their findings, as
relayed by the coordinator, and how the second draft answers each are in
`docs/designs/042-stream-operators.reviews/2026-11-13/`. D042-016 to D042-037 and G042-008 to G042-014 come from that
review. D042-002 and D042-004 stand as decided by D042-012; D042-005's position order now governs only `skip` and `take`
counts (D042-035).

