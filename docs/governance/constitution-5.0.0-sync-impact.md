# Constitution 5.0.0 — Sync Impact Report

This record accompanies the pull request that amends the constitution so that release 0.3.0 can hold everything needed
to surpass Temporal and Windmill. **Status: PROPOSED**; it is adopted by merging the pull request that carries it, on
three decisions of the owner of 2026-11-24, recorded in [`docs/owner-decisions.md`](../owner-decisions.md):

- Owner decision 2026-11-24-1 ("Inclua tudo na 0.3.0"): release 0.3.0 contains every approved draft (019 to 032, 036),
  all of 022, slices 2 to 5 of 042, designs 038 to 040, new designs 044 to 051, and features 013 and 015.
- Owner decision 2026-11-24-2 ("Aprovo a emenda"): the constitution is amended, MAJOR, to remove what blocks that scope:
  "reimplementing Temporal" leaves the out-of-scope list, "single region" becomes one writable region per cell with
  fenced failover (032), low-code internal apps (027) and a curated integration and template hub (021, 028) are
  admitted, and the declarative IR is joined by a procedural code-first mode. The three repository reviewers read it.
- Owner decision 2026-11-24-3 ("Sim, entra na 0.3.0"): workflow as code is built in 0.3.0 in all six SDKs, under a new
  design (044) that starts with a determinism study per language; the declarative graph stays a first-class model.

Owner decision 2026-11-24-4 ("Quero que os agents continuem por mim. Tomem a melhor decisão", recorded by pull request
#274) is the authority, during release 0.3.0, for the acceptances that the constitution assigns to a person: settling a
contested or unfixed review finding of this amendment, and any acceptance this report records, are written as
"delegate (Owner decision 2026-11-24-4)" with their reasoning. The owner may overrule any of it by a later amendment.

## Scope and why MAJOR

The amendment changes one principle and the Delivery Constraints; the Development Workflow and Governance sections are
unchanged.

- **Principle IV is redefined.** Until 4.7.0 the unit of compatibility was "a published workflow version and its IR",
  and the Studio, the SDK builders and the engine all spoke one model, the declarative IR. In 5.0.0 a published version
  is either a declarative IR (the graph model) or a workflow function with a pinned build (the procedural model,
  workflow as code). Every obligation of the old text is kept word for word except "and its IR", which no longer holds
  for every version, and the principle gains the rules that make the second model safe (below).
- **The Delivery Constraints change what is in and out of scope.** "Reimplementing Temporal or Restate" and "a
  connector marketplace" leave the out-of-scope list; "single region with high availability" becomes "one writable
  region per cell, with high availability inside the region"; a "Regions" bullet and an "Admitted scope" bullet are
  added; the out-of-scope list is rewritten to say exactly what stays out.

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
| Principle IV, rationale | REWRITE | ADR-0003, ADR-0007, overview, rule 8 | adds `workflow-ir-and-sdk-modes.md` and Owner decision 2026-11-24-3 | Sources of the new rules |
| Delivery Constraints, technology baseline | REWRITE | "graph-native declarative IR; single region with high availability" | "two authoring models on one engine, the graph-native declarative IR and the procedural code-first model of Principle IV; one writable region per cell, with high availability inside the region" | Owner decision 2026-11-24-2 |
| Delivery Constraints, Regions | ADD | — | A cell may declare standby regions and fail over active-passive by an operator's activation that raises a database-checked region epoch (032, ADR-0067); different cells may be writable in different regions; production only after the region drill report and the commit-latency benchmark; a mode that can lose acknowledged work says so, reports the loss and never claims a recovery point objective of zero | Owner decision 2026-11-24-2; keeps ADR-0067's evidence gate (Decision 1592 of feature 032) |
| Delivery Constraints, Admitted scope | ADD | — | Workflow as code; internal apps over declared operations, sessions no wider than the viewer's own authority, author code only in an isolated frame (027, ADR-0062); a curated hub whose only sources are the release-signed index and operator indexes with operator-pinned keys, items imported as drafts under the authored-content rules, nothing run in the server (021, 028, 047) | Owner decision 2026-11-24-2 |
| Delivery Constraints, out of scope | REWRITE | "multi-region active-active, exactly-once physical execution of external effects, a connector marketplace, WASM as a mandatory runtime, reimplementing Temporal or Restate, and automatic migration of arbitrary code" | "two regions accepting writes for the same cell …; exactly-once physical execution of external effects; an open marketplace …; a generic application builder with its own backend, database or server-side author code; wire, protocol or history compatibility with another engine …; WASM as a mandatory runtime; and automatic migration of arbitrary code" | Each removal is the owner's; each item kept or added is explained below |
| Delivery Constraints, Targets are not promises; Security-by-default | KEEP | — | Unchanged | — |
| Development Workflow and Quality Gates | KEEP | — | Unchanged | — |
| Governance | KEEP | — | Unchanged, including the owner-decision and approval clauses (see "The delegation and the constitution's own text") | — |
| Footer | REWRITE | `Version: 4.7.0`, last amended 2026-11-20 | `Version: 5.0.0`, last amended 2026-11-24, this report, the adoption sentence | — |

Every sentence of 4.7.0 that uses MUST or MUST NOT is present word for word in 5.0.0 (line breaks aside). Principle IV
adds three: the workflow function MUST NOT perform an external effect itself; a diverging replay MUST stop with a typed,
audited non-determinism error and commit none of its commands; the procedural model MUST NOT ship in an SDK before that
language's determinism study and the shared replay conformance scenarios pass. The kept sentence that the six SDKs MUST
NOT fork the server semantics now covers replay too.

## The procedural model: why these rules and no others

Workflow as code is Temporal's programming model: a function in the author's language whose control flow decides the
next step, replayed against the history after a crash. The constitution has to admit it without giving up what makes
this platform's runs explainable. Each sentence of the new paragraph of Principle IV closes one way it could go wrong:

- **One engine, one journal, one semantic contract.** Two engines would fork the server semantics, which Principle IV
  already forbids; the procedural model is a second way to write a version, not a second runtime on the server.
- **The function runs in an SDK worker, never in the control plane or the scheduler.** Principle V already forbids
  untrusted code inline in those processes; the sentence says where the code does run.
- **It proposes commands; the server accepts them under Principle I and commits them with the journal and the outbox
  in one transaction.** A workflow task's result is a worker result like any other: valid lease, fencing token, stale
  attempts refused. This is what keeps Principle I whole without rewriting it: the next task is still scheduled in the
  transaction of the transition, by the server.
- **The worker's memory is a cache that replay can discard and rebuild.** PostgreSQL stays the authority (Principle I);
  sticky execution (a worker keeping a run's state warm) is an optimisation, never a record.
- **No external effect in the function itself; effects are activities under Principle III.** Code outside an activity
  runs again on every replay. Without this sentence a payment written in the workflow body would be repeated by every
  recovery, and Principle III's idempotency and reconciliation rules would not apply to it.
- **A diverging replay stops with a typed, audited error and commits nothing.** The alternative, accepting the commands
  of a replay that disagrees with the journal, applies a transition the history does not explain. Whether the run then
  waits for a fixed build or fails is a policy that design 044 declares; it may never be a silent continuation.
- **A code change that running histories depend on enters only through an explicit version marker or a new version.**
  This is the procedural form of "no automatic migration of a running workflow": the author marks the change, the
  platform does not guess.
- **The model ships in an SDK only after that language's determinism study and the shared replay conformance pass.**
  Owner decision 2026-11-24-3 puts the study first; Principle VIII forbids a guarantee that is not proven. A language
  whose study fails does not get the model, rather than getting a weaker one.

Design 044 decides everything else (the command set, the history format, sticky queues, version markers, the
non-determinism policy, the replay tests) and writes the record that supersedes ADR-0003 in part.

## Regions: why active-active inside a cell stays out

The owner left this to the amendment. Three readings were weighed.

1. **Admit multi-region active-active.** Two regions accept writes for the same runs and reconcile afterwards.
2. **Admit active-passive per cell only, and say nothing about the fleet.**
3. **Admit active-passive per cell, keep two writable regions for the same cell out, and say that different cells may
   be writable in different regions.** (Chosen.)

Why not 1. Principle I requires a transition, the next tasks, the timers and the outbox to commit in one transaction,
and a transition applied twice or lost after commit cannot be repaired. Run state, quotas, permits, approvals, receipts
and leases are mutable rows, not a mergeable log. Two writers for one cell need either a consensus commit across
regions on every transition (a cross-region round trip in every commit, which the commit-latency benchmark of 032
measures for the synchronous standby already) or conflict resolution after the fact, which accepts a split brain by
design (ADR-0067, alternative 2). Neither is a gap against the competitors: in Temporal's multi-cluster replication a
namespace is active in one cluster at a time and a split brain is resolved after the fact by failover versions, and
Windmill runs one database per installation. A per-cell epoch fence that refuses rather than merges is the stronger
guarantee, and it is what 032 specifies.

Why 3 over 2. The business reason for active-active is usually locality and blast radius, not two writers for the same
row. Cells give both: a tenant has a home cell (032, owner question 1562), and the cells of a fleet can be writable in
different regions, so the platform serves Europe from a European cell and America from an American one while each cell
keeps one authority. Saying so in the constitution prevents a later reader from taking "one writable region" for "one
region for the whole fleet".

What the constitution now requires of failover, beyond 4.7.0: the evidence gate that ADR-0067 and Decision 1592 put on
the amendment itself moves into the text ("offered for production only after its region drill report and its
commit-latency benchmark exist"), because the owner chose to amend before the evidence exists. And the asynchronous mode
and a forced activation, which can lose acknowledged work, must say so before they are chosen, count and report the loss
afterwards, and never claim a recovery point objective of zero. That keeps Principle VIII (a guarantee that is not proven
is not faked) and "Targets are not promises" true for the new topology, and it is why Principle I needs no change: in
those modes the platform does not hide the loss, it measures it.

## The other items of the out-of-scope list

- **Reimplementing Temporal or Restate** leaves, as the owner decided. What the old item protected is kept by a narrower
  one: **wire, protocol or history compatibility with another engine** (running Temporal or Restate SDKs against this
  server, importing their histories). The platform adopts Temporal's programming model, not its protocol. A
  compatibility layer would bind the server to another engine's history format, failover versions and error model,
  which is a fork of the server semantics that Principle IV forbids, and it would make every change of theirs a change
  of ours. Nothing in release 0.3.0 asks for it (no feature or design mentions importing a Temporal or Windmill
  history); migration is served by the SDKs' own procedural model and by the benchmark of design 051. Decided by the
  coordinating session for architectural advantage; delegate (Owner decision 2026-11-24-4) if contested.
- **A connector marketplace** becomes **an open marketplace (publishing by tenants or third parties, unsigned or
  unpinned sources, ratings, payments)**. Curation is a property of the source, as feature 028 already designed it
  (Decision 1434, owner question 1402): the release-signed index and operator indexes with operator-pinned keys. The
  admission also says that hub items are imported as drafts under the authored-content rules and that the hub runs
  nothing in the server, so a hub item can never be a way around Principle V's isolation or the publication review.
- **A generic application builder with its own backend, database or server-side author code** is new in the
  constitution; it was a non-goal of `docs/architecture/overview.md` only. Feature 027's bounded builder is admitted and
  the generic one is kept out, as owner question 1360 recommended: an app backend running author code in the server is
  what Principle V forbids, and a database per app is a second authority beside PostgreSQL.
- **The viewer's own authority** is written into the admission of internal apps because it is the security property
  that makes an app safe to share (ADR-0062). A delegated mode, in which an app acts with its publisher's authority
  (Windmill's default; owner question 1361, recommended "no"), would widen authority through a page; the constitution
  says it needs its own ADR and an amendment, so that it cannot arrive as an implementation detail.
- **Exactly-once physical execution of external effects, WASM as a mandatory runtime and automatic migration of
  arbitrary code** stay, unchanged in meaning: the procedural model makes the first one more important, not less, and
  converting arbitrary code into the IR is still not promised.

## What stays and why

- **PostgreSQL is the authority and the broker is a hint** (Principles I and VII, byte-identical). The procedural
  model's commands commit in the server's transaction; a standby region's epoch is a database row checked by the
  database.
- **No exactly-once for external effects** (Principle III, byte-identical), now also enforced on the workflow function.
- **Untrusted code is isolated** (Principle V, byte-identical). The workflow function runs in the author's SDK worker;
  internal apps run author code only in an isolated frame; the hub runs nothing in the server.
- **Principle VIII** (byte-identical) is what the determinism study and the region drill answer to.

## The delegation and the constitution's own text

The constitution says that product, scope, budget, release and security-exception decisions belong to the owner and are
recorded in `docs/owner-decisions.md`, and that a review finding the owner accepts in writing there is not open. It does
not say that the owner must act in person. Owner decision 2026-11-24-4 is itself a decision of the owner, recorded there,
and each act taken under it is recorded there as a row that cites it; the same text carried the earlier delegations
(version 4.0.0 on "Decida por mim", version 4.6.0 under Owner decision 2026-11-20-6) without being changed. So the text
does not contradict the delegation, and this amendment leaves the Owner decisions and Approval clauses as they are. A
standing delegation clause was considered and rejected: the delegation is scoped to release 0.3.0, and writing it into
the constitution would make a temporary arrangement permanent. One clause does need the owner's own hands and is not
touched by the delegation: the EGM pin, which requires that the owner has read the diff of the plugin.

## Principle hashes

Previous-source: 5459c214aa5436caa2cb484907a23eb9adcf52a1

The line above is the git blob of `docs/constitution.md` at 4.7.0. A principle block is hashed as in the 4.0.0 report.

| Principle | Disposition | sha256 (previous) | sha256 (this version) |
|---|---|---|---|
| I | KEEP | 32d2ec982a4ae0995a472628184c4762ea740f9081886bb4c78fa461237a5a21 | 32d2ec982a4ae0995a472628184c4762ea740f9081886bb4c78fa461237a5a21 |
| II | KEEP | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 |
| III | KEEP | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 |
| IV | REWRITE | ff10043d76dfd25ac3b2db1ea27b943804c52d4d46c83838e6908cdaccc32f7b | 105a0916424dc0627c7bfe216bc7eecdc16aa392a96e3849df65bcb1e194254c |
| V | KEEP | b816cb76163ef1d8cc8579bb3e326f59c774b077131029cfd64fa45d470f38f9 | b816cb76163ef1d8cc8579bb3e326f59c774b077131029cfd64fa45d470f38f9 |
| VI | KEEP | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc |
| VII | KEEP | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 |
| VIII | KEEP | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 |

## Dependents updated in this change

- `docs/architecture/overview.md`: the section "Explicit scope left for later releases" becomes "Explicit scope left
  out", says what 0.3.0 admits and lists what stays out, in the constitution's words.
- `docs/architecture/workflow-ir-and-sdk-modes.md`: the procedural row is no longer "later phase"; the paragraph under
  the table states the Principle IV rules and the determinism study.
- `docs/architecture/execution-semantics.md`: the code-orchestrated mode is "release 0.3.0, design 044, not built".
- `sdks/SPEC.md`: a dated note under "An SDK is not a workflow runtime": the procedural model will change that, design
  044 amends the section, and the server keeps the journal and commits every transition.
- `docs/adr/ADR-0003-declarative-ir.md` and `docs/adr/ADR-0009-single-region-first.md`: a dated note each; both stay
  accepted until the records that supersede them in part (design 044's, ADR-0067) are accepted on their evidence.
- `docs/adr/ADR-0067-…`: status line and the section "Constitution amendment required (stated, not applied)", which
  becomes "applied in 5.0.0, ahead of the evidence", with the evidence gate now in the constitution; the index row in
  `docs/adr/README.md`.
- `docs/adr/ADR-0062-…`: a note that owner question 1360 is answered; acceptance of the record is still open.
- Feature 032 (`spec.md`, `research.md`, `plan.md`, `tasks.md`): owner question 1560 marked answered by Owner decision
  2026-11-24-2; Decision 1583 superseded in part; the Delivery Constraints row of the Constitution Check reads Pass;
  tasks 032-001 and 032-063 annotated, not ticked (owner questions 1561 and 1562 and the evidence are still open).
- Feature 027 (`spec.md`, `research.md`, `plan.md`, `tasks.md`): owner question 1360 answered as recommended, gap 1360
  closed; task 027-001 annotated, not ticked (owner questions 1361 and 1362 and ADR-0062 remain).
- Feature 028 (`spec.md`): the reason for curation now cites the admission; replay concepts belong to design 044.
- Feature 025 (`spec.md`): its out-of-scope marketplace line cites the 5.0.0 wording.
- Specifications 001 to 012: a dated note under each "V1 single-region HA" and "Reimplementing Temporal/Restate"
  assumption line (24 notes), as Decision 1583 asked: annotated, not rewritten, because those features' scope does not
  change.
- `SPECIFICATION-INDEX.md` and `docs/roadmap.md`: the gates that 5.0.0 settles are marked settled; the roadmap rows of
  027 and 032 and the parity paragraph follow; the 0.3.0 paragraph links this report.
- `docs/runbooks/disaster-recovery.md`: the multi-region case is 032's, admitted and not built.
- `docs/governance/README.md`: the row of this report.

Not changed, on purpose:

- `AGENTS.md`, `CLAUDE.md` and `README.md` repeat none of the changed clauses.
- `docs/owner-decisions.md`: the rows of 2026-11-24 already describe this amendment, and pull request #274 edits the same
  lines; the status of Owner decision 2026-11-24-2 becomes "Decided; amended in 5.0.0" after both merge (follow-up).
- The approved design docs 038, 042 and 043 say "single region" in their Constitution Check rows for the Delivery
  Constraints. They remain true under 5.0.0 (each cell has one writable region and those designs add no region), and an
  approved design doc returns to draft if its text changes, so they are left as they are.
- `docs/roadmap-assessment-2026-11-11.md` is a dated assessment; it records what was true that day.

## Follow-ups that are not this change's

- **Retention in feature 020 still needs an amendment of Principle I** (ADR-0054, gap 1101): removing whole journals of
  terminal runs changes an obligation of Principle I, and Owner decision 2026-11-24-2 does not name it. It is a 5.1.0
  (MINOR, as 020 proposed) or a MAJOR amendment of its own, with its own review, before 020's user stories 08 and 09
  start. Recommended: the coordinating session opens it under Owner decision 2026-11-24-4 when 020 reaches those
  stories, because it changes a durability obligation and deserves a review that is not shared with scope changes.
- **Design 044** writes the procedural model, its determinism study per language, its replay conformance scenarios, the
  record that supersedes ADR-0003 in part, and the amendment of `sdks/SPEC.md` and `docs/architecture/execution-semantics.md`.
- **Feature 032** keeps task 032-063: ADR-0067 `Accepted`, ADR-0009's "Superseded in part" line and the first
  production statement of cross-region failover, all on the region drill report and the commit-latency benchmark.
- **Feature 027**: owner questions 1361 and 1362 and the acceptance of ADR-0062, under Owner decision 2026-11-24-4.
- **Design 047**: the integration hub's sources, signatures and import path follow the admission (the release-signed
  index and operator-pinned indexes, drafts, nothing run in the server).
- **Owner decisions file**: the status of Owner decision 2026-11-24-2 after this pull request and #274 merge.

## Review

The read-only `architecture-reviewer`, `security-reviewer` and `correctness-reviewer` read the diff of this pull request
against Owner decisions 2026-11-24-1 to 2026-11-24-4. Their findings and what was done with each are below.

(Filled in after the reviews.)

## Risks and rollback

- The procedural model is admitted before its design exists. The constitution's rules bound what design 044 may decide;
  if the determinism study shows a rule cannot be met in a language, that language does not get the model, and the rule
  is not weakened without a new amendment.
- The region topology is admitted before its evidence exists. The "Regions" bullet keeps production use behind the
  drill report and the benchmark, so an installation cannot claim cross-region failover on the strength of this text.
- Rollback is a forward amendment (6.0.0) that restores the 4.7.0 Delivery Constraints and Principle IV; nothing in the
  code depends on this text yet.
