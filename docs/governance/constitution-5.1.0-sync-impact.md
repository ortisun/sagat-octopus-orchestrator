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

The coordinating session drafts it, the three repository reviewers read it, and it merges without returning to the owner
unless a reviewer finds a Blocker. Findings are settled as "delegate (Owner decision 2026-11-24-4)", except security
findings, which are fixed. The owner may overrule any of it by a later amendment.

## Scope and why MINOR

Principles I, IV and V and the CI gates bullet of the Development Workflow change; Principles II, III, VI, VII and VIII,
the Delivery Constraints and Governance do not.

- **Principle I gains guidance**: a paragraph that admits two deletion paths, each with its preconditions, its audit
  record and what stays forbidden. Every obligation of 5.0.0 stays: the journal is still append-only while its run can
  change, no entry is ever rewritten, and the runtime role still cannot delete. Feature 020 (ADR-0054) and design 049
  call it MINOR: a principle's guidance materially expanded. The closest call is the first sentence, "no other component
  holds the only copy of a transition": after a purge the verified archive is the only copy. The text names that as the
  one exception, scoped to a purged, terminal, settled run, rather than redefining the sentence, so a reader of 5.0.0
  can still rely on it for every run that can change.
- **Principle V gains a third declared set** and replaces "Both paths" with "Every path". Every check of the two
  existing sets applies to the third; nothing is relaxed for the existing two.
- **Principle IV is reworded** where design 044 showed that 5.0.0 said something the design cannot and should not do
  (below). The obligations stay: one transaction for what commits, no automatic migration, a verified move.
- **The CI gates bullet gains one gate**, as versions 4.4.0 and 4.5.0 did for the budget and the step-log suites.

A principle redefined incompatibly would be MAJOR. None is: each change adds a path, a set or a gate with its own
conditions, or narrows the wording to what the design does while keeping the obligation. It adds obligations, so it is
not PATCH.

## Complete clause impact

| Unit of 5.0.0 | Disposition | Old | New | Why |
|---|---|---|---|---|
| Principle I, first paragraph | KEEP | — | Byte-identical | — |
| Principle I, deletion paragraphs | ADD | — | Two audited paths after a verified archive; what stays forbidden; the archive as the only copy after a purge | Owner decision 2026-11-25-3, point 1 |
| Principle I, rationale | REWRITE | ADR-0002, semantics, rule 3 | adds why history may be removed and the sources ADR-0054 and design 049 | Sources of the new paragraph |
| Principle IV, "A completion is one transaction" | REWRITE (last sentence) | "A completion that ends the run is refused while an accepted signal, update or cancellation is still unseen by the function." | "The command that ends the run is not applied while … is still unseen by the function: it is deferred, the rest of the completion commits all or nothing, and the function sees those events in its next activation." | Gap 044-024 |
| Principle IV, "A pin changes only by a verified move" | REWRITE (three sentences) | "… and the journal has not advanced past the activation that was verified …"; markers "so that a verified move can succeed" | "… and the run has not completed an activation past the one that was verified …; events committed after that activation … are delivered in the next activation, on the target code"; "The version a run started on stays its origin; its pin names the version its workflow tasks run"; markers' code "reaches a running run only through such a move" | Gaps 044-025 and 044-021 |
| Principle IV, every other sentence | KEEP | — | Unchanged | — |
| Principle V, "limited to two declared sets" | REWRITE | two | three | Owner decision 2026-11-25-3, point 2 |
| Principle V, third bullet | ADD | — | A declared resource's settings, its slots' values and, for an OAuth credential, only the current access token; never a refresh token, client secret or private or signing key; the authorities that must agree; how each executor kind is bound; a workflow task is not such an executor | Same; the text of design 047, Technical Plan 12 |
| Principle V, "Both paths MUST validate …" | REWRITE | "Both paths"; "Managed database delivery MUST also verify …" | "Every path"; "Managed delivery (database and integration runner) MUST also verify …" | The third set is bound by the same checks |
| Principle V, every other sentence | KEEP | — | Unchanged | — |
| Development Workflow, CI gates | REWRITE (one clause added) | "… the suite that budgets what they cost the database, and, …" | "… the suite that budgets what they cost the database and, once design 052 has built it, the throughput regression gate (…), and, …" | Owner decision 2026-11-25-3, point 3 |
| Every other unit | KEEP | — | Unchanged | — |
| Footer | REWRITE | `Version: 5.0.0`, last amended 2026-11-24 | `Version: 5.1.0`, last amended 2026-11-26, this report, the adoption sentence | — |

Of the 39 clauses of 5.0.0 that use MUST or MUST NOT (white space collapsed, the text split after every full stop,
semicolon and colon), 36 are present word for word in 5.1.0. The other three are the REWRITE rows above, each wider
than before: "Both paths MUST validate …" became "Every path MUST validate …", "Managed database delivery MUST also
verify …" became "Managed delivery (database and integration runner) MUST also verify …", and the CI gates clause gained
the gate. Principle V adds one MUST (the authorities that must authorize the third set's disclosure). No obligation is
dropped.

## Point 1: the two deletion paths

**What each path requires.** The text writes, for each path, its preconditions, its audit record and what stays
forbidden, as the owner's row and the coordinating session asked.

- *Per-run purge* (feature 020, ADR-0054 decisions 1 to 5). Preconditions:
  - an explicit namespace retention policy, without which every run is kept (owner question 1080's default);
  - past that policy's retention;
  - terminal, with its whole tree terminal and, for a chain, the chain final;
  - effects settled: not `PARTIAL` or `COMPENSATION_FAILED`, no unresolved unknown outcome or review. This is Principle
    III's evidence rule, which retention never erases;
  - an archive written and read back with matching size and SHA-256 before anything is deleted.

  Mechanism: one privileged database function, under the run's lock, removes the whole journal and content and leaves
  the run row as a stub that points to the archive. Audit: a value-free record committed with the deletion.
- *Partition retirement* (design 049, Requirement 049-009 and the retirement procedure).
  - Who: the operator, with a credential no platform process holds.
  - When: only after every run of the month has been purged on the first path, and after a check that the detached
    partitions hold no row of a run that is not purged.
  - Audit: the drop commits with the retirement's audit record and one per affected tenant. Every step is recorded so
    that a crash resumes.

  This path deletes no journal that the first path has not already emptied of meaning: it removes the rows of purged
  runs in bulk instead of one by one.
- *What stays forbidden*:
  - deleting or rewriting an entry of a run that can still change;
  - deleting part of a journal;
  - deleting without a verified archive;
  - any other path. The runtime role still holds no `DELETE` on runs, steps, the journal or payloads (ADR-0054 decision
    4, and feature 020's planned privilege sweep).

**What the owner's words leave out, and why this text follows them.** The owner approved deletion "only after a verified
archive" and the purge "under a namespace retention policy". Two things feature 020 and design 049 propose fall outside
those words, and the text does not admit them:

- **`archive = NONE`** (feature 020's Requirement 020-030, ADR-0054, design 049's "or its policy is `archive: NONE`"):
  deletion with no copy. Out.
- **Erasure** (feature 020's Requirement 020-037, the LGPD counterpart of retention): a purge at a subject's request,
  before the policy's retention, with the archive optional. Out as written. This matters for the business: a subject's
  request has a legal deadline, and a namespace whose policy keeps runs for a year could not answer it. Recommended to
  the owner: admit erasure as an audited request by a person holding `retention:manage`, for a terminal, settled run,
  still only after a verified archive, whose object the erasure then deletes. The archive's own deletion is the archive
  store's lifecycle, not the journal's, so this needs no exception to "only after a verified archive". The coordinating
  session cannot add it under Owner decision 2026-11-24-4, which does not delegate amending the constitution. Feature
  020's Gap 1101 stays open for both.

**The archive as the only copy.** After a purge, the archive in object storage is the only copy of that run's
transitions and is no longer protected by the database's transactions or backups (ADR-0054's own wording). Principle I's
first sentence says no other component holds the only copy of a transition. The text names the purged run as the one
exception instead of weakening the sentence, so the sentence still protects every run that can change.

## Point 2: the third declared set

The text is design 047's Technical Plan 12, with two changes. "Private key" becomes "private or signing key", the owner's
word. And one sentence is added: "A workflow task of Principle IV is not such an executor". Principle IV already says a
workflow task carries no secret value, and the third set must not become a way around that.

**Slot secrets.** The owner's row summarizes the set as "the settings of a declared resource and its current access
token". Design 047's text, which the coordinating session named as the wording, and feature 021's Decision 021-022 also
release "the values its slots bind at that revision: their secrets", for example the API key of an HTTP resource or the
password of an SMTP resource. Without them feature 021's integration runner cannot use a resource that is not OAuth, so
the text keeps them. They are bound by the same authorities as the first set's declared secrets (current membership,
binding and pool policy, the publisher's and initiator's permission, and the secret-access rules of each slot), and by
the same audit-before-values and revalidation rules. What the owner's "never" protects is kept exactly: a refresh token,
a client secret, and a private or signing key never leave the broker. This reading is the first item of the
after-the-fact report to the owner. If the owner meant settings and access tokens only, a PATCH amendment removes "their
secrets and," and feature 021 resolves static credentials through the first set instead.

**Every path.** "Both paths MUST validate …" becomes "Every path MUST validate …". Managed delivery, database and
integration runner alike, also verifies the recorded lease-owning executor subject and live session. Delivery to an
ordinary worker or a sandbox runner is bound like ordinary delivery, as design 047 states.

## Point 3: the throughput gate

When a pull request changes the code that runs a lease or a completion, it runs the throughput regression gate as well
as the budget suite. The selector is design 038's, the same files that already select the budget suite. The gate
compares the base and the head of the pull request on the same scenarios, alternated, on the standard hosted runner. It
fails on a slowdown larger than the minimum detectable effect that its recorded pilot measured on that runner, by the
rule design 052 states (Requirement 052-030: six alternated pairs, the head's median slower by more than that effect
and slower in at least five pairs).

Two choices:

- **"Once design 052 has built it."** The job does not exist yet (Task 052-14). A MUST that no CI job can satisfy would
  put every hot-path pull request out of compliance until it exists, so the obligation starts with the job.
- **The thresholds stay in the design.** The constitution names what the gate compares and on what. The numbers belong
  to design 052, which measures them, so recalibrating the gate after a new pilot needs no amendment. The owner's "Nada
  pago" (no paid runners) is why the text says "the standard hosted runner".

## Point 4: design 044

- **Gap 044-024, a deferred terminal command.** 5.0.0 said a completion that ends the run "is refused" while an accepted
  signal, update or cancellation is unseen. Design 044 found that a refused completion repeats for ever when the cause
  is an obligation the function cannot settle, so it defers the terminal command instead and commits the rest. The new
  text says that: the command that ends the run is not applied, it is deferred, the rest of the completion commits all
  or nothing, and the function sees the events in its next activation. The obligation of 5.0.0 is kept: a run never
  ends over an event its sender was told was accepted. Only the mechanism changes.
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
| I | REWRITE | 32d2ec982a4ae0995a472628184c4762ea740f9081886bb4c78fa461237a5a21 | a461ccf73107ad7de83cf1c2f3891a75db7365f4b4e88a759a6b2f127fcd9d52 |
| II | KEEP | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 |
| III | KEEP | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 |
| IV | REWRITE | 2bf35e4b0a41b41d2bd7a98dd2e2825a18ea635bc8eed63ed34b6c98e8c6e5eb | 41825ddb2cdfdef007f05af1e2ac7f6a42ae0987f1aad93c6e989d7c7c996f86 |
| V | REWRITE | b816cb76163ef1d8cc8579bb3e326f59c774b077131029cfd64fa45d470f38f9 | dfe19bb31f789a8cdec823f65197060693a26dc1308a559dbeb4c6cb41b2db08 |
| VI | KEEP | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc |
| VII | KEEP | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 |
| VIII | KEEP | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 |

## What enforces each new rule

| Rule | Check today | Check to add, and where |
|---|---|---|
| The runtime role cannot delete journal rows | the privilege audit of the runtime role (`SAGAT_TEST_AS_RUNTIME_ROLE`) and the append-only triggers | feature 020: the sweep with and without a purge authorization (ADR-0054, Enforced by) |
| Deletion only after a verified archive; only the two paths | none (neither path is built) | feature 020: the purge refuses an unverified archive (a fault test with a corrupted and a missing object); design 049: Task 049-14's crash and race tests |
| The third set never releases a refresh token, client secret or private or signing key | none (design 047 not built) | design 047: the broker's canary tests across the protocol, logs and audit |
| A workflow task is not an executor of the third set | none | design 044: the workflow task carries no secret (Requirement 044-003) and design 047's resolver refuses it |
| The throughput gate | none (Task 052-14) | design 052: the `throughput-gate` job, which fails on the hand mutation and passes on its revert |
| A deferred terminal command; the move clause | none (design 044 not built) | design 044's tests of `UNHANDLED_EVENTS` and of the move (ADR-0105) |

## Dependents updated in this change

- `docs/architecture/execution-semantics.md`: the persistence line names the two deletion paths, not built.
- `docs/architecture/security-and-sandbox.md`: the disclosure boundaries name the third set and "every path".
- `docs/test-strategy.md`: the hot-path selection names the throughput gate, not built.
- `docs/adr/ADR-0054-…`: a dated note on its "Required amendment of principle I": applied in 5.1.0 with three
  differences (two paths, no `archive = NONE`, no erasure outside the policy).
- Feature 020 (`spec.md`, `plan.md`, `research.md`, `tasks.md`): the same note. Gap 1101 is closed for the per-run purge
  and stays open for `archive = NONE` and erasure, with the recommendation to the owner. Task 020-061 is annotated, to be
  ticked with this pull request once it merges.
- `SPECIFICATION-INDEX.md` and `docs/roadmap.md`: the retention gate is settled for the purge and still open for the
  rest; the roadmap names 5.1.0.
- `docs/governance/README.md`: the row of this report.

Not changed, on purpose:

- Designs 044, 047, 049 and 052 and feature 021's register live on other branches, which their agents own
  (`feature/044-workflow-as-code`, `docs/windmill-parity-readiness`, `docs/scale-resilience-readiness`). Their changes
  are follow-ups.
- `scripts/select-test-targets.py` does not select the throughput gate: the gate is a job of its own, which design 052
  adds with its selector (Task 052-14).
- The 5.0.0 report keeps its follow-up "Retention in feature 020 still needs an amendment of Principle I": it records
  what was true on 2026-11-24.

## Follow-ups that are not this change's

- **The owner: erasure and `archive = NONE`** (feature 020, Gap 1101). The recommendation is above. Until the owner
  answers, feature 020 builds the purge under a policy with a required archive, and its erasure story (user story 09)
  waits.
- **The owner: slot secrets in the third set** (above). They are kept, and this is the first item of the after-the-fact
  report.
- **Design 049**: drop "or its policy is `archive: NONE`" from Requirement 049-009, and change the `Owner-gate:` lines of
  Tasks 049-11 and 049-14 to cite Owner decision 2026-11-25-3 once this pull request merges. Task 049-18 is done by this
  text.
- **Design 052**: change Task 052-14's `Owner-gate:` line to cite Owner decision 2026-11-25-3; Task 052-15 is done by
  this text.
- **Design 047**: Technical Plan 12's condition is met. Its Constitution Check row for Principle V reads Pass once this
  merges, and it uses "private or signing key".
- **Feature 021**: Decision 021-022 is answered; its capability resolution may ship under the third set.
- **Design 044**: gaps 044-021, 044-024 and 044-025 close when this merges, and its Constitution Check for principles I
  and IV can drop the conditions they caused.

## Review

The read-only `architecture-reviewer`, `security-reviewer` and `correctness-reviewer` read the diff of this pull request
one after the other, with the owner's row and no conclusion of the author's. Each finding is listed with what was done.

### Architecture review

(Filled in after the review.)

### Security review

(Filled in after the review.)

### Correctness review

(Filled in after the review.)

## Risks and rollback

- Deleting history is irreversible. The text admits it only on two named paths, each after a verified archive. Both
  paths are designed but not built, and each ships with the fault tests named above.
- The third set widens what an executor can receive. The existing two sets' checks bind it, and what the owner excluded
  is excluded by name.
- Rollback is a forward amendment that removes the deletion paragraphs, the third set and the gate. No code depends on
  this text yet.
