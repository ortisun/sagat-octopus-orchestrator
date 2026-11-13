# ADR-0087: Collection operators are applied once at scope entry and the snapshot is the record

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-11-13; the owner asked for the operators in OD-2026-11-13-13 and chose their scope in OD-2026-11-13-15, and has not accepted the mechanism)
**Refines / supersedes:** refines ADR-0032 (additive IR v1, one capability at a time, the frozen expression limits) and ADR-0030 (bounded expressions, scoped instances); builds on ADR-0003. Supersedes none.
**Enforced by:** proposed checks of design 042, slices S0 and S1: the IR contract fixtures (T042-6), the operator tests and the restart-on-snapshot test in `integration_workflow_primitives` (T042-7 to T042-9), and the six-language scenarios `collection-operators`, `collection-operators-runtime` and `collection-operators-limits` (T042-11). None exists yet; until they do, this rule is review only.

## Context

A `forEach` of feature 014 evaluates `items` once, snapshots the membership (`iteration_scopes`,
`collection_scope_items`) and runs its body per item. There is no way to drop, project, flatten, deduplicate or group
items in the definition; authors do it in an activity before the collection. The owner asked for reactive-stream
operators and decided that release 0.3.0 carries them on collections (design 042, O042-001 and O042-002).

The forces:

- **Replay never re-decides.** A collection's membership is read from its snapshot. Anything that shapes the list has
  to be part of what the snapshot records, or a replay could see another list.
- **No code in the engine.** Principle V and AGENTS rule 7 leave only the bounded expression language
  (`crates/infra-expr`): deterministic, no clock, no randomness, no I/O, no object literal, limits frozen by ADR-0032.
- **The schema is closed.** A new field is refused by a reader that does not know it (ADR-0032, decisions 2 and 8).
- **Scope entry is a transaction with locks.** Work added there lengthens them unless it is done before they are taken.

Unknown: the cost of a long chain over a large list at scope entry (to be measured, T042-9).

## Decision

1. A `forEach` MAY declare `operators`, one to eight stages from the closed set `filter`, `map`, `flatMap`, `skip`,
   `take`, `distinct`, `window {count}`, in any order. Every parameter that is not a number MUST be an expression of the
   existing language, validated at publication against the existing limits. The language MUST NOT be extended for this.
2. The chain is a pure function of the list `items` evaluated to and of the variables `items` already sees. It MUST be
   applied exactly once per scope, and its output MUST be what the membership snapshot and its digest record, in the
   transaction that writes them. No table, journal event or timer is added.
3. After the snapshot exists, nothing MUST evaluate the chain again: not a replay, a retry, a restart, a reconciliation
   or a read API. Body `index`, admission order and result order follow the snapshot.
4. `maxItems` bounds the output. The input keeps the present source ceilings, and the chain a work ceiling of 20,000
   expression evaluations per scope entry. Exceeding a ceiling, or an expression that fails, MUST fail the `forEach` at
   entry with a typed error through the path of a failing `items`; a list MUST NOT be truncated silently.
5. The field is additive in IR v1 behind the capability `collection-operators`. With it off, publication MUST refuse the
   field. An operator outside the set, or one that only a trigger has (`debounce`, `ignoreElements`, a time window),
   MUST be refused at publication.
6. The six SDKs MUST emit the same IR for the same chain and pass the same scenarios.

## Considered options

1. **Apply once, record in the snapshot** — no new state, and determinism for the reason collections already have it.
2. **A journal event per stage** — records intermediate lists nobody needs and multiplies stored bytes by the chain
   length; the snapshot already proves the result.
3. **Engine-run virtual activities** — a task, a lease and an attempt per stage for a pure function of one input.
4. **A sandboxed function per stage** — expressive, and a worker round trip, nondeterminism to police, and nothing the
   Studio can draw.
5. **Lazy operators evaluated per iteration** — membership would depend on when an iteration is admitted, which breaks
   the snapshot and feature 014's capacity accounting.

## Consequences

- Good: no migration, no scheduler change, no new failure mode; an author removes a preparatory activity.
- Bad: `map` selects or computes and cannot build an object (G042-001); a chain cannot see the body's results; scope
  entry does more work in the engine, bounded by decision 4.
- Compatibility: definitions without the field are unchanged. A binary that predates the field refuses a stored
  definition that has it, so readers ship before the capability is enabled (ADR-0032, decision 8).
- Rollback: disable the capability; publication refuses new chains, and existing scopes run from their snapshots.
- Nested collections (design 039) inherit the rule unchanged when they exist: one application per scope.

## Evidence

None yet. Design: `docs/designs/042-stream-operators.md` (FR-042-004, FR-042-005, SC-042-003, CUJ 3); registers:
`specs/042-stream-operators/research.md` (D042-010, G042-001). The measurements of T042-9 are added here when taken.
