# ADR-0143: The Studio draws node kinds from one registry checked against the workflow schema and enabled by the capabilities the server advertises; a kind the server refuses is never offered

**Status:** Proposed
**Decided by:** agent proposal (design 050, 2026-11-24), for acceptance under Owner decision 2026-11-24-4 once the review of design 050 is complete
**Refines / supersedes:** refines Architecture decision 0016 (the Studio is a same-origin application over the REST API) and Architecture decision 0032 (the workflow IR grows additively); supersedes research decisions Decision 145 and Gap 147 of feature 007 ("a node the schema accepts and the engine does not run yet" is shown in the palette, marked "does not run yet")
**Enforced by:** planned in design 050 and not existing yet: `frontend/apps/studio/src/model/registry.test.ts` (every `type` constant of `$defs/node` and `$defs/bodyNode` of `specs/001-platform-foundation/contracts/workflow.schema.json` has exactly one registry entry with its scopes, every registry entry names a schema kind, and every required property of each scope's member is a field or a handle of the entry); `tests/contract/workflow-studio/capabilities.rs` (every variant of `AdvertisedCapability` is in the `x-known-values` of `openapi.yaml` and the reverse, the list a test deployment advertises matches the switches it was started with, and a reader ignores an unknown name); the Studio unit test that a kind whose capability is absent is not in the palette and that a document holding a refused kind opens read-only on that node

## Context

The Studio (feature 007) keeps its node kinds in a hand-written list, `frontend/apps/studio/src/model/registry.ts`. On
`develop` at 2026-11-24 the list has twelve entries; the workflow schema has nineteen node kinds and fourteen body
kinds. The list offers two kinds the server refuses at publication (`foreach` and `humanApproval`:
`crates/infra-ir/src/convert.rs` answers "legacy collection placeholder has no executable definition" and "legacy
approval placeholder has no executable definition"), marked "does not run yet", and lacks `switch` and `approval`,
which the server publishes and runs, and `forEach`, `while`, `repeatUntil`, `return`, `continueAsNew` and
`icebergWrite`, which the server publishes only in fixed shapes or when a deployment switch is on (`unsupported` in the
same file: "this workflow capability is not enabled by
the runtime" in the basic path, while dedicated paths accept collections, loops and continuations in fixed shapes;
`lake.enabled` for `icebergWrite`; the collection-operators switch of design 042). Nothing tells the
Studio which of these a given server accepts, so the list was written for one moment of the server and drifted.

Three more features add capabilities the Studio must follow in release 0.3.0: internal apps (027, the `apps`
capability), the time-skipping test server (028, `virtual-clock`) and access control (029, a capability per item
kind). Each of their drafts invents its own advertisement: 027 "advertises the `apps` capability", 028 adds "an
additive `capabilities` list" to `PlatformVersion`, 029 adds "the additive `accessControl` capabilities of
`GET /version`".

## Decision

1. The Studio MUST hold its node kinds in one registry that is checked against the workflow schema by a unit test: one
   entry per `type` constant of the members of `$defs/node` and `$defs/bodyNode`, with its scopes and its required
   fields per scope, and no entry without a member. A new kind or a changed required field in the schema fails the
   Studio's tests until the registry follows.
2. `PlatformVersion` (`GET /version`) MUST carry `capabilities`, a list of strings of the form `<area>.<capability>`
   (for example `workflow.collection-operators`). The names MUST be deployment-wide, never per tenant. The OpenAPI
   document lists the shipped names as `x-known-values`, never as an `enum`, and a reader MUST ignore a name it does not
   know; an absent field means a server older than 0.3.0. A name is present when the replica that answers would
   accept the matching publication or route; it is advice for a client, never an authorization. The names are the
   variants of one typed enumeration in `crates/application` (`AdvertisedCapability`, distinct from the instance
   registry's `Capability` and from the workflow switches of `crates/infra-ir`). Every 0.3.0 feature that switches a
   behaviour on or off adds its name, with its source, when it ships, instead of a field of its own; a name that follows
   an operator's activation in the instance registry (Architecture decision 0082) MUST be derived from the activated
   requirements the registry member reads after a successful registration and on every epoch change and keeps beside
   its gate, through one tested mapping, which the first feature shipping such a name builds. A switch that already has
   an advertisement (the database drivers list for lake writes) keeps it and gets no second one.
3. A registry entry MUST name the capability it needs, or none. The palette MUST NOT offer a kind whose capability is
   absent. A document that already holds such a node, or a node of a kind the server refuses at publication for good
   (`foreach`, `humanApproval`), MUST open with that node shown and read-only, with the reason in words and the server
   problem from `validate`; the Studio MUST NOT drop, rewrite or convert the node without the author's explicit action.
4. A refused legacy kind MAY be replaced through a dialog that fills the fields that map exactly to the executable
   kind (`foreach` to `forEach`, `humanApproval` to `approval`) and asks the author for every field that does not; the
   replacement is an ordinary edit of the draft, undoable, and published only by the author.
5. The server stays the authority. Publication and `validate` decide. The shapes publication accepts (the publishable
   subset, narrower than the schema) are the codec's alone: the Studio MUST NOT write them again as rules; it shows a
   note per kind and marks a refused combination at its node from the editor's live validation. A capability shown by
   one replica and refused by another during a rolling upgrade costs the author a validation problem, never a wrong
   publication.

## Considered options

1. **One registry checked against the schema, kinds enabled by advertised capabilities (chosen).** The test makes
   drift a build failure; the capability list makes the palette follow the server it talks to; one list serves every
   feature.
2. **Keep the hand-written list and update it per feature.** No new contract. Rejected: it is how the Studio came to
   offer two refused kinds and to miss eight; nothing fails when it drifts.
3. **Offer every schema kind and rely on `validate`.** No capability contract. Rejected: an author builds a graph the
   server will refuse and learns it only when validating; the palette would show kinds that a deployment has
   switched off on purpose (lake writes).
4. **Generate the Studio's forms from the JSON Schema.** Least hand-written code. Rejected for 0.3.0: the schema says
   what is valid, not how a person edits it (handles, bodies, expressions, durations); the registry keeps those and
   the test ties it to the schema.
5. **A capability field per feature (`accessControl`, `apps`, `virtualClock`).** Each feature decides its own shape.
   Rejected: three shapes for one question, and a client must learn each.

## Consequences

- The palette changes with the server: a deployment without lake writes never offers `icebergWrite`. Against a server
  without the field (before 0.3.0) the Studio offers every kind that needs no name and lets validation decide.
- Feature 029's `accessControl` object of `GET /version` becomes entries of `capabilities` (one per item kind and
  state); feature 027's `apps` capability and feature 028's `virtual-clock` become entries too. Their documents are
  corrected in the same delivery as design 050's readiness pass.
- A document with a refused kind still opens, so no author loses work; publication still refuses it.
- Rollback: the field is additive; a client that ignores it behaves as before.

## Evidence

The drift is measured on `develop` at 2026-11-24 (design 050, The Problem): the registry has twelve entries against
nineteen node kinds; two of the twelve are refused by `crates/infra-ir/src/convert.rs`. The planned tests are listed
under **Enforced by**; none exists yet.
