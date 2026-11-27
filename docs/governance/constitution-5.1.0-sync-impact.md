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

Principles I, IV and V, the CI gates bullet of the Development Workflow and the footer change. Principles II, III, VI,
VII and VIII, the Delivery Constraints and the Governance clauses do not.

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

- Nothing is purged without a namespace retention policy and a verified archive that only a dedicated retention role
  can record, and nothing is retired without the operator's own credential.
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
| Development Workflow, CI gates | REWRITE (two clauses added) | "… the suite that budgets what they cost the database, and, …"; "… and of the packages that depend on those, and only those" | "… the suite that budgets what they cost the database and, once design 052 has built it, the throughput regression gate (…), and, …"; "…, and, on every pull request once design 052 has built it, that design's chart checker (…), and only those" (the checker is `scripts/check-chart-wake.py`; the constitution names no path that does not exist yet) | Owner decision 2026-11-25-3, point 3; the chart checker as a delegate reading (see Point 3) |
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

Who may purge: a change that creates a policy, shortens its retention, moves or re-credentials its archive's
destination, or narrows what its archive keeps applies only after a notice period announced to the namespace's
administrators. Without the notice, one person could point the archive at a store they control, or narrow it to
metadata, and purge the next day unseen (security review, Major 4; correctness review, Major 5). The function is executable only by a retention role whose credential no
process that holds the runtime credential or runs tenant code holds. Only that role's function records an archive as
verified and writes the stub and the purge's state, and a trigger refuses any other writer (correctness review, Majors 3
and 4). The runtime role keeps table-level `UPDATE` on `runs`, which would otherwise cover the stub's columns; retirement
judges "purged" by the retention role's records, not by a stub column the runtime role could forge. Feature 020's design let the
runtime role write the policy, the verified state and audit rows (ADR-0054 decision 1, Gap 1102). A holder of the
runtime credential could then have marked invented archives verified and erased every settled journal with no copy
anywhere (security review, Major 2). With the verified state in another role's hands, a forged policy can at worst make
a run eligible early, and its journal still exists in a real, verified archive.

Mechanism and audit: one privileged database function works in bounded transactions, as feature 020 designs it.
- The first transaction takes the run's lock and checks eligibility again. It checks that the archive covers the run's
  journal up to the last sequence it finds under the lock; otherwise it deletes nothing and the run is archived again,
  so an entry journaled after the export is never lost (security review, Major 3). It then turns the run row into a stub
  that carries the archive's location, size and SHA-256 and accepts no further write, and commits a value-free audit
  record with the policy revision and the archive's digest. So a purge that stops after a deletion chunk is never
  unaudited (security review, minor 3).
- The others remove the run's journal and content, or, in design 049's mark-only form, leave them in their month's
  partition for the second path to drop.
- The stub refuses every later append to the run's journal and every write but the purge's and the retirement's own.
  The coverage check is sound because of that: no entry can commit after the last sequence it found (correctness review,
  minors 2 and 3).
- An interrupted purge resumes until nothing of the journal is left. A run left in the mark-only form whose retirement
  does not complete is finished by this path (design 049's `CONTENT_PENDING_DROP`; correctness review, minor 1).
- A second audit record commits once the run's rows are gone.

**What an archive keeps.** The archive keeps every journal entry, and the payload values up to the policy's level
(`archiveValues`), never a secret value. The values above that level leave with no copy, and the policy says so. This
is feature 020's design (Requirement 020-030, ADR-0054 decision 1). It means that a policy that archives
`METADATA_ONLY` deletes payload values with no copy, which is close to what the owner excluded with "only after a
verified archive" (correctness review, Major 5). The text says it openly rather than hide it, and it is put to the
owner (follow-ups). The alternative, requiring every value in the archive, would put `pii` into object storage for every
namespace that wants retention at all, which feature 020 deliberately made a separate, granted choice (`UP_TO_PII`).

**Partition retirement** (design 049, Requirements 049-009 and 049-014, and its retirement procedure).
- Who: the operator, with a credential that only the operator's retirement command holds and that no process that
  serves requests, runs work or runs tenant code holds or can obtain. Design 049's opt-in scheduled job, with its own
  service account and the Secret only it mounts, is that command; the first draft's "no platform process" contradicted
  it (correctness review, minor 4). The command may run the mark-only purges with that credential. The credential is
  the database owner's, which could also drop a partition outside the command, so a partition that is missing without a
  completed retirement record is detected and alarmed (security review, minor 2).
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
- every read of it is checked against the size and SHA-256 that its stub carries before it is served or used, so a
  forged or altered archive is never served with the platform's authority (security review, Major 4);
- it is deleted only by an audited act under its namespace's policy;
- an archive in a customer's own store is as durable as that store, which the platform does not promise (ADR-0054
  allows a customer destination by secret reference);
- for a run that ended `PARTIAL` or `COMPENSATION_FAILED`, it is the step and history evidence Principle III retains.

The exceptions sentence also names the surviving-timeline rule of the Delivery Constraints (5.0.0), which already lets
a lossy failover lose acknowledged transitions (correctness review, minor 5).
  Principle III says terminal failure "retains state and step/history evidence"; after an audited resolution, the
  verified archive keeps that evidence, so Principle III stays byte-identical (security review, minor 4).

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

**How the text meets feature 020's own decisions.**
- "Without a policy every run is kept" agrees with owner question 1080 as feature 020 decided it on 2026-11-24 under
  the delegation (no default retention), and with the owner's "under a namespace retention policy".
- "Turns the run row into a stub" agrees with owner question 1082 as feature 020 decided it the same day (stubs kept);
  its details (which fields stay, what is cleared) stay feature 020's. ADR-0054 was accepted then too; its note records
  where the constitution now differs from it.

## Point 2: the third declared set

**In the owner's words, and tighter.** The owner's row says the third set is "the settings of a declared resource and
its current access token, never a refresh token, client secret or signing key". The text says that: the non-secret
settings of the pinned revision and, for an OAuth credential, only its current access token, never a refresh token, a
client secret or a private or signing key. The security review tightened it twice, toward the owner's intent:
- the access token is a secret value under Principle V, so it may not reach the journal, logs, audit or telemetry like
  any other (Major 5);
- a token with no expiry, such as a GitHub OAuth application's, is as durable as the refresh token the owner excluded,
  so one with no stated expiry (`expires_in` is optional in OAuth) is disclosed only to a managed integration runner
  (Major 5).

The first draft followed design 047's Technical Plan 12 further, and also released "the values its slots bind …: their
secrets" (an API key, an SMTP password). The architecture review found that this widens a security exception beyond the
owner's words, which only the owner can do (Blocker 1). The text now says instead: "A secret that one of its slots binds
reaches an executor only as a declared secret of the first set". Feature 021's integration runner gets static
credentials the way every step gets its secrets: declared by the step's immutable task definition, all or nothing,
under the first set's authorities.

Those first-set secrets go under the first set's checks and the resource's own authorities as well (the initiator's
right to use the resource and cause effects through it, the resource's state), so that a slot's secret never skips
what design 047 requires of every delivery (correctness review, minor 7). When they go to a managed integration
runner, they are bound like managed delivery: the recorded
lease-owning executor subject and live session too, so a long-lived API key is never bound more weakly than a one-hour
token (security review, minor 1). Feature 021 can ask the owner for the wider set if that matters.

**The rest of design 047's text stays.**
- The authorities that must agree: current membership, worker or activity binding and pool policy; the publisher's and
  initiator's permission to use the resource and to cause effects through it; and the secret-access rules of its slots.
- How each executor kind is bound: a managed integration runner like the database runner, an ordinary worker or a
  sandbox runner like ordinary delivery.
- "Every path MUST validate …", and managed delivery's check of the recorded lease-owning executor subject and live
  session.

One sentence is added: "A workflow task of Principle IV is not such an executor". Principle IV says a workflow task
carries no secret value, and the third set must not become a way around that.

## Point 3: the throughput gate, and the chart checker beside it

**The gate.** When a pull request changes the code that runs a lease or a completion, it runs the throughput regression
gate as well as the budget suite. The text follows Requirement 052-030 as design 052 was approved and merged:

- `scripts/select-test-targets.py` sets the gate's `throughput_gate` output on design 038's hot-path files and on the
  gate's own files, so the selector that the CI gates bullet names decides it;
- the merge base and the head are alternated on the standard hosted runner, both driven by the merge base's scenario
  and configuration, so that a pull request cannot edit what it is judged by (the security review's Nit);
- the gate fails on a slowdown larger than the minimum detectable effect that its recorded pilot measured on that
  runner, by the rule design 052 states (six alternated pairs, the head slower in all six and the median of the paired
  differences beyond the threshold; ten pairs if the pilot's power is too low);
- when the merge base's scenario cannot build or run against the head, the pull request passes as "not comparable"
  only with a pilot re-run on its own branch. The first draft said "each built by the side it measures", which the
  correctness review proposed (Major 6) before design 052 settled the case this way; the text now says what design 052
  does.

Two choices:

- **"Once design 052 has built it."** The job does not exist yet (Task 052-11). A MUST that no CI job can satisfy would
  put every hot-path pull request out of compliance until it exists, so the obligation starts with the job.
- **The thresholds stay in the design.** The constitution names what the gate compares and on what. The numbers belong
  to design 052, which measures them, so recalibrating after a new pilot needs no amendment. The owner's "Nada pago"
  (no paid runners, Owner decision 2026-11-25-1) is why the text says "the standard hosted runner".

**The chart checker: a delegate reading, reported to the owner.** Design 052 (Task 052-6, Requirement 052-054) adds
`scripts/check-chart-wake.py`, a static checker that renders the Helm chart with `helm template`, with the wake off,
on NATS and on Redis. It fails on a chart that opens the platform's broker channel without its credentials Secret, or
that opens broker egress from the control plane without the wake. It runs as a step of the `documentation` job on every
pull request, so that a change of the chart alone is checked, and it never skips. The CI gates bullet enumerates what a
pull request runs ("and only those"), so a check on every pull request needs the text.

The owner's row names "a throughput regression gate"; it does not name the chart checker. The coordinating session
reads the checker as part of design 052's gating that the owner approved in point 3. It is the same design's CI
obligation, it guards the channel through which the throughput remedies work, and it costs seconds on the free runner.
This reading is a delegate reading (Owner decision 2026-11-24-4), recorded here and reported to the owner, who can
strike it. Striking it would move the checker back to the `architecture` job and the full runs, where design 052 runs
it until this merges. Design 052's Task 052-10 checks that the text admits both, and records the merge commit.

## Point 4: design 044

- **Gap 044-024, a deferred terminal command.** 5.0.0 said a completion that ends the run "is refused" while an
  accepted signal, update or cancellation is unseen. Design 044 found that a refused completion repeats for ever when
  the cause is an obligation the function cannot settle, so it defers the terminal command and commits the rest. It
  defers for two causes, and the text names both: an accepted event the function has not seen, and an obligation the
  function cannot settle (an aborting saga scope, a step under review or with an unknown outcome; Requirement 044-031,
  ADR-0104 decision 4, cause `OUTSTANDING_OBLIGATIONS`). The first draft named only the first, which left the very case
  of the gap outside the text (correctness review, Major 1). The
  new text says that, and that the deferral is journaled, as ADR-0104 decision 4 and Requirement 044-031 record it in
  `workflow_task.completed`. The obligation of 5.0.0 is kept: a run never ends over an event its sender was told was
  accepted. Only the mechanism changes.
- **Gap 044-025, the move clause.** 5.0.0 applied a move only while "the journal has not advanced past the activation
  that was verified". Events such as a signal can be journaled after that activation without the function having seen
  them, and they do not change the commands the replay checked. The new text applies the move while "the run has not
  completed an activation past the one that was verified", and says those events are delivered in the next activation,
  on the target code. What a later completion would invalidate is still refused. The security review added that the
  target must admit every accepted signal, update and cancellation the function has not yet seen, so that a move never
  turns an event its sender was told was accepted into one the target refuses (minor 6). The correctness review added
  that the replay must reproduce any recorded deferral too, since a deferred command takes no command number and a
  target that dropped it would pass verification (minor 11).
- **Gap 044-021, the pin.** 5.0.0 already said "a run's pin names exactly one version at any moment". The new text adds
  that the version a run started on stays its origin, and that a later version's code reaches a running run only
  through a verified move, as the gap asked.

## Principle hashes

Previous-source: 921842ef20d89a9ca83b1e8f030e842374b17fdb

The line above is the git blob of `docs/constitution.md` at 5.0.0. A principle block is hashed as in the 4.0.0 report.

| Principle | Disposition | sha256 (previous) | sha256 (this version) |
|---|---|---|---|
| I | REWRITE | 32d2ec982a4ae0995a472628184c4762ea740f9081886bb4c78fa461237a5a21 | 6b1457b923af2a64c908e4439ebb9f2a65eabb0b03023637e09dba3cf6b9e2ae |
| II | KEEP | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 |
| III | KEEP | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 |
| IV | REWRITE | 2bf35e4b0a41b41d2bd7a98dd2e2825a18ea635bc8eed63ed34b6c98e8c6e5eb | 170a5d24a4cb6003401ead07afc9baf6ce7e8d44d2320f785532c98fd3f52bdc |
| V | REWRITE | b816cb76163ef1d8cc8579bb3e326f59c774b077131029cfd64fa45d470f38f9 | 9d6efc5532b3493214a073e0a60d4314dbf20df682da62c2a9d1f329d9a4f999 |
| VI | KEEP | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc |
| VII | KEEP | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 |
| VIII | KEEP | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 |

## What enforces each new rule

| Rule | Check today | Check to add, and where |
|---|---|---|
| The runtime role holds no `DELETE` or `TRUNCATE` on runs, steps, the journal or payloads | `scripts/check-architecture.py` (new in this change) reads every migration, up and down. It refuses a `GRANT` of `DELETE`, `TRUNCATE` or `ALL` on `runs`, `steps`, `run_events`, `payloads`, `run_payloads` or a monthly or legacy partition of them (named, through `ALL TABLES` or default privileges, with any `WITH` option or `GRANTED BY`) to `sagat_app` or `PUBLIC`; a role granted to `sagat_app` by `GRANT`, `CREATE ROLE … ROLE`/`IN ROLE` or `ALTER GROUP … ADD USER`; `OWNER TO sagat_app` and `REASSIGN OWNED … TO sagat_app`; and such a `GRANT` built for dynamic SQL, in one literal or through `format()` arguments. Block and line comments are skipped, and a last statement without its semicolon is read. It does not judge `GRANT EXECUTE` on a definer function that deletes journal rows, which is the purge's own pattern; the retention role's exclusivity (above) is checked by feature 020's tests. Tests are in `scripts/tests/test_check_architecture.py`. At run time, `crates/infra-postgres/src/privilege.rs` refuses `DELETE` on `run_events` and `payloads` only | the runtime check the security review asked for: `privilege.rs` extended to `DELETE` and `TRUNCATE` on `runs`, `steps` and every partition of the journal tables and `run_payloads` (through `pg_inherits`), which follows memberships and grants made outside migrations; it is feature 020's privilege sweep and needs a Rust build, so it is a follow-up |
| Only the retention role can purge, record an archive as verified and write the stub | none (feature 020 not built) | feature 020, Task 020-070: the definer function's `EXECUTE` granted only to the retention role, a trigger that refuses any other writer of the stub and purge state, and tests that the runtime role can do neither |
| Deletion only after a verified archive; only the two paths | none (neither path is built) | feature 020: the purge refuses an unverified archive (fault tests with a corrupted and a missing object); design 049: Task 049-14's crash and race tests |
| The third set never releases a refresh token, client secret or private or signing key, nor a slot's secret | none (design 047 not built); `scripts/check-architecture.py` already keeps the `"grant_type"` literal inside `crates/infra-oauth` | design 047: the broker's canary tests across the protocol, logs and audit |
| A workflow task is not an executor of the third set | none | design 044: the workflow task carries no secret (Requirement 044-003), and design 047's resolver refuses it |
| The throughput gate; the chart checker on every pull request | none (Tasks 052-11 and 052-6) | design 052: the `throughput-gate` job, which fails on the hand mutation and passes on its revert |
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
    request once it merges. A new Task 020-070 brings the retention design to 5.1.0 before Tasks 020-044 to 020-048
    start, and Task 020-052 (erasure) carries its own `Owner-gate:` line on Gap 1101 (correctness review, Major 2 and
    minor 9).
  - `plan.md`'s Constitution Check row now reads "Requires design change"; `data-model.md`'s grant note and "Purge
    privilege" section are annotated.
- `SPECIFICATION-INDEX.md` and `docs/roadmap.md`: the retention gate is settled for the purge and still open for the
  rest; the roadmap names 5.1.0.
- `docs/governance/README.md`: the row of this report.

Not changed, on purpose:

- Designs 044, 049 and 052 are approved and on `develop`; an approved design doc returns to draft if its text changes,
  so their changes are follow-ups for the agents that hold them, through their own follow-up tasks. Design 047 and
  feature 021's register are on `docs/windmill-parity-readiness`, which their agent owns.
- The 5.0.0 report keeps its follow-up "Retention in feature 020 still needs an amendment of Principle I": it records
  what was true on 2026-11-24.

## Follow-ups that are not this change's

- **The owner: erasure and `archive = NONE`** (feature 020, Gap 1101). The recommendation is above. Until the owner
  answers, feature 020 builds the purge under a policy with a required archive, and its erasure story (user story 09)
  waits.
- **The owner, if feature 021 asks: slot secrets in the third set.** They are out; feature 021 uses the first set.
- **The owner: an archive below `UP_TO_PII`.** Such a policy deletes payload values above its level with no copy. The
  text says so and requires the notice period for narrowing it. Recommended: keep it, since requiring every value would
  force `pii` into object storage; the owner may instead require full values, by a later amendment.
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
- **Design 052** (merged): Task 052-10 checks that this text admits the gate as Requirement 052-030 writes it and the
  chart checker's step on every pull request, and records the merge commit; Tasks 052-11 and the chart checker's move
  into the `documentation` job then follow. The draft's Task 052-15 is done by this text.
- **Design 044**: gaps 044-021, 044-024 and 044-025 close when this merges, and its Constitution Check for principles I
  and IV can drop the conditions they caused. Its move (ADR-0105) must also check that the target admits every accepted,
  unseen signal, update and cancellation, and the verification must reproduce a recorded deferred terminal command.
- **Feature 020, from the security review**: a retention role that the runtime role neither is nor can become, which
  alone executes the purge function and records an archive as verified (Gap 1102 then closes); the coverage check of
  the archive against the journal's last sequence under the lock, with re-archiving; a notice period for a change that
  shortens retention or moves the destination; the archive's size and SHA-256 on the stub, checked on every read
  (history export and feature 028's replay included); an audit record with the first purge transaction; and the
  runtime privilege check of `runs`, `steps` and every journal partition.
- **Design 047, from the security review**: tokens disclosed to activities join the redaction set as they do for
  scripts; a token with no expiry is disclosed only to a managed integration runner.
- **Design 049, from the correctness review**: the drop of the legacy partition writes a retirement record too, or the
  missing-partition alarm would flag it.
- **Design 049, from the security review**: the retirement credential is obtainable by no platform pod (today no chart
  Role grants Secret access, and the control plane mounts no service-account token), and a missing partition without a
  completed `journal_retirements` record raises an alarm.

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
| minor 5: owner questions 1080 and 1082 answered by the text | Fixed: the report says the text agrees with both as feature 020 decided them on 2026-11-24 (no default retention; stubs kept), and what stays 1082's |
| minor 6: the adoption sentence cited the wrong authority | Fixed: Owner decision 2026-11-25-3 for merging without the owner, 2026-11-24-4 for the rest, security exceptions to the owner |
| minor 7: "a PATCH amendment removes the slot secrets" was wrong | Moot: the slot secrets are out |
| minor 8: stale lines in designs 049 and 052; `crates/infra-postgres/AGENTS.md` | Fixed: the follow-ups name the lines; the crate guide gains the rule |
| Nit: the deferral should be journaled | Fixed |
| Nit: line wrapping in the roadmap and feature 020 | Fixed in the roadmap; the notes in 020 are single dated lines, kept so that a reader sees them where the old text is |
| Nit: the eligibility list read as complete | Fixed: "under the further conditions its design sets" |
| Nit: the fitness table could cite the `"grant_type"` rule | Fixed |

### Security review (no Blocker; five Major, six minor, three Nit; every one fixed, none settled)

| Finding | Disposition |
|---|---|
| Major 1: `security-and-sandbox.md` still released slot secrets | Fixed: it says what Principle V says |
| Major 2: the runtime role could mark invented archives verified and start a purge | Fixed: the function is executable only by a retention role that the runtime role neither is nor can become, and only that role records an archive as verified |
| Major 3: the archive was not tied to the journal it replaces | Fixed: the first transaction checks under the lock that the archive covers the journal up to the last sequence, or deletes nothing and re-archives |
| Major 4: a customer store and an immediate policy change let one insider destroy evidence; a forged archive could be served | Fixed: a notice period announced to the namespace's administrators for a change that shortens retention or moves the destination; the stub carries the archive's size and SHA-256, and every read is checked against them |
| Major 5: the access token was not called a secret value; a token with no expiry was disclosed like any other | Fixed: the token is a secret value under Principle V; a token with no expiry goes only to a managed integration runner |
| minor 1: a first-set secret to a managed runner was bound more weakly than a token | Fixed: bound like managed delivery |
| minor 2: the retirement credential and partitions dropped outside the command | Fixed: "holds or can obtain"; a missing partition without a completed retirement record is detected and alarmed |
| minor 3: a purge had no audit record until its last transaction | Fixed: an audit record also commits with the first transaction |
| minor 4: purging failed runs against Principle III's evidence rule | Fixed: for such a run the verified archive is the evidence Principle III retains; Principle III stays byte-identical |
| minor 5: the grant check missed `WITH GRANT OPTION`, `GRANTED BY`, role membership, `OWNER TO`, dynamic SQL, down migrations, `run_payloads`, and misread `REVOKE GRANT OPTION FOR` | Fixed in the static check, with tests seen failing first and the reviewer's own reproduction now refused; the runtime check through `pg_inherits` and memberships needs a Rust build and is feature 020's follow-up, named in the enforcement table |
| minor 6: the move could hand unseen events to code that refuses them | Fixed: the target must admit every accepted, unseen signal, update and cancellation; design 044 follows |
| Nit: the access token comes from a slot too | Fixed: "any other secret that one of its slots binds" |
| Nit: the new test class came after the main block | Fixed: it is before the final one |
| Nit: the governance index called it "a third declared set of secrets" | Fixed |
| Nit, in "classes with no finding": a pull request could edit the gate's scenarios | Fixed: "the base's scenarios" |

### Correctness review (no Blocker; six Major, eleven minor, five Nit; every one fixed, none settled)

| Finding | Disposition |
|---|---|
| Major 1: the deferral covered only unseen events, not open obligations, which is gap 044-024's own case | Fixed: both causes are named |
| Major 2: feature 020's design contradicts 5.1.0 while its plan read Pass | Fixed: the plan reads "Requires design change", Task 020-070 brings the design to 5.1.0 before Tasks 020-044 to 020-048, the data model is annotated |
| Major 3 (security): the retention role's credential had no home | Fixed: no process that holds the runtime credential or runs tenant code holds it |
| Major 4 (security): the runtime role could forge the stub | Fixed: only the retention role's function writes the stub and the purge state, a trigger refuses any other writer, and retirement judges "purged" by that role's records |
| Major 5 (security and the owner's words): an archive may keep no values; the notice period had holes | Fixed: the text says what the archive keeps and that values above the policy's level leave with no copy; the notice period covers creating a policy, re-credentialing the destination and narrowing what the archive keeps. The level itself is put to the owner (follow-ups) |
| Major 6: "the base's scenarios" could make the gate impossible to pass | Fixed, as design 052 settled it when it merged: both sides run the merge base's scenario, and one that cannot build or run against the head passes as not comparable only with a pilot re-run |
| minor 1: no path finished an abandoned mark-only purge | Fixed |
| minor 2: the coverage check needed appends refused | Fixed: the stub refuses every later append |
| minor 3: "accepts no further write" was stricter than the purge itself | Fixed: except the purge's and the retirement's own |
| minor 4: two statements contradicted design 049 | Fixed: the retirement command's credential and its mark-only purges |
| minor 5: the exceptions ignored the surviving-timeline rule | Fixed |
| minor 6: the grant check missed `WITH ADMIN`/`INHERIT`/`SET`, `CREATE ROLE … ROLE`, `ALTER GROUP`, `REASSIGN OWNED`, partitions, `format()` arguments, and the report overclaimed | Fixed in the check, with tests seen failing first; the report now says what it does not judge (`GRANT EXECUTE` on a definer function) |
| minor 7 (security): a slot's secret through the first set skipped the resource's checks | Fixed: "under that set's checks and the resource's authorities below" |
| minor 8: the report credited design 052 with a selector change it does not make | Fixed: design 052 as merged now writes the selector's `throughput_gate` output, and Point 3 cites it |
| minor 9: erasure lost its gate with Task 020-061 | Fixed: Task 020-052 carries its own `Owner-gate:` line |
| minor 10: ADR-0054's note was incomplete | Fixed: decision 4 and the four added obligations |
| minor 11: the move's check might not cover a deferred command | Fixed: "and any recorded deferral" |
| Nit: Governance and the footer | Fixed: the scope names the footer |
| Nit: this section was a placeholder | Fixed |
| Nit: "with no expiry" | Fixed: "no stated expiry" |
| Nit: design 049's legacy partition drop has no retirement record | Follow-up for design 049 |
| Nit: the check missed a last statement without `;` and block comments | Fixed, with tests |

## Risks and rollback

- Deleting history is irreversible. The text admits it only on two named paths, each after a verified archive. Both are
  designed but not built, and each ships with the fault tests named above.
- The third set widens what an executor can receive, by settings and an OAuth access token only. The existing two
  sets' checks bind it, and what the owner excluded is excluded by name.
- Rollback is a forward amendment that removes the deletion paragraphs, the third set and the gate. No code depends on
  this text, except the grant check, which a revert removes.
