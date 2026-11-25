# Constitution 5.0.0 — Sync Impact Report

This record accompanies the pull request that amends the constitution so that release 0.3.0 can hold everything needed
to surpass Temporal and Windmill. **Status: PROPOSED**; it is adopted by merging the pull request that carries it, on
three decisions of the owner of 2026-11-24, recorded in [`docs/owner-decisions.md`](../owner-decisions.md):

- Owner decision 2026-11-24-1 ("Inclua tudo na 0.3.0"): release 0.3.0 contains every approved draft (019 to 032, 036),
  all of 022, slices 2 to 5 of 042, designs 038 to 040, new designs 044 to 051, and features 013 and 015.
- Owner decision 2026-11-24-2 ("Aprovo a emenda"): the constitution is amended, MAJOR, to remove what blocks that scope:
  "reimplementing Temporal" leaves the out-of-scope list, "single region" becomes one writable region per cell with
  fenced failover (032), low-code internal apps (027) and a curated integration and template hub (021, 028) are
  admitted, and the declarative IR is joined by a procedural code-first mode. The three repository reviewers read it,
  and it merges without returning to the owner unless a reviewer finds a Blocker.
- Owner decision 2026-11-24-3 ("Sim, entra na 0.3.0"): workflow as code is built in 0.3.0 in all six SDKs, under a new
  design (044) that starts with a determinism study per language; the declarative graph stays a first-class model.

Owner decision 2026-11-24-4 ("Quero que os agents continuem por mim. Tomem a melhor decisão") lets the agents settle
contested or unfixed review findings in writing during release 0.3.0. This report uses it for the findings of the three
reviews, each written as "delegate (Owner decision 2026-11-24-4)" with its reasoning, with one limit taken from Owner
decision 2026-11-24-2: a Blocker of this amendment is either fixed, or, if the agents would contest it instead, it goes
back to the owner before the merge. Owner decision 2026-11-24-4 does not list amending the constitution among the acts
it delegates, and Owner decision 2026-11-24-2 names that condition for this amendment. The owner may overrule any of it
by a later amendment.

## Scope and why MAJOR

The amendment changes one principle, the Delivery Constraints and one sentence of the Development Workflow (the Owner
decisions clause); Governance is unchanged.

- **Principle IV is redefined.** Until 4.7.0 the unit of compatibility was "a published workflow version and its IR",
  and the Studio, the SDK builders and the engine all spoke one model, the declarative IR. In 5.0.0 a published version
  is either a declarative IR (the graph model) or a workflow function named by the digest of its code with a declared
  envelope (the procedural model, workflow as code). Every obligation of the old text is kept word for word except "and
  its IR", which no longer holds for every version, and the principle gains the rules that make the second model safe
  (below).
- **The Delivery Constraints change what is in and out of scope.** "Reimplementing Temporal or Restate" and "a
  connector marketplace" leave the out-of-scope list; "single region with high availability" becomes "one writable
  region per cell, with high availability inside the region"; a "Regions" bullet and an "Admitted scope" bullet are
  added; the out-of-scope list is rewritten to say exactly what stays out.
- **The Owner decisions clause gains one time-boxed sentence**: until release 0.3.0 is published, an act recorded as
  decided by `delegate (OD-2026-11-24-4)` counts as the owner's for that clause and for the Approval clause (see "The
  delegation and the constitution's own text").

A principle redefined incompatibly is MAJOR under Governance, Versioning. Principle IV could be read as merely expanded
(MINOR), because no old obligation is dropped, but the owner asked for MAJOR and the conservative reading agrees: a
reader of 4.7.0 could rely on "every published version has an IR", and in 5.0.0 that is no longer true.

## Complete clause impact

| Unit of 4.7.0 | Disposition | Old | New | Why |
|---|---|---|---|---|
| Principles I, II, III, V, VI, VII, VIII | KEEP | — | Byte-identical; hashes below | The new scope is met by the rules as they stand (see "What stays and why") |
| Principle IV, first paragraph | REWRITE | "A published workflow version and its IR are immutable …" | "A published workflow version is immutable …"; the sentences on wire contracts, the one semantic contract of the six SDKs and pinned toolchains are kept verbatim | A procedural version has no IR (Owner decision 2026-11-24-3) |
| Principle IV, second paragraph | ADD | — | The two models share one engine, one journal and one semantic contract; the procedural model's rules (below) | Owner decisions 2026-11-24-2 and 2026-11-24-3 |
| Principle IV, Studio sentence | KEEP (moved) | "The Studio draws the declarative IR; imperative code that cannot be converted is not promised a round trip." | The same, with "and shows a procedural run as the graph observed from its journal" and "converted to the IR" | Principle VI already makes the runtime graph a projection of the journal |
| Principle IV, rationale | REWRITE | ADR-0003, ADR-0007, overview, rule 8 | adds design 044 and its records ADR-0103 to ADR-0109, `workflow-ir-and-sdk-modes.md` and Owner decision 2026-11-24-3 | Sources of the new rules |
| Delivery Constraints, technology baseline | REWRITE | "graph-native declarative IR; single region with high availability" | "two authoring models on one engine, the graph-native declarative IR and the procedural code-first model of Principle IV; one writable region per cell, with high availability inside the region" | Owner decision 2026-11-24-2 |
| Delivery Constraints, Regions | ADD | — | Standby regions and active-passive failover by an operator's activation that raises a database-enforced region epoch, with a written attestation for an unplanned activation (032, ADR-0067); cells share no data and no transaction spans two cells; different cells may be writable in different regions; production only after the region drill passes and the commit-latency benchmark is published; a mode that can lose acknowledged work says so, reports the loss and never claims a recovery point objective of zero | Owner decision 2026-11-24-2; keeps ADR-0067's evidence gate (Decision 1592 of feature 032) |
| Delivery Constraints, Admitted scope | ADD | — | Workflow as code; internal apps over declared operations, sessions no wider than the viewer's own authority, author code only in an isolated frame (027, ADR-0062); a curated hub whose only sources are the release-signed index and operator indexes with operator-pinned keys, items imported as drafts under the authored-content rules, no item carrying code the server executes (021, 028, 047) | Owner decision 2026-11-24-2 |
| Delivery Constraints, out of scope | REWRITE | "multi-region active-active, exactly-once physical execution of external effects, a connector marketplace, WASM as a mandatory runtime, reimplementing Temporal or Restate, and automatic migration of arbitrary code" | "two regions accepting writes for the same cell …; exactly-once physical execution of external effects; an open marketplace …; a generic application builder with its own backend, database or server-side author code; wire, protocol or history compatibility with another engine …; distributed transactions across cells; WASM as a mandatory runtime; and automatic migration of arbitrary code" | Each removal is the owner's; each item kept or added is explained below |
| Delivery Constraints, Targets are not promises; Security-by-default | KEEP | — | Unchanged | — |
| Development Workflow, Owner decisions | REWRITE (one sentence added) | — | Until release 0.3.0 is published, an act recorded as decided by `delegate (OD-2026-11-24-4)` counts as the owner's for this clause and the Approval clause, within that decision's scope; publishing the release, spending money, creating accounts, irreversible acts outside the repository and amending the constitution stay with the owner; `scripts/check-design-docs.py` names the delegation it accepts | Owner decision 2026-11-24-4, whose Covers column names "docs/constitution.md (approval and acceptance clauses)"; the architecture review's Major 8 |
| Every other Development Workflow bullet | KEEP | — | Unchanged | — |
| Governance | KEEP | — | Unchanged | — |
| Footer | REWRITE | `Version: 4.7.0`, last amended 2026-11-20 | `Version: 5.0.0`, last amended 2026-11-24, this report, the adoption sentence | — |

Every sentence of 4.7.0 that uses MUST or MUST NOT is present word for word in 5.0.0 (line breaks aside; 36 sentences,
checked by splitting both texts on sentence ends and searching each old sentence in the new text). Principle IV adds
three: the workflow function MUST NOT perform an external effect itself; a diverging replay MUST stop with a typed,
audited non-determinism error and commit none of its commands; the procedural model MUST NOT ship in an SDK before that
language's determinism study and the shared replay conformance scenarios pass. The kept sentence that the six SDKs MUST
NOT fork the server semantics now covers replay too.

## The procedural model: why these rules and no others

Workflow as code is Temporal's programming model: a function in the author's language whose control flow decides the
next step, replayed against the history after a crash. The constitution has to admit it without giving up what makes
this platform's runs explainable. It states obligations; design 044 and its records (ADR-0103 to ADR-0109, proposed)
hold the mechanism, and the two were read against each other for this amendment. Each sentence closes one way it could
go wrong:

- **One engine, one journal, one semantic contract.** Two engines would fork the server semantics, which Principle IV
  already forbids; the procedural model is a second way to write a version, not a second runtime on the server
  (ADR-0103: "There MUST be no second execution engine").
- **The version names the function by the digest of its code and declares an envelope.** The envelope (activities,
  secrets, signals, child workflows, scripts, approvals) is what the publisher's permissions are checked against at
  publication, as a graph version's nodes are (Requirement 044-013).
- **The function runs in an SDK worker, never in a platform process.** Principle V already forbids untrusted code inline
  in the control plane or the scheduler; the sentence says where the code does run, and "a platform process" also
  covers replay checks and queries (ADR-0103 decision 1).
- **The server checks each command against the envelope, the current authorization and the per-run ceilings.** Without
  this, a worker holding `worker:execute` could ask at run time for a secret, a child workflow or a fan-out the publisher
  never declared, because Principle V releases secrets only as "declared by its immutable task definition" and in this
  model the definition is created by a command. The rule makes a command no more powerful than a graph node with the
  same declarations. The ceilings (history length, payload size, fan-out) replace the publication-shape limits of the
  graph's loops and collections, which a function does not have.
- **Accepted as a worker result under Principle I and committed with the journal and the outbox in one transaction.**
  A workflow task's result is a worker result like any other: valid lease, fencing token, stale attempts refused. This
  keeps Principle I whole without rewriting it: the next task is still scheduled in the transaction of the transition,
  by the server.
- **The worker's memory is a cache that replay can discard and rebuild.** PostgreSQL stays the authority (Principle I);
  sticky execution is an optimisation, never a record.
- **No external effect in the function itself; effects are activities under Principle III; the SDK's guard refuses or
  reports one.** Code outside an activity runs again on every replay. The rule binds the author, so the SDK's
  determinism guard is what makes it observable (ADR-0106, ADR-0107: no local activity).
- **A diverging replay stops with a typed, audited error and commits nothing.** Accepting the commands of a replay that
  disagrees with the journal applies a transition the history does not explain. Design 044 holds the run with a cause
  (`CHAIN_MISMATCH`, ADR-0104); it is never a silent continuation.
- **A run pins one version at any moment; the pin changes only by a verified move.** The move is an explicit request
  per run by a person allowed to operate it, applied only when a replay of the run's own history against the target
  code reproduces the journaled commands and every outstanding step fits the target's envelope; it is journaled and
  audited, a refused move changes nothing, and no policy moves runs by itself. This is how a fix reaches a run of
  months without the automatic migration that the first paragraph forbids: the platform never guesses, and the replay
  proves the move before it is applied (ADR-0105; gap 044-010 asked the constitution to say what a verified move is).
  Version markers in the code let one version branch for older histories, as Temporal's patches do.
- **The history and command format is a wire contract.** Open runs pinned to older code must keep replaying it across
  upgrades of the server, N and N-1 included (design 048).
- **Ships in an SDK only after that language's determinism study and the shared replay conformance pass; an SDK
  without it says so.** Owner decision 2026-11-24-3 puts the study first; Principle VIII forbids a guarantee that is not
  proven. An SDK that does not offer the model declares it in its capabilities and its procedural scenarios are reported
  as not offered, never as passed. Owner decision 2026-11-24-3 asks for the model in all six SDKs and Owner decision
  2026-11-24-1 publishes the release only when everything is built, so shipping 0.3.0 with a language short is a release
  decision, and it goes to the owner.

## Regions: why active-active inside a cell stays out

The owner left this to the amendment. Three readings were weighed.

1. **Admit multi-region active-active.** Two regions accept writes for the same runs and reconcile afterwards.
2. **Admit active-passive per cell only, and say nothing about the fleet.**
3. **Admit active-passive per cell, keep two writable regions for the same cell out, and say that different cells may
   be writable in different regions.** (Chosen.)

Why not 1. Principle I requires a transition, the next tasks, the timers and the outbox to commit in one transaction,
and a transition applied twice or lost after commit cannot be repaired. Run state, quotas, permits, approvals, receipts
and leases are mutable rows, not a mergeable log. Two writers for one cell need either a consensus commit across
regions on every transition (a cross-region round trip in every commit) or conflict resolution after the fact, which
accepts a split brain by design (ADR-0067, alternative 2). Neither is a gap against the competitors: in Temporal's
multi-cluster replication a namespace is active in one cluster at a time and a split brain is resolved after the fact by
failover versions, and Windmill runs one database per installation. A per-cell epoch fence that refuses rather than
merges is the stronger guarantee, and it is what 032 specifies.

Why 3 over 2. The business reason for active-active is usually locality and blast radius, not two writers for the same
row. Cells give both: a tenant has a home cell (032, owner question 1562), and the cells of a fleet can be writable in
different regions, so the platform serves Europe from a European cell and America from an American one while each cell
keeps one authority. Saying so in the constitution prevents a later reader from taking "one writable region" for "one
region for the whole fleet". "Cells share no data and no transaction spans two cells" (032: "cells share nothing") and
"distributed transactions across cells" in the out-of-scope list keep multi-cell from becoming a distributed database.

What the constitution now requires of failover, beyond 4.7.0:

- The evidence gate that ADR-0067 and Decision 1592 put on the amendment itself moves into the text, because the owner
  chose to amend before the evidence exists. It is a gate on a result, not on a document: the region drill must pass (no
  acknowledged work lost in the synchronous quorum mode, which Task 032-045 defines as a failure) and the commit-latency
  benchmark must be published, each with its environment and failures. Until then a release may contain 032 built, but
  not offered for production; release 0.3.0 can therefore ship it as built and say so.
- The epoch fences what was granted before the activation on the new authority. A former primary that is not fenced
  could still act on its own timeline in an unplanned activation, so that activation needs the operator's written
  attestation first (ADR-0067 decision 7).
- The asynchronous mode and a forced activation, which can lose acknowledged work, must say so before they are chosen,
  count and report the loss afterwards, and never claim a recovery point objective of zero. That keeps Principle VIII
  and "Targets are not promises" true for the new topology, and it is why Principle I needs no change: in those modes the
  platform does not hide the loss, it measures it.

## The other items of the out-of-scope list

- **Reimplementing Temporal or Restate** leaves, as the owner decided. The owner named Temporal; "Restate" leaves with
  it because the old item excluded them together, and keeping one of the two would exclude a programming model that
  the owner admitted. What the old item protected is kept by a narrower one: **wire, protocol or history compatibility
  with another engine** (running Temporal or Restate SDKs against this server, importing their histories). The platform
  adopts Temporal's programming model, not its protocol. A compatibility layer would bind the server to another
  engine's history format, failover versions and error model, which is a fork of the server semantics that Principle IV
  forbids, and every change of theirs would become a change of ours. Nothing in release 0.3.0 asks for it (no feature or
  design mentions importing a Temporal or Windmill history); migration is served by the SDKs' own procedural model and
  by the benchmark of design 051. Delegate (Owner decision 2026-11-24-4): recorded here as a scope decision with this
  reasoning, for the most architectural advantage.
- **A connector marketplace** becomes **an open marketplace (publishing by tenants or third parties, unsigned or
  unpinned sources, ratings, payments)**. Curation is a property of the source, as feature 028 already designed it
  (Decision 1434): the release-signed index and operator indexes with operator-pinned keys. The admission permits
  operator indexes and forbids tenant or public sources; whether 028 offers operator indexes is still its owner question
  1402. Hub items are imported as drafts under the authored-content rules and no item carries code the server executes
  (the server fetches and verifies indexes; it executes nothing from them), so a hub item can never be a way around
  Principle V's isolation or the publication review. Resource types as hub items stay 021's owner question 1122.
- **A generic application builder with its own backend, database or server-side author code** is new in the
  constitution; it was a non-goal of `docs/architecture/overview.md` only. Feature 027's bounded builder is admitted and
  the generic one is kept out, as owner question 1360 recommended: an app backend running author code in the server is
  what Principle V forbids, and a database per app is a second authority beside PostgreSQL.
- **The viewer's own authority** is written into the admission of internal apps because it is the security property
  that makes an app safe to share (ADR-0062). A delegated mode, in which an app acts with its publisher's authority
  (Windmill's default; owner question 1361, recommended "no"), would widen authority through a page. Feature 027 said a
  delegated mode would need a separate ADR; the constitution now says it needs its own ADR and an amendment, so that it
  cannot arrive as an implementation detail. That raises the bar on owner question 1361 without answering it; the
  question stays with the agent that takes feature 027.
- **Exactly-once physical execution of external effects, WASM as a mandatory runtime and automatic migration of
  arbitrary code** stay, unchanged in meaning: the procedural model makes the first one more important, not less, and
  converting arbitrary code into the IR is still not promised.

## What stays and why

- **PostgreSQL is the authority and the broker is a hint** (Principles I and VII, byte-identical). The procedural
  model's commands commit in the server's transaction; a standby region's epoch is a database row checked by the
  database.
- **No exactly-once for external effects** (Principle III, byte-identical), now also required of the workflow function.
- **Untrusted code is isolated** (Principle V, byte-identical). The workflow function runs in the author's SDK worker;
  its commands are bound to the version's envelope; internal apps run author code only in an isolated frame; no hub item
  carries code the server executes.
- **Principle VIII** (byte-identical) is what the determinism study and the region drill answer to.

## The delegation and the constitution's own text

The constitution says that product, scope, budget, release and security-exception decisions belong to the owner and are
recorded in `docs/owner-decisions.md`, and that a review finding the owner accepts in writing there is not open. The
first draft of this report read that as already compatible with Owner decision 2026-11-24-4, since a delegation is
itself an owner decision. The architecture review showed that the repository's own check disagrees:
`scripts/check-design-docs.py` accepts as a gate or an acceptance only a row whose "Decided by" cell is exactly `owner`,
and its tests (`test_a_delegate_row_is_not_citable`, `test_a_delegate_acceptance_fails`) say so on purpose, because
design 4.0.0 did not want an agent to open its own gate. So delegate acts under Owner decision 2026-11-24-4 would either
fail CI or be written down as the owner's, which misattributes them. The owner's decision names "docs/constitution.md
(approval and acceptance clauses)" in its Covers column.

Three options were weighed:

1. **Leave the text and write delegate acts as "owner".** Rejected: it falsifies the record.
2. **A standing delegation clause, and a checker that accepts any `delegate` row.** Rejected: the delegation is scoped to
   release 0.3.0, and the protection that design 4.0.0 chose would be lost for good.
3. **One time-boxed sentence that names the delegation, and a checker that accepts only it.** (Chosen.) The sentence
   says that until release 0.3.0 is published an act recorded as `delegate (OD-2026-11-24-4)` counts as the owner's for
   the Owner decisions and Approval clauses, within that decision's scope, and repeats what stays the owner's:
   publishing the release, spending money, creating accounts, irreversible acts outside the repository and amending this
   constitution. `scripts/check-design-docs.py` gains the constant `DELEGATIONS = ("OD-2026-11-24-4",)`: a row decided by
   `delegate (<id>)` is citable only when `<id>` is in that list and its own row is an owner row with status `Decided`.
   A bare `delegate`, a delegation not listed, a delegation whose row is missing or is itself a delegate row are still
   refused. Five tests were added and seen failing before the change (the two that should pass failed with "is not an
   owner row"); the 128 tests of the script pass after it. The pull request that publishes release 0.3.0 removes the
   entry, and from then on delegate rows stop satisfying a gate or an acceptance.

The EGM pin keeps requiring the owner's own reading of the plugin's diff; the delegation does not name it.

## Principle hashes

Previous-source: 5459c214aa5436caa2cb484907a23eb9adcf52a1

The line above is the git blob of `docs/constitution.md` at 4.7.0. A principle block is hashed as in the 4.0.0 report.

| Principle | Disposition | sha256 (previous) | sha256 (this version) |
|---|---|---|---|
| I | KEEP | 32d2ec982a4ae0995a472628184c4762ea740f9081886bb4c78fa461237a5a21 | 32d2ec982a4ae0995a472628184c4762ea740f9081886bb4c78fa461237a5a21 |
| II | KEEP | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 |
| III | KEEP | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 |
| IV | REWRITE | ff10043d76dfd25ac3b2db1ea27b943804c52d4d46c83838e6908cdaccc32f7b | 44579587cf16c115dbe57e15b1cf5f93e28f65a867860a1386811605360438a3 |
| V | KEEP | b816cb76163ef1d8cc8579bb3e326f59c774b077131029cfd64fa45d470f38f9 | b816cb76163ef1d8cc8579bb3e326f59c774b077131029cfd64fa45d470f38f9 |
| VI | KEEP | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc |
| VII | KEEP | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 |
| VIII | KEEP | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 |

## What enforces each new rule

The constitution's new rules have no check yet, except the delegation. Each row names the check that must exist and who
builds it; until then the rule is held by review.

| Rule | Check today | Check to add, and where |
|---|---|---|
| Only the named delegation counts as the owner's | `scripts/check-design-docs.py` (`DELEGATIONS`) and its five tests | removed by the pull request that publishes 0.3.0 |
| The workflow function never runs in a platform process | review | `scripts/check-architecture.py` refuses embedded language runtimes (`deno_core`, `v8`, `pyo3`, `boa_engine`, `mlua`, `wasmtime`) in the dependency graph of the server crates (design 044) |
| A command stays within the envelope | none | Requirement 044-013, one test per envelope member |
| A diverging replay commits nothing | none | ADR-0104's `a_completion_with_a_foreign_command_chain_is_refused_and_holds_the_run` and the conformance scenario `code-nondeterminism` |
| A pin changes only by a verified move | none | ADR-0105's trigger test `the_version_of_a_run_changes_only_through_the_move_function` |
| No effect in the function | none | a conformance scenario that injects an effect outside an activity; the SDK guard must refuse or report it (ADR-0106, ADR-0107) |
| An SDK without the model says so | none | a capability declaration per SDK in the conformance suite; the procedural scenarios report "not offered" for it (design 044) |
| Cross-region failover is not offered before its evidence | none | Task 032-063, and a step of the release workflow that fails if the documents or release notes call it supported while the drill or the benchmark report is missing or failing (feature 032) |
| Hub sources and viewer authority | none | the planned tests of 028 (signed and operator-pinned indexes) and 027 (viewer-narrowed sessions) |

## Dependents updated in this change

- `docs/architecture/overview.md`: the section "Explicit scope left for later releases" becomes "Explicit scope left
  out", says what 0.3.0 admits and lists what stays out, in the constitution's words.
- `docs/architecture/workflow-ir-and-sdk-modes.md`: the procedural row is no longer "later phase", its "executes
  arbitrary code" cell names the workflow function itself; the paragraph under the table states the Principle IV rules
  and points to design 044.
- `docs/architecture/execution-semantics.md`: the code-orchestrated mode is "release 0.3.0, design 044, not built", and
  the opening paragraph says it describes the engine as built.
- `sdks/SPEC.md`: a dated note under "An SDK is not a workflow runtime".
- `docs/adr/ADR-0003-declarative-ir.md` and `docs/adr/ADR-0009-single-region-first.md`: a dated note each; both stay
  accepted until the records that refine or supersede them in part (design 044's ADR-0103 to ADR-0109, ADR-0067) are
  accepted on their evidence.
- `docs/adr/ADR-0051-…` and `docs/adr/ADR-0059-…`: a dated note each (code runs replay; the procedural mode is no longer
  deferred). `specs/019-worker-deployments-dispatch/spec.md`: two notes (code runs change version only by a verified
  move, never through 019's pins or auto-upgrade; replay concepts are design 044's).
- `docs/adr/ADR-0067-…`: status line and the section "Constitution amendment required (stated, not applied)", which
  becomes "applied in 5.0.0, ahead of the evidence", with the evidence gate now in the constitution; the index row in
  `docs/adr/README.md`.
- `docs/adr/ADR-0062-…`: a note that owner question 1360 is answered; acceptance of the record is still open.
- Feature 032 (`spec.md`, `research.md`, `plan.md`, `tasks.md`): owner question 1560 marked answered by Owner decision
  2026-11-24-2; Decision 1583 superseded in part; the Delivery Constraints row of the Constitution Check reads Pass;
  tasks 032-001 and 032-063 annotated, not ticked.
- Feature 027 (`spec.md`, `research.md`, `plan.md`, `tasks.md`): owner question 1360 answered as recommended, gap 1360
  closed; task 027-001 annotated, not ticked.
- Feature 028: `spec.md` (the reason for curation cites the admission; replay concepts belong to design 044),
  `plan.md` (the Delivery Constraints row reads Pass against "Admitted scope"), `research.md` (owner question 1402: the
  admission permits operator indexes; the question stays open).
- Feature 021: `research.md` (owner question 1122 noted against the admission) and `spec.md` (the out-of-scope
  marketplace line).
- Feature 025 (`spec.md`): its out-of-scope marketplace line cites the 5.0.0 wording.
- Specifications 001 to 012: a dated note under each "V1 single-region HA" and "Reimplementing Temporal/Restate"
  assumption line (24 notes), as Decision 1583 asked: annotated, not rewritten, because those features' scope does not
  change.
- `SPECIFICATION-INDEX.md` and `docs/roadmap.md`: the gates that 5.0.0 settles are marked settled; the roadmap rows of
  027 and 032 and the parity paragraph follow; the 0.3.0 paragraph links this report.
- `docs/runbooks/disaster-recovery.md`: the multi-region case is 032's, admitted and not built.
- `docs/owner-decisions.md`: the status of Owner decision 2026-11-24-2 becomes "Decided; amended in 5.0.0 (pull request
  #275)", and its Covers cell links this report.
- `scripts/check-design-docs.py` and `scripts/tests/test_check_design_docs.py`: the named delegation (above).
- `docs/governance/README.md`: the row of this report.

Not changed, on purpose:

- `AGENTS.md`, `CLAUDE.md` and `README.md` repeat none of the changed clauses.
- The approved design docs 038, 042 and 043 say "single region" in their Constitution Check rows for the Delivery
  Constraints. They remain true under 5.0.0 (each cell has one writable region and those designs add no region), and an
  approved design doc returns to draft if its text changes.
- The plans of features 001 to 012, 014, 016, 022, 025 to 027, 029 to 031 and 036 that assume "single-region" describe
  their own scope, which does not change; the specifications' notes cover the assumption once per feature.
- `sdks/go/README.md`, `sdks/go/doc.go`, `sdks/python/README.md` and `sdks/dotnet/README.md` say an SDK is not a
  workflow runtime. They describe the SDKs as built, and design 044 changes them with the code (documentation first:
  the normative `sdks/SPEC.md` carries the note now).
- "Technology baseline (V1)" and "Out of scope for V1" keep "V1": it names the product line before 1.0, which every
  release so far, 0.3.0 included, belongs to.
- `docs/roadmap-assessment-2026-11-11.md` is a dated assessment; it records what was true that day.

## Follow-ups that are not this change's

- **Retention in feature 020 still needs an amendment of Principle I** (ADR-0054, gap 1101): removing whole journals of
  terminal runs changes an obligation of Principle I, and Owner decision 2026-11-24-2 does not name it. The agents may
  draft it as 5.1.0 when 020 reaches its user stories 08 and 09, with its own three reviews, but amending the
  constitution is not among the acts Owner decision 2026-11-24-4 delegates, so the owner approves it.
- **Design 044** carries the mechanism and the checks of the table above; its gap 044-010 (what a verified move is) is
  answered by Principle IV's text.
- **Feature 032** keeps task 032-063: ADR-0067 `Accepted`, ADR-0009's "Superseded in part" line, the release-workflow
  check, and the first production statement of cross-region failover, all on a passing region drill and a published
  commit-latency benchmark.
- **Feature 027**: owner questions 1361 and 1362 and the acceptance of ADR-0062, for the agent that takes the feature.
- **Design 047**: the integration hub's sources, signatures and import path follow the admission.
- **Release 0.3.0's publishing pull request** removes `OD-2026-11-24-4` from `DELEGATIONS` in
  `scripts/check-design-docs.py`, and the time-boxed sentence of the Owner decisions clause expires with it (a PATCH
  amendment that deletes the sentence).

## Review

The read-only `architecture-reviewer`, `security-reviewer` and `correctness-reviewer` read the diff of this pull request
one after the other, with the four owner decision rows of 2026-11-24 and no conclusion of the author's. Each finding
is listed with what was done. "Fixed" means the text above and the diff changed; "settled" means it was not changed,
with the reasoning, as delegate (Owner decision 2026-11-24-4).

### Architecture review (no Blocker; eight Major, ten minor, three Nit)

| Finding | Disposition |
|---|---|
| Major 1: the authoring-model baseline changes with no architecture decision record | Fixed differently. Design 044 already proposes ADR-0103 to ADR-0109 (branch `feature/044-workflow-as-code`), which record the alternatives the reviewer lists (replay in the engine, a second engine, code compiled to the IR) and the enforcement tests. A separate ADR-0102 would duplicate them, so none is written and the number stays unused. Principle IV's sources and the ADR-0003 note cite them |
| Major 2: Principle IV contradicted itself on version, build and version marker | Fixed, aligned with ADR-0105: a run pins one version at any moment, named by code digest, and changes it only by a verified move (explicit, authorized, per run, after a matching replay, journaled and audited, never by a policy); version markers branch inside one version. The reviewer's "ordered line of builds" was considered and rejected because design 044 already chose digest pins and verified moves, and the constitution should not contradict the design that implements it |
| Major 3: feature 019, ADR-0051 and ADR-0059 not listed as dependents | Fixed: notes added and listed |
| Major 4: commands bound to Principle I but not to Principle V or ceilings | Fixed: the envelope, the current authorization and the per-run ceilings are in Principle IV |
| Major 5: the per-language gate against "six SDKs" and the release rule | Fixed: an SDK without the model declares it and its scenarios report "not offered"; shipping 0.3.0 with a language short goes to the owner |
| Major 6: the failover gate could be met by a failed drill | Fixed: "after its region drill passes (no acknowledged work lost in the synchronous quorum mode) and its commit-latency benchmark is published"; built-but-not-offered is stated; the release-workflow check is in the enforcement table and the follow-ups |
| Major 7: the report let the delegate settle what stays the owner's | Fixed: a contested Blocker of this amendment goes to the owner, and the amendment of Principle I for retention is approved by the owner |
| Major 8: the checker refuses delegate rows | Fixed by option 3 of "The delegation and the constitution's own text": one time-boxed sentence and a checker that accepts only the named delegation, with five tests |
| minor: the report contradicted its diff on the status of Owner decision 2026-11-24-2 | Fixed: listed under dependents, follow-up removed |
| minor: stale dependents (execution semantics opening paragraph, 021 owner question 1122 and its spec line, 028's Constitution Check row) | Fixed |
| minor: owner questions 1402 and 1361 decided silently | Fixed differently: neither is answered here; the 028 note says the admission permits operator indexes and the question stays open, and the report says the constitution raises the bar on 1361 and leaves the question with feature 027's agent |
| minor: the compatibility exclusion was recorded only "if contested", and dropping "Restate" goes beyond the owner's words | Fixed: recorded as a delegate scope decision with its reasoning, including why "Restate" leaves with "Temporal" |
| minor: "nothing granted before the activation can act after it" overclaimed | Fixed: "on the new authority", and the written attestation for an unplanned activation |
| minor: cell isolation and distributed transactions across cells missing | Fixed: "Cells share no data and no transaction spans two cells"; "distributed transactions across cells" is out of scope |
| minor: the history and command format is not named a wire contract | Fixed |
| minor: "MUST NOT perform an external effect" binds the author and nothing checks it | Fixed: the SDK's determinism guard refuses or reports; the enforcement table names the conformance scenario |
| minor: sticky cache across region epochs and history disclosure under feature 016 and 031 rules | Handed to design 044 (follow-ups): those are mechanisms, and the constitution's "the worker's memory is a cache, never a record" and Principle V already bind them |
| minor: "runs nothing in the server" was ambiguous; resource types as hub items | Fixed: "none of whose items carries code that the server executes"; resource types stay 021's owner question 1122 |
| Nit: "V1" | Settled: kept, it names the product line before 1.0 |
| Nit: "Executes arbitrary code: Yes, only in activities and steps" | Fixed |
| Nit: plans and SDK READMEs not named under "Not changed, on purpose" | Fixed |
| Fitness: no rule has an "Enforced by" | Fixed: the enforcement table above |

### Security review

(Filled in after the review.)

### Correctness review

(Filled in after the review.)

## Risks and rollback

- The procedural model is admitted before its code exists. The constitution's rules bound what design 044 may decide;
  if the determinism study shows a rule cannot be met in a language, that language does not get the model, and the rule
  is not weakened without a new amendment.
- The region topology is admitted before its evidence exists. The "Regions" bullet keeps production use behind a
  passing drill and a published benchmark, so an installation cannot claim cross-region failover on this text.
- The named delegation is a hole in the owner-only gate for the length of release 0.3.0. It is narrow (one decision,
  checked to be an owner row) and it is removed when the release is published.
- Rollback is a forward amendment (6.0.0) that restores the 4.7.0 Delivery Constraints and Principle IV; nothing in the
  code depends on this text yet, except the checker's `DELEGATIONS`, which a revert removes.
