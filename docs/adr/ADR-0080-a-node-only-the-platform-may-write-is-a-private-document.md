# ADR-0080: A node that only the platform may write is a private document, not a member of the public schema

**Status:** Proposed (2026-11-09). Accepting it is a person's: it fixes the shape in which the platform stores what it writes for itself.
**Decided by:** Agent proposal under the owner's delegation for feature 035 ("Tome as decisões por mim", 2026-11-02), made after the
`architecture-reviewer` found (finding 1 of its review of D035-150, 2026-11-09) that the first design, a node type outside the public
schema that "parses only for one origin", could not be stored: every restore validates against that schema.
**Refines / supersedes:** none. It follows the way [ADR-0071](ADR-0071-lake-writes-and-destinations-are-managed-effects-recovered-at-the-destination.md)
and feature 017 keep the platform's own runs (ad hoc and probe) as private definitions of a workflow flagged `internal`.
**Enforced by:** `crates/infra-ir/tests/private_documents.rs` (no public entry point accepts a private node; the private ones accept it and
give the domain a probe; a private document still obeys every other rule of the schema; an ordinary document parses to the same bytes
in both) and two tests in `crates/infra-ir/src/schema.rs` (the private schema is the public one plus the private node types and nothing
else, byte for byte; no private node type appears in the text of `workflow.schema.json`), `tests/integration/database-connections/lake_probe_start.rs`
(a private version restores through the private entry and a public one never does, whatever its document holds, both in the run store's
loader and in run creation, which reads the stored document for its declared state; the ad hoc endpoint and the REST probe cannot start a
probe) and `crates/application/tests/private_entry_sites.rs` (only five listed files mention a private entry: a file that is not on the
list fails the test, and so does a file on the list that no longer mentions one). The two source-text rules (only
two callers use a private entry) are added to `crates/application/tests/managed_kind_sites.rs` in that slice.

## Context

The probe of an Iceberg connection (D035-150, T035-045) is a run whose single node the platform writes and no author may. A SQL
connection's probe (feature 017) avoids the question: its node is an ordinary public `database` node (`SELECT 1`), which any author
could also write. A lake probe has no such node: it has no table, mapping or records, only a connection to test.

What the code does today (read, not assumed):

- `crates/infra-ir/src/lib.rs`: `validated_document` (authoring) and `validated_stored_document` (every restore of a stored version,
  published or private) both validate against the one embedded `workflow.schema.json` (`schema::issues`). A node type that is not in
  that schema cannot be authored, stored or restored.
- The public `activity` node has no member that can carry a connection name and a revision; a disguise as an ordinary activity with a
  reserved name would have to put the connection in `idempotencyTemplate` or `secrets`, which are for other things.
- `workflows.internal` (migration `20261125000000_database_ad_hoc`) already marks a private workflow, is immutable (a trigger), cannot be
  active, and is the scope of `private_execution_definitions`. The loader (`PgRunStore::definition`) reads `workflow_versions.document` by
  version id and knows nothing of it, but a join gives it.
- A version belongs to one workflow for its lifetime, so it is private or public for its lifetime, and a cache keyed by version cannot
  mix the two.

## Decision

1. A node type that only the platform may create (today, `icebergProbe`) MUST NOT be a member of `workflow.schema.json`.
2. The IR codec MUST offer a **private** twin of each of its two parse entry points (authoring and stored). They validate against the
   public schema extended with the private node types, built from that one embedded schema at start, never from a second copy. The
   **public** entry points MUST keep refusing a private node as they refuse any unknown node type.
3. A stored definition MUST be parsed privately only when its workflow is `internal`, and the loader MUST decide it from the row (a
   join on `workflows.internal`), never from the document. No publication path and no restore of a published version reaches a private
   entry.
4. The platform MUST be the only author of a private document: the entry that starts one (today, `probe_connection`) builds the document
   itself, and the start path compares its canonical form with what it was handed, as the SQL probe already does.
5. A private node type MUST be a managed kind (a variant of the managed payload of `Node`), so that every site that asks `managed_kind()`
   has to decide what it does with it (`crates/domain/AGENTS.md`: exhaustive matches, no wildcard arm).

## Considered options

1. **A private twin of the codec's entry points, loader signal `workflows.internal` (chosen).** The public contract does not change: no
   SDK type, no reference doc, no baseline learns a node no author may write. The cost is real and bounded: a second validator derived
   from the first, two entry points, one join in the loader.
2. **A reserved node type in the public schema, refused at publication.** Simpler to build (no mode, no join) and the stated **fallback**
   if the loader cannot tell the two apart without a migration (it can, so this is not taken). It lost because the contract only grows:
   a node no author may use would be in every generated type, every reference page and every baseline for ever.
3. **An ordinary public activity with a reserved name.** It lost because no public member can carry the connection honestly.
4. **A mechanism that is not a run** (a table of probe requests and a worker call that polls it). It lost because it needs a new wire
   call, a new lease, a retention rule and an audit shape for a thing a run already has all of.
5. **Run the probe in the control plane.** Forbidden by rule 7 of `AGENTS.md` (nothing runs inline in the control plane).

## Consequences

- Good: no public contract changes; the platform's own runs stay private; the exhaustive match on the managed payload makes a new
  private kind a decision at every site (the defect class that S0 of D035-150 removed).
- Bad: a second validator to keep equal to the first (a test holds it: the private schema is the public one plus the private types);
  the loader gains a join; the authoring path and the stored path each gain a variant.
- Rollback and mixed versions: a server of the version before this one cannot restore a private probe document and answers
  `UNSUPPORTED_NODE` for it, as D035-059 already accepts for writes. A probe document is private and lives for one run of at most 30
  seconds, so a rollout loses a probe, never a workflow.
- What becomes harder: a second kind of private node needs an entry in the private extension and a variant of the managed payload; it
  does not need a decision again unless it breaks one of the five rules.

## Evidence

- The review of D035-150, finding 1 (`specs/035-iceberg-write-steps/research.md`, D035-150, "History").
- `crates/infra-ir/src/lib.rs` (`validated_document`, `validated_stored_document`), `crates/infra-ir/src/schema.rs`,
  `crates/infra-postgres/src/execution.rs` (`definition`), `crates/infra-postgres/migrations/20261125000000_database_ad_hoc.up.sql`.
- The tests named under **Enforced by**, which are written first and seen failing in the slice that builds this.
