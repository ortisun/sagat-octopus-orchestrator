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
