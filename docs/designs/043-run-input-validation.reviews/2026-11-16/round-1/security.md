# Security review of design 043, round 1 (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: f562614874016262dd9812e241f734038d4d70de
Reviewer: security (agent blob c02123dd239a7128f22e02c9dc39de8f110cc811)
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Read files only.

## Major

1. A panic in compile or validate is not caught for the leader of the single flight: a 500 in the control plane, and
   in the engine the supervised loop exits; the same work crash-loops every replica. No input that panics was found.
2. The pre-check a parse runs (publication, the dry-run validate, every ad hoc document) compiles up to 32 patterns on
   the async workers, outside the permits and with no limit on concurrency.

## Nit

3. `errors.md` and the `FieldPolicy` doc still describe graded disclosure; "never a fragment of the schema's text" is
   loose since `field` carries declared names.
4. The memory of unusable schemas has no per-tenant share.
5. The gate has no size ceiling of its own, and the continuation checks its size only after the gate.

Classes looked at, nothing found: tenant boundary, authorization and admission, disclosure of values, least privilege,
injection and parsing, races.

Counts: Blocker 0, Major 2, Nit 3
