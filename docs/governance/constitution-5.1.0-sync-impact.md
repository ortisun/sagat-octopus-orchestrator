# Constitution 5.1.0 — Sync Impact Report

This record accompanies the pull request that amends the constitution on four points that the design agents of release
0.3.0 found. **Status: PROPOSED**; it is adopted by merging the pull request that carries it, on Owner decision
2026-11-25-3 ("Aprovo os quatro"), recorded in [`docs/owner-decisions.md`](../owner-decisions.md):

1. Principle I admits two audited ways to delete journal rows, only after a verified archive: the per-run purge of
   feature 020 under a namespace retention policy, and the retirement of a whole month partition of design 049 once
   every run in it is purged.
2. Principle V admits a third declared set: an executor may receive the settings of a declared resource and its current
   access token, never a refresh token, client secret or signing key (feature 021, design 047).
3. A throughput regression gate joins the CI gates (design 052).
4. The 5.0.0 text is reconciled with design 044 (a deferred terminal command, the wording of a run's pin, the move
   clause; gaps 044-021, 044-024, 044-025).

The same decision lets the amendment merge without returning to the owner unless a reviewer finds a Blocker. Findings
are settled as "delegate (Owner decision 2026-11-24-4)", within what that decision delegates. Amending the constitution
and security exceptions stay with the owner: a finding that would widen the text beyond the owner's words, or touch a
security exception, is fixed toward the owner's words or put to the owner. The owner may overrule any of it by a later
amendment.

## Scope and why MINOR

Principles I, IV and V and the CI gates bullet of the Development Workflow change. Principles II, III, VI, VII and VIII,
the Delivery Constraints and Governance do not.

**MINOR departs from this repository's precedents, and the owner chose it.** Two earlier amendments set the test:

- Version 3.0.0 classed the addition of the executor-read sets as a relaxation and bumped MAJOR.
- Version 5.0.0 chose MAJOR because "a reader of 4.7.0 could rely on" a statement that stopped being true.

By that test 5.1.0 relaxes obligations a reader of 5.0.0 could rely on:

- **Principle I.** "The journal is append-only" and "no other component holds the only copy of a transition" held for
  every run; now they hold for every run except a purged one.
- **Principle IV.** A completion that ended the run used to be refused while an accepted event was unseen; now its
  terminal command is deferred and the rest commits.
- **Principle V.** The executor-read exception has a third set.

The owner named the version "5.1.0", and there is a reason that holds: **every relaxation is opt-in, and no existing
deployment or run changes until someone acts.**

- Nothing is purged without a namespace retention policy that a person writes, and nothing is retired without the
  operator's own credential.
- Nothing reaches an executor under the third set unless a step declares a resource, a feature that does not exist yet.
- The deferral and the move wording concern workflow as code, which is not built.

So a reader who relied on 5.0.0 is still right about every system that does not opt in. That is the line this report
draws between MINOR and MAJOR here. A reader who prefers the 3.0.0 test would call it 6.0.0. The text would be the same,
and `scripts/check-design-docs.py` checks only that the version rises.

## Complete clause impact

| Unit of 5.0.0 | Disposition | Old | New | Why |
|---|---|---|---|---|
| Principle I, first paragraph | KEEP | — | Byte-identical | — |
| Principle I, deletion paragraphs | ADD | — | Two audited paths after a verified archive, each with its preconditions, its mechanism and its audit record; what stays forbidden; the archive as the only copy, how it may be deleted and what a customer store means; the exceptions named | Owner decision 2026-11-25-3, point 1 |
| Principle I, rationale | REWRITE | ADR-0002, semantics, rule 3 | adds why history may be removed and the sources ADR-0054 and design 049 | Sources of the new paragraphs |
| Principle IV, "A completion is one transaction" | REWRITE (last sentence) | "A completion that ends the run is refused while an accepted signal, update or cancellation is still unseen by the function." | "The command that ends the run is not applied while … is still unseen by the function: it is deferred, and the deferral journaled, the rest of the completion commits all or nothing, and the function sees those events in its next activation." | Gap 044-024 |
| Principle IV, "A pin changes only by a verified move" | REWRITE (three sentences) | "… and the journal has not advanced past the activation that was verified …"; markers "so that a verified move can succeed" | "… and the run has not completed an activation past the one that was verified …; events committed after that activation … are delivered in the next activation, on the target code"; "The version a run started on stays its origin; its pin names the version its workflow tasks run"; markers' code "reaches a running run only through such a move" | Gaps 044-025 and 044-021 |
| Principle IV, every other sentence | KEEP | — | Unchanged | — |
| Principle V, "limited to two declared sets" | REWRITE | two | three | Owner decision 2026-11-25-3, point 2 |
| Principle V, third bullet | ADD | — | A declared resource's non-secret settings and, for an OAuth credential, only its current access token; never a refresh token, client secret or private or signing key; a slot's secret reaches an executor only as a declared secret of the first set; the authorities that must agree; how each executor kind is bound; a workflow task is not such an executor | Same, in the owner's words; design 047, Technical Plan 12, narrowed (see Point 2) |
| Principle V, "Both paths MUST validate …" | REWRITE | "Both paths"; "Managed database delivery MUST also verify …" | "Every path"; "Managed delivery (database and integration runner) MUST also verify …" | The third set is bound by the same checks |
| Principle V, every other sentence | KEEP | — | Unchanged | — |
| Development Workflow, CI gates | REWRITE (one clause added) | "… the suite that budgets what they cost the database, and, …" | "… the suite that budgets what they cost the database and, once design 052 has built it, the throughput regression gate (…), and, …" | Owner decision 2026-11-25-3, point 3 |
| Every other unit | KEEP | — | Unchanged | — |
| Footer | REWRITE | `Version: 5.0.0`, last amended 2026-11-24 | `Version: 5.1.0`, last amended 2026-11-26, this report, the adoption sentence | — |

**How the MUST clauses were compared.** White space was collapsed and the text split after every full stop, semicolon
and colon. Of the 39 clauses of 5.0.0 that use MUST or MUST NOT, 36 are present word for word in 5.1.0. The other three
are REWRITE rows above, each wider than before:

- "Both paths MUST validate …" became "Every path MUST validate …".
- "Managed database delivery MUST also verify …" became "Managed delivery (database and integration runner) MUST also
  verify …".
- The CI gates clause gained the gate.

Principle V adds one MUST: the authorities that must authorize the third set's disclosure. No obligation is dropped.

## Point 1: the two deletion paths

**What each path requires.** The text writes, for each path, its preconditions, its mechanism, its audit record and what
stays forbidden.

**Per-run purge** (feature 020; ADR-0054 decisions 1 to 5; Requirements 020-031 and 020-034).

Preconditions:
- an explicit namespace retention policy; without one every run is kept, which is owner question 1080's default and
  the owner's "under a namespace retention policy";
- the run is past that policy's retention;
- the run is terminal, with its whole tree terminal and, for a chain, the chain final;
- its effects are settled: no unresolved unknown outcome or review, and a run that ended `PARTIAL` or
  `COMPENSATION_FAILED` only after an authorized, audited human resolution under Principle III, as ADR-0054 decision 2
  and Requirement 020-031 say;
- "the further conditions its design sets": sink application, and design 049's leaf-certificate exclusion;
- an archive written and read back with matching size and SHA-256 before anything is deleted.

Mechanism and audit: one privileged database function works in bounded transactions, as feature 020 designs it.
- The first transaction takes the run's lock, checks eligibility again, and turns the run row into a stub that points to
  the archive and accepts no further write.
- The others remove the run's journal and content, or, in design 049's mark-only form, leave them in their month's
  partition for the second path to drop.
- An interrupted purge resumes until nothing of the journal is left.
- The value-free audit record commits with the last transaction.

**Partition retirement** (design 049, Requirements 049-009 and 049-014, and its retirement procedure).
- Who: the operator, with a credential no platform process holds.
- When: only after every run of the month has been purged on the first path, in full or in its mark-only form, and
  after a check that the detached partitions hold no row of a run that is not purged.
- Audit: the drop commits with the retirement's audit record and one per affected tenant. Every step is recorded, so a
  crash resumes from where it stopped.

This path deletes nothing that the first path has not already turned into a stub: it removes, in bulk, the rows of runs
whose purge is already recorded.

**What stays forbidden:**
- deleting or rewriting an entry of a run that can still change;
- deleting without a verified archive;
- leaving a journal half removed once its purge has started (it is resumed to the end);
- any other path. The runtime role holds no `DELETE` or `TRUNCATE` on runs, steps, the journal or payloads.

**The archive as the only copy.** After a purge, the archive is the only copy of that run's transitions, outside the
database's transactions and backups. The text names the purged run as the one exception to Principle I's first and
third sentences, instead of weakening those sentences, so they still protect every run that can change. It also
governs the archive that now carries that duty:
- it is deleted only by an audited act under its namespace's policy;
- an archive in a customer's own store is as durable as that store, which the platform does not promise (ADR-0054
  allows a customer destination by secret reference).

**What the owner's words leave out, and why this text follows them.** The owner approved deletion "only after a
verified archive" and the purge "under a namespace retention policy". Two things feature 020 and design 049 propose fall
outside those words, and the text does not admit them:

- **`archive = NONE`** (Requirement 020-030, ADR-0054 decision 1, design 049's "or its policy is `archive: NONE`"):
  deletion with no copy. Out.
- **Erasure** (Requirement 020-037, the LGPD counterpart of retention): a purge at a subject's request, before the
  policy's retention, with the archive optional. Out as written.

Erasure matters for the business: a subject's request has a legal deadline, and a namespace whose policy keeps runs for
a year could not answer it. Feature 020's Gap 1101 records the recommendation to the owner, stated plainly: admit
erasure as an audited request by a person holding `retention:manage`, for a terminal, settled run, after a verified
archive. The erasure then deletes that archive, by the audited act the text requires, **so it ends with no copy of the
run at all**, which is what an erasure is for. It is the owner's decision, because "only after a verified archive" was
meant to avoid exactly that end. The coordinating session cannot add it: Owner decision 2026-11-24-4 does not delegate
amending the constitution.

**What the text writes in that feature 020 left to the owner.**
- "Without a policy every run is kept" answers owner question 1080 with its default; the owner's "under a namespace
  retention policy" covers it.
- "Turns the run row into a stub" fixes the shape of owner question 1082, but not its details (which fields stay, what
  is cleared); those stay feature 020's, under that question.

## Point 2: the third declared set

**In the owner's words.** The owner's row says the third set is "the settings of a declared resource and its current
access token, never a refresh token, client secret or signing key". The text says exactly that: the non-secret settings
of the pinned revision and, for an OAuth credential, only its current access token, never a refresh token, a client
secret or a private or signing key.

The first draft followed design 047's Technical Plan 12 further, and also released "the values its slots bind …: their
secrets" (an API key, an SMTP password). The architecture review found that this widens a security exception beyond the
owner's words, which only the owner can do (Blocker 1). The text now says instead: "A secret that one of its slots binds
reaches an executor only as a declared secret of the first set". Feature 021's integration runner gets static
credentials the way every step gets its secrets: declared by the step's immutable task definition, all or nothing,
under the first set's authorities.

What that costs feature 021 is recorded as a follow-up. Its runner's binding for those secrets is ordinary delivery
(the bearer capability with the current caller's authorities), not managed delivery's session binding. Feature 021 can
ask the owner for the wider set if that matters.

**The rest of design 047's text stays.**
- The authorities that must agree: current membership, worker or activity binding and pool policy; the publisher's and
  initiator's permission to use the resource and to cause effects through it; and the secret-access rules of its slots.
- How each executor kind is bound: a managed integration runner like the database runner, an ordinary worker or a
  sandbox runner like ordinary delivery.
- "Every path MUST validate …", and managed delivery's check of the recorded lease-owning executor subject and live
  session.

One sentence is added: "A workflow task of Principle IV is not such an executor". Principle IV says a workflow task
carries no secret value, and the third set must not become a way around that.

## Point 3: the throughput gate

When a pull request changes the code that runs a lease or a completion, it runs the throughput regression gate as well
as the budget suite. Design 052 adds the gate to `scripts/select-test-targets.py`'s output, with design 038's budget
target and the same files (Requirement 052-030; Task 052-14). The gate compares the base and the head of the pull
request on the same scenarios, alternated, on the standard hosted runner. It fails on a slowdown larger than the minimum
detectable effect that its recorded pilot measured on that runner, by the rule design 052 states: six alternated pairs,
the head's median slower by more than that effect, and slower in at least five pairs.

Two choices:

- **"Once design 052 has built it."** The job does not exist yet. A MUST that no CI job can satisfy would put every
  hot-path pull request out of compliance until it exists, so the obligation starts with the job.
- **The thresholds stay in the design.** The constitution names what the gate compares and on what. The numbers belong
  to design 052, which measures them, so recalibrating after a new pilot needs no amendment. The owner's "Nada pago"
  (no paid runners, Owner decision 2026-11-25-1) is why the text says "the standard hosted runner".

## Point 4: design 044

- **Gap 044-024, a deferred terminal command.** 5.0.0 said a completion that ends the run "is refused" while an
  accepted signal, update or cancellation is unseen. Design 044 found that a refused completion repeats for ever when
  the cause is an obligation the function cannot settle, so it defers the terminal command and commits the rest. The
  new text says that, and that the deferral is journaled, as ADR-0104 decision 4 and Requirement 044-031 record it in
  `workflow_task.completed`. The obligation of 5.0.0 is kept: a run never ends over an event its sender was told was
  accepted. Only the mechanism changes.
- **Gap 044-025, the move clause.** 5.0.0 applied a move only while "the journal has not advanced past the activation
  that was verified". Events such as a signal can be journaled after that activation without the function having seen
  them, and they do not change the commands the replay checked. The new text applies the move while "the run has not
  completed an activation past the one that was verified", and says those events are delivered in the next activation,
  on the target code. What a later completion would invalidate is still refused.
- **Gap 044-021, the pin.** 5.0.0 already said "a run's pin names exactly one version at any moment". The new text adds
  that the version a run started on stays its origin, and that a later version's code reaches a running run only
  through a verified move, as the gap asked.

## Principle hashes

Previous-source: 921842ef20d89a9ca83b1e8f030e842374b17fdb

The line above is the git blob of `docs/constitution.md` at 5.0.0. A principle block is hashed as in the 4.0.0 report.

| Principle | Disposition | sha256 (previous) | sha256 (this version) |
|---|---|---|---|
| I | REWRITE | 32d2ec982a4ae0995a472628184c4762ea740f9081886bb4c78fa461237a5a21 | 30c8e88e107935e21bd861338b5e8216f467d083356550f90f39a0b94ef97f1f |
| II | KEEP | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 |
| III | KEEP | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 |
| IV | REWRITE | 2bf35e4b0a41b41d2bd7a98dd2e2825a18ea635bc8eed63ed34b6c98e8c6e5eb | af1098424e5a4dc3d89329b97d508baf1b2044ae6b71a45c42398adbc3a705c1 |
| V | REWRITE | b816cb76163ef1d8cc8579bb3e326f59c774b077131029cfd64fa45d470f38f9 | c40f55b6678bcb4e34e13e72aace0890d2783d6114b49b983feb178030e8ceb4 |
| VI | KEEP | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc |
| VII | KEEP | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 |
| VIII | KEEP | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 |

## What enforces each new rule

| Rule | Check today | Check to add, and where |
|---|---|---|
| The runtime role holds no `DELETE` or `TRUNCATE` on runs, steps, the journal or payloads | `scripts/check-architecture.py` (new in this change): it refuses a migration that grants `DELETE`, `TRUNCATE` or `ALL` on those tables (named, through `ALL TABLES` or through default privileges) to `sagat_app` or `PUBLIC`; its tests are in `scripts/tests/test_check_architecture.py`. At run time, `crates/infra-postgres/src/privilege.rs` refuses `DELETE` on `run_events` and `payloads` only, not on `runs` or `steps` | feature 020: the privilege sweep with and without a purge authorization (ADR-0054, Enforced by), which also covers a grant made outside a migration |
| Deletion only after a verified archive; only the two paths | none (neither path is built) | feature 020: the purge refuses an unverified archive (fault tests with a corrupted and a missing object); design 049: Task 049-14's crash and race tests |
| The third set never releases a refresh token, client secret or private or signing key, nor a slot's secret | none (design 047 not built); `scripts/check-architecture.py` already keeps the `"grant_type"` literal inside `crates/infra-oauth` | design 047: the broker's canary tests across the protocol, logs and audit |
| A workflow task is not an executor of the third set | none | design 044: the workflow task carries no secret (Requirement 044-003), and design 047's resolver refuses it |
| The throughput gate | none (Task 052-14) | design 052: the `throughput-gate` job, which fails on the hand mutation and passes on its revert |
| A deferred terminal command; the move clause | none (design 044 not built) | design 044's tests of `UNHANDLED_EVENTS` and of the move (ADR-0105) |

`docs/engineering/architecture-principles.md` gains the row for the first rule.

## Dependents updated in this change

- `docs/architecture/execution-semantics.md`: the persistence line names the two deletion paths, not built.
- `docs/architecture/security-and-sandbox.md`: the disclosure boundaries name the third set and "every path".
- `docs/test-strategy.md`: the hot-path selection names the throughput gate, not built.
- `docs/engineering/architecture-principles.md`: the row of the deletion rule and its check.
- `crates/infra-postgres/AGENTS.md`: the two paths and the grant rule, for the crate that will host the purge.
- `scripts/check-architecture.py` and `scripts/tests/test_check_architecture.py`: the grant check.
- `docs/adr/ADR-0054-…`: a dated note on its "Required amendment of principle I" and on decisions 1 and 3 and rejected
  option 3. It records three differences from the proposal: two paths, no `archive = NONE`, no erasure outside the
  policy.
- Feature 020:
  - `spec.md`: the overview note; Requirement 020-030 marks `NONE` as not admitted; Requirement 020-037 is gated on Gap
    1101.
  - `data-model.md`: the `archive = NONE` arrow is marked.
  - `plan.md`: the Constitution Check reads Pass for the purge after a verified archive.
  - `research.md`: Gap 1101 is closed for the purge and open for `archive = NONE` and erasure, with the recommendation.
  - `checklists/requirements.md` and `tasks.md`: Task 020-061 is reworded and annotated, to be ticked with this pull
    request once it merges.
- `SPECIFICATION-INDEX.md` and `docs/roadmap.md`: the retention gate is settled for the purge and still open for the
  rest; the roadmap names 5.1.0.
- `docs/governance/README.md`: the row of this report.

Not changed, on purpose:

- Designs 044, 047, 049 and 052 and feature 021's register live on other branches, which their agents own
  (`feature/044-workflow-as-code`, `docs/windmill-parity-readiness`, `docs/scale-resilience-readiness`). Their changes
  are follow-ups.
- The 5.0.0 report keeps its follow-up "Retention in feature 020 still needs an amendment of Principle I": it records
  what was true on 2026-11-24.

## Follow-ups that are not this change's

- **The owner: erasure and `archive = NONE`** (feature 020, Gap 1101). The recommendation is above. Until the owner
  answers, feature 020 builds the purge under a policy with a required archive, and its erasure story (user story 09)
  waits.
- **The owner, if feature 021 asks: slot secrets in the third set.** They are out; feature 021 uses the first set.
- **Feature 021**: Decision 021-022 is answered in part. The settings and an OAuth access token come through the third
  set; a slot's secret comes through the first set, declared by the step, with ordinary delivery's binding. Its
  capability resolution must follow that.
- **Design 047**: Technical Plan 12's condition is met in the owner's narrower words. Its text should drop "the values
  its slots bind … their secrets", use "private or signing key", and its Constitution Check row for Principle V reads
  Pass once this merges.
- **Design 049**:
  - drop "or its policy is `archive: NONE`" from Requirement 049-009;
  - replace "pending (Owner decision 2026-11-24-511)" in its overview, its "Owner decisions needed" and its Constitution
    Check;
  - change the `Owner-gate:` lines of Tasks 049-11 and 049-14 to cite Owner decision 2026-11-25-3 once this merges.

  Task 049-18 is done by this text.
- **Design 052**: replace "pending (Owner decision 2026-11-24-512)" in Open Question 2, its owner-decision list and its
  Constitution Check, and change Task 052-14's `Owner-gate:` line to cite Owner decision 2026-11-25-3. Task 052-15 is
  done by this text.
- **Design 044**: gaps 044-021, 044-024 and 044-025 close when this merges, and its Constitution Check for principles I
  and IV can drop the conditions they caused.

## Review

The read-only `architecture-reviewer`, `security-reviewer` and `correctness-reviewer` read the diff of this pull request
one after the other, with the owner's row and no conclusion of the author's. Each finding is listed with what was done.

### Architecture review (one Blocker, six Major, eight minor, four Nit)

| Finding | Disposition |
|---|---|
| Blocker 1: the third set disclosed static slot secrets, beyond the owner's words and a security exception | Fixed toward the owner's words: settings and the current OAuth access token only; a slot's secret only as a declared secret of the first set |
| Major 1: the purge text was stricter than feature 020's chunked mechanism | Fixed: bounded transactions, the first locks and stubs, resumes to the end, audit with the last; "never part of a journal" became "a journal whose purge has started is removed in full" |
| Major 2: retirement depends on design 049's mark-only form, which the text did not admit | Fixed: the first path may leave the rows in their month's partition for the second to drop; retirement accepts runs purged "in full or in its mark-only form" |
| Major 3: `PARTIAL` and `COMPENSATION_FAILED` runs could never be purged | Fixed: after an authorized, audited human resolution under Principle III, as ADR-0054 and Requirement 020-031 say |
| Major 4: MINOR contradicts the precedents; two statements of the report were wrong | Fixed: the report states the departure, why it holds (every relaxation is opt-in), and that the 3.0.0 test would give 6.0.0; the wrong statements are gone. The version stays the owner's "5.1.0" |
| Major 5: nothing governs the archive that becomes the only copy | Fixed: the archive is deleted only by an audited act under its namespace's policy; a customer store's durability is not promised. The erasure recommendation now says plainly that it ends with no copy |
| Major 6: the no-`DELETE` rule had no check for `runs` and `steps`, and the report claimed one | Fixed: a check in `scripts/check-architecture.py` with tests seen failing first, the row in `architecture-principles.md`, and the enforcement table corrected |
| minor 1: feature 020's requirements still offered `archive = NONE` and optional-archive erasure | Fixed: Requirements 020-030 and 020-037, the data model, the plan's Constitution Check, the checklist and Task 020-061 are marked |
| minor 2: the CI clause says the selector decides, and the report said it would not select the gate | Fixed: design 052 adds the gate to the selector's output |
| minor 3: the exception covered only the first sentence of Principle I | Fixed: "the first and third sentences" |
| minor 4: ADR-0054's note covered one section | Fixed: decisions 1 and 3 and rejected option 3, citing ADR-0132 on its branch |
| minor 5: owner questions 1080 and 1082 answered by the text | Fixed: the report says what the text fixes (1080's default; the stub's shape) and what stays 1082's |
| minor 6: the adoption sentence cited the wrong authority | Fixed: Owner decision 2026-11-25-3 for merging without the owner, 2026-11-24-4 for the rest, security exceptions to the owner |
| minor 7: "a PATCH amendment removes the slot secrets" was wrong | Moot: the slot secrets are out |
| minor 8: stale lines in designs 049 and 052; `crates/infra-postgres/AGENTS.md` | Fixed: the follow-ups name the lines; the crate guide gains the rule |
| Nit: the deferral should be journaled | Fixed |
| Nit: line wrapping in the roadmap and feature 020 | Fixed in the roadmap; the notes in 020 are single dated lines, kept so that a reader sees them where the old text is |
| Nit: the eligibility list read as complete | Fixed: "under the further conditions its design sets" |
| Nit: the fitness table could cite the `"grant_type"` rule | Fixed |

### Security review

(Filled in after the review.)

### Correctness review

(Filled in after the review.)

## Risks and rollback

- Deleting history is irreversible. The text admits it only on two named paths, each after a verified archive. Both are
  designed but not built, and each ships with the fault tests named above.
- The third set widens what an executor can receive, by settings and an OAuth access token only. The existing two
  sets' checks bind it, and what the owner excluded is excluded by name.
- Rollback is a forward amendment that removes the deletion paragraphs, the third set and the gate. No code depends on
  this text, except the grant check, which a revert removes.
