# Owner decisions

Decisions that only the product owner could take, with what each one unblocks. Each entry is also
recorded where the open item lived (gap register, ADR, runbook or specification). "Not built"
means the decision is taken and the implementation is still to be done; it is not evidence.

## 2026-11-08 — A reset that waits for its source expires, and the CI scope pull request was merged

Feature 018. The assistant asked the owner two things that were theirs: to merge pull request 120 (a shared file selects no
integration suite in a pull request, OD-2026-11-06-2) and what a pending `CANCEL_SOURCE` entry should do when its source never ends
(an entry never expired and could not be withdrawn: a run in manual review could hold the 16 places of a namespace, and the
replacement run could start days later on an acknowledgement given long before), with three options: expire after a time, an
operator withdraws it, or both. The owner answered "Faça isso por mim". The assistant took it as the word to merge pull request 120
(merged 2026-11-09, `5057de7b`) and as a delegation of the second decision, which it took the conservative way. The owner may
overrule it; until the owner changes the cell to `owner` it is not an owner gate.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-08-1 | "Faça isso por mim" (the owner's whole answer to: merge pull request 120, and expire or withdraw a pending `CANCEL_SOURCE` entry). Taken: **expiry only, built next.** An entry that is still `PENDING_SOURCE` 24 hours after its request (the deployment may set 1 hour to 7 days) ends `FAILED` with the closed reason `SOURCE_DID_NOT_END`, no run is made and its place in the namespace is freed. No call that withdraws an entry is built: the operator's lever is to end the source (a run in manual review is resolved like any other run) or to let the entry expire; a withdrawal is a later decision. `SAGAT_RESET_CANCEL_SOURCE_ENABLED` stays off by default until the expiry is built and reviewed. | delegate | Built and reviewed 2026-11-09 (three independent reviews found no blocker; their findings are fixed, D2079 to D2082; the gates were rerun by the coordinator on the final tree; the flag stays off, and lifting it by default waits for the alert of G2026) | specs/018-schedules-bulk-operations/research.md (D2000, G1991, D2059 to D2068, G2014 to G2018), docs/runbooks/run-reset.md, docs/adr/ADR-0081-cancel-source-is-switched-on-per-process-until-the-capability-registry-exists.md |

## 2026-11-06 — Mutation testing runs only when a release is closed

The owner gave the instruction in the Claude session, right after the pull requests that closed the host of feature 035 up to
its REST catalog, each of which had a mutation pass of its own (a few minutes to half an hour of throw-away runs per pull
request). It follows the rule on CI: what a pull request and a merge cost is cut to what the change needs, and the expensive
check waits for the release.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-06-2 | "Desabilite também testes mutantes. Deixe somente ao fechar release igual a regra anterior" | owner | Decided | docs/engineering/ai-engineering.md, docs/test-strategy.md |

Read as the owner wrote it: no mutation run while a feature is built, in a pull request or in the CI; one when a release is
closed. It replaces the pause of 2026-10-09 ("re-enable only when asked"), which had said only that mutation testing was off.
The Constitution does not mention mutation testing, so it is not amended: the rule lives in
[the engineering guide](engineering/ai-engineering.md) and [the test strategy](test-strategy.md). A test is still written
first and seen failing, and an independent review still reads a slice that crosses a trust boundary; only the weakening of
each guard to see that a test notices it moves to the release.

## 2026-11-05 — A merge into `develop` starts no CI

The owner gave the instruction in the Claude session while the assistant was merging a stack of pull requests, each of
which would have started a run on `develop`. The assistant recorded it; the rule is Constitution 4.2.0 (CI gates).

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-05-1 | "Desativar o ci para merges na develop. Deixar o ci somente para merges de release" | owner | Decided | docs/constitution.md, .github/workflows/ci.yml |
| OD-2026-11-06-1 | "O que deve ter é execucao de testes do que foi alterado ou modificado e seus dependentes, e integração também só do que foi alterado" | owner | Decided | docs/constitution.md (4.3.0), scripts/select-test-targets.py |
| OD-2026-11-06-2 | "Sim. Rodar os testes somente dos arquivos alterados e suas dependencias." (the answer to: should a shared input, that is a contract, `tests/support`, `tests/fixtures`, `runners/`, a dependency of `tests/Cargo.toml`, select no suite in a pull request and leave all to the release?) | owner | Decided | scripts/select-test-targets.py, docs/test-strategy.md, tests/AGENTS.md |

Read as the owner wrote it: the CI that a *merge* into `develop` starts (the `push` event) is turned off. The CI of a *pull
request* is not a merge and keeps the rule of 4.1.0 (the tests of what it changes; the whole stack for a pull request into
a release branch). If the owner meant that a pull request into `develop` should start no CI either, that is one more
amendment and one more line of the workflow.

## 2026-11-06 — A pull request runs what it changed, its dependents' unit tests, and only the integration it changed

Row OD-2026-11-06-1 states what a pull request runs. The CI of a pull request selected every integration suite whose sources
name an affected package; with `sagat-application` changed that was nearly all of them, and each pull request of feature 035
ran for about an hour and three quarters (the assistant cancelled four runs of it). The owner said that a pull request should
run the tests of what was changed and of its dependents, and the integration suites only for what was changed. From 4.3.0 the unit tests of a changed package and of every package that depends on it run, and an
integration suite runs only when it was itself changed (its files, its `[[test]]` entry or a harness it drives). A
release still runs everything, and a merge into `develop` still starts no run.

Row OD-2026-11-06-2 closes what 4.3.0 left: it still selected every integration suite when a file that all of them share
(`tests/support`, `tests/fixtures`, a `runners/` directory, a contract under `specs/`, a dependency of `tests/Cargo.toml`)
changed. Pull request 98 of feature 018 changes the OpenAPI contract and the test support, so it was selected as "every
target" (about ninety suites, two hours a run, two heads in a row). The assistant put the question to the owner, who said yes.
From now on those files select no suite; the unit tests of the packages they belong to (and of their dependents) still run,
`Cargo.lock` and the toolchain files still select the unit tests of every package, and the release runs every suite. The
price, as the owner was told: an error in the support or in a contract that no changed suite exercises is found by the
release and not by the pull request.

## 2026-11-05 — The lake containers share the runner's group

Feature 035. The relay token of an Iceberg attempt is meant to be readable by the attempt's container alone, which takes
`CAP_CHOWN` in a runner that the contract says is a plain user (`USER 10001`) and runs with `no-new-privileges`. The assistant put
three options to the owner: the runner runs as root with that one capability, the sandbox runner prepares the directory, or the
slots share a group. The owner chose the third, in the Claude session.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-05-2 | "Vamos com a letra C. As vagas compartilham um grupo." | owner | Decided | specs/035-iceberg-write-steps/contracts/lake-runtime.md, crates/infra-sandbox/src/lake.rs |

What the choice costs, as the owner was told: no privilege is added to the process that holds the credentials, and the user id of a
slot stops protecting the files between attempts. What keeps two attempts apart is the mount (a container is given its own
sub-path only, which the isolation tests show) and the token on the relay. If the engine failed to keep a container to its sub-path,
the files of another attempt would be readable to it. The test `the_user_of_a_slot_is_not_a_boundary_between_attempts` states
this and fails on the day the user becomes a boundary again.

## 2026-11-05 — A correction to a delegated decision

The row below corrects OD-2026-11-04-3 under the same delegation. It is `Decided by: delegate` and reversible.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-05-1 | The sentence of ADR-0075 telling a membership creator to lock chains in ascending order and then runs is withdrawn. A review of the lock-graph draft found it unreconciled with ADR-0030 §19 and ADR-0037 §6. ADR-0075 was accepted by delegation on 2026-11-04 in a pull request that had not been merged, so `develop` never carried the sentence as an accepted rule and the text was corrected before the acceptance reached it. ADR-0075 keeps the rule that a creator takes the group header `NOWAIT` before the first member insert; the chain and run lock set of the creator is open (ADR-0078). | delegate | Decided | docs/adr/ADR-0075-worker-leases-retain-child-root-possession.md |

## 2026-11-04 — Delegated decisions for release 0.3.0 (capacity ADR, collection and wait ceilings)

The owner delegated the open decisions of the 0.3.0 ledger: "Tome as decisoes por mim. Que os melhores
especialistas em arquitetura, dados, banco de dados, decidam." The assistant put each question to an
independent read-only reviewer (architecture, PostgreSQL and transactions, data and performance),
gave them the question and the files and not its own leaning, and decided from their opinions, in
[the evidence](../specs/014-workflow-primitives/evidence/delegated-decisions-20261104/README.md).
Nothing was run. Every row is `Decided by: delegate`: reversible, and the owner confirms one by
changing that cell to `owner`. `scripts/check-design-docs.py` does not accept a `delegate` row as an
owner gate. Release scope stays as the owner decided on 2026-10-26: the original acceptance criteria
of the eight tasks, which no row below lowers.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-04-3 | ADR-0075 is accepted for the ChildRoot worker-lease guard only, with the amendments of the architecture and PostgreSQL reviews. Production activation is not accepted: the guard stays off and no membership may be created until every condition of the ADR's Activation gate holds. | delegate | Decided | docs/adr/ADR-0075-worker-leases-retain-child-root-possession.md |
| OD-2026-11-04-4 | Gap G721 is not closed by ADR-0075, ADR-0030, ADR-0037 or ADR-0038: a separate lock-graph ADR (total lock order, possession protocol, caller boundaries, creator, SCOPED_PATH, runnable predicate G715, rollout, fitness checks) must be accepted before any production caller. | delegate | Decided, not built | specs/014-workflow-primitives/capacity-vacate-acquire.md |
| OD-2026-11-04-5 | O460 and O464. Collections publish `maxItems` 10,000 if the T014-042 acceptance run on the release candidate meets its thresholds, otherwise 1,000. Loops publish `maxIterations` 1,000 unless a 10,000-iteration loop completes within 1,800 s on that run. The worker report path (G520) is a documented limitation, not a release blocker. The limit goes in the runbook and the release notes and is bounded by the tenant quota; validation keeps accepting up to 10,000. | delegate | Decided, not built | specs/014-workflow-primitives/research.md |
| OD-2026-11-04-6 | O463. The accepted ceiling is 20,000 pending waits (approvals, timers, event waits) per scheduler replica at the default 500 ms interval, observed by a new `scheduler_tick_seconds` metric with a slow-tick alert and an alert above 16,000 waits per replica. Index-only discovery (the `node_kind` column) is required before 0.3.0 only if the acceptance run fails the idle-tick thresholds, burst of 5,000 approval starts from a definition of at least 100 KB included. The reviewer made it unconditional; the delegate made it conditional because the cost on large definitions is inferred from the SQL and not measured. | delegate | Decided, not built | specs/014-workflow-primitives/research.md |
| OD-2026-11-04-7 | T014-042 keeps its original acceptance criteria (10,000 items at widths 1, 16 and 256, 10,000 suspended approvals, the nested 100,000-instance workload). A flat 10,000 items by 10 instances run is extra evidence and not a substitute, and the task stays open while nested scopes are closed. The thresholds and the method (release build, runtime role, separate database host, two repetitions with the worse one counting) are those of the data review. | delegate | Decided | specs/014-workflow-primitives/tasks.md |
| OD-2026-11-04-8 | Before `release/0.3.0` is cut, the whole stack is dispatched manually on `develop` as a rehearsal (a manual run is a release run, OD-2026-11-04-2). T014-040, T014-042 and T014-043 are recorded from one release-candidate commit that is an ancestor of `develop`. | delegate | Decided | .github/workflows/ci.yml |
| OD-2026-11-04-9 | `feature/014-bounded-execution`, superseded by develop, is deleted. A verified bundle (tip `4f70bd08`) and the patch of its uncommitted edits are kept in `~/Documents/Projects/sagat-worktree-archive-2026-11-02/`. | delegate | Done | specs/014-workflow-primitives/implementation-ledger.md |

## 2026-11-04 — The CI runs the tests of the change; the whole stack runs for a release

The owner gave two instructions in the Claude session, the second replacing the first. The assistant recorded them; the
rule is Constitution 4.1.0 (CI gates).

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-04-1 | "Quero que os testes de integração só rodem quando modificados ou adicionados. Eles custam muito do tempo de entrega." Implemented in #87 and #89. | owner | Superseded | .github/workflows/ci.yml |
| OD-2026-11-04-2 | "Quero desativar alguns steps do ci. Está demorando muito. Quero que somente os testes unitários das alterações sejam executados e também os testes de integração que tocam nos arquivos afetados pelas edições. O que quero que rode completo é quando eu fechar uma release. Nesse momento preciso que rode toda a stack de testes, lint etc... Deixe isso como regra." | owner | Decided | docs/constitution.md, .github/workflows/ci.yml |

## 2026-11-03 — EGM replaces Spec Kit

Rows 1–8 record what the owner answered in the Claude session of 2026-11-02 (the gates of `egm-brainstorm` and
`egm-draft`). On 2026-11-03 the owner delegated the open process decisions to the assistant ("Tome as decisões por
mim."); rows 9–31 record the assistant's decisions A to W of
[`docs/designs/constitution-4.0.0-egm.md`](designs/constitution-4.0.0-egm.md). They are reversible and stay
`Decided by: delegate` until the owner confirms one by changing that cell to `owner`; `scripts/check-design-docs.py`
does not accept a `delegate` row as an owner gate or as the acceptance of a finding.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-03-1 | EGM is the only specification workflow. Only the tooling is removed; every `specs/NNN-*/` directory stays. | owner | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-2 | All eight principles stay whole. The new constitution is `docs/constitution.md`. | owner | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-3 | Approval is automatic and has one state; Blocker and Major block; a contested or unfixed finding is settled only when the owner accepts it in writing. | owner | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-4 | The approval record is a status block in the doc; ticks after approval are checked against the reviewed commit. | owner | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-5 | The whole EGM procedure lives in the constitution. | owner | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-6 | The EGM pin is a settings entry plus a list of accepted hashes in the repository. | owner | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-7 | An owner decision gates a task, not the doc; the three repository reviewers are part of approval. | owner | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-8 | The 20 features with open tasks are converted into design docs by an agent that maps the sources, judged by `egm-review`; the id allocator ships with the tooling. | owner | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-9 | A. The conversion of the 20 features leaves the design and becomes a follow-up design. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-10 | B. The Spec Kit tooling is removed in PR 2, together with the constitution. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-11 | C. Version 4.0.0, and the Versioning clause is amended: a change of the mandated specification process is MAJOR. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-12 | D. No `CODEOWNERS` and no branch-protection change in this work (repository settings; recorded as a risk). | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-13 | E. No document-level `Scope-approval` field; the per-task owner gate stays. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-14 | F. No blob fallback for the reviewed commit; branches that hold approved docs merge `develop` and are not rebased. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-15 | G. Counts are stated per report with one `Counts:` line. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-16 | H. Owner gates cite an id: `Owner-gate: OD-YYYY-MM-DD-N`. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-17 | I. The id allocator prints the next number only; no table of reserved ranges. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-18 | J. The contests of the first review round on automatic approval, the allocator and the diff mechanism are accepted: the owner's decisions stand. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-19 | K. Security exceptions use the existing homes; confirmations keep the recommendation practice outside EGM sessions; review reports live in `docs/designs/NNN-slug.reviews/`. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-20 | L. The "broader reviewed Constitution candidate" of the 3.0.0 report stays out of scope. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-21 | M. Transitional clause: until converted, a feature's `spec.md`, `plan.md` and `tasks.md` stand in for its design doc, edited by hand. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-22 | N. Delegated decisions carry `Decided by: delegate`, which the check does not accept as an owner decision. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-23 | O. No new `permissions.allow` entry and no `permissions.deny` entry for the plugin. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-24 | P. ADR-0077 records the decision. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-25 | Q. An unnumbered design declares a `Task-prefix`; this design uses `EGM`. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-26 | R. Bootstrap: the first design is approved by hand and its tasks are ticked in a closing commit; PR 1 merges without that approval. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-27 | S. The body rule compares the normalized text of the reviewed commit with the working text, not diff hunks. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-28 | T. The plugin is enabled in PR 2, not PR 1. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-29 | U. The tree hash includes the file mode, excludes only image and font files below `assets/` and the `.in_use` markers, and refuses symlinks. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-30 | V. An approval record has exactly four reports, one per reviewer; an acceptance row cites `<report>#<n>`. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-31 | W. Checks that need the reviewed commit run only for docs that a pull request changes. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-32 | The owner delegated the approval of PR 1 and PR 2 and the confirmation of rows 9 to 31 to the assistant: "Decida por mim" (2026-11-03), after being told what needed approval. | owner | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-33 | X. Keep the checklist gate in the transitional clause: unticked `checklists/*.md` items of a feature not yet converted block its implementation until the owner accepts them. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-34 | Y. Keep the EGM plugin enabled in project scope, as the owner chose on 2026-11-02. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-35 | Z. Keep version 4.0.0 (MAJOR) and the amendment of the Versioning clause. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-36 | AA. Leave the design `Draft`: no finding is accepted on the owner's behalf, and rows 9 to 31 stay `delegate` (a delegate cannot become the owner by editing a cell). | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-37 | AB. Merge PR 1 and then PR 2 with merge commits once CI is green, on the owner's delegation. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |
| OD-2026-11-03-38 | AC. Keep ADR-0077 `Proposed`: the ADR rules say a person accepts. | delegate | Decided | docs/designs/constitution-4.0.0-egm.md#decisions |

## 2026-11-02 — Feature 035 implementation gates (Iceberg writes and step-output destinations)

The owner answered the four questions below in the Claude session that took over feature 035, each
by the recommended option. They open task T035-001 of `specs/035-iceberg-write-steps/tasks.md`;
nothing here is built.

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | ADR-0071 | **Accepted** (“Aceitar (Recomendado)”): the platform-owned Python lake runtime, pinned by hash and confined to `lake` sandbox containers, extends the technology baseline. The three constitution-V conditions (T-isolation, relay and hostile-catalog canaries, T-commit-once) still gate the end of I1. | `docs/adr/ADR-0071-…` (status line) | Decided; not built |
| 2 | O035-003 ceilings | **The plan's “Finite ceilings” table is approved as written** (D035-045), including the reference environment of SC-035-009. | `specs/035-iceberg-write-steps/research.md` (D035-070) | Decided; not built |
| 3 | O035-004 failure policy | **`hold` is the default** for synchronous destinations (D035-023): an unproven outcome waits for a person. | `specs/035-iceberg-write-steps/research.md` (D035-070) | Decided; not built |
| 4 | O035-005 020 sinks | **020's `OBJECT_STORAGE` and `CLICKHOUSE` sinks reference the new 017 connection kinds** `object_storage` and `clickhouse`; a destination is configured once for both modes. 020 is still a draft in PR #53's lineage. | `specs/035-iceberg-write-steps/research.md` (D035-070) | Decided; 020 not updated |
| 5 | Audit gate for the independent reader | **Ignore RUSTSEC-2026-0194, RUSTSEC-2026-0195 and RUSTSEC-2024-0436, documented** (“Ignorar os 3, documentados”): the `iceberg` crates are test-only dev-dependencies of `sagat-tests`, enforced by `scripts/check-architecture.py`; re-evaluate on 2026-12-02. | `.cargo/audit.toml`; `specs/035-iceberg-write-steps/research.md` (G035-024, D035-072); `specs/001-platform-foundation/research.md` (G035-024) | Decided; ignores in place |

Not asked, because neither blocks I0–I1: O035-002 (priority of ORC/Avro and Kerberos Hive) stays open
for a later delivery; and developers may **not** publish writes, which keeps D035-066 (admin-only
`database:write`) and ADR-0036 decision 5 unchanged. Either can be reopened by the owner.


## 2026-11-01 — Feature 037 delivery, B01 split and upgrade compatibility

The owner answered three questions in the Claude session that took over Codex's work. The answers
were "Sim, autorizo", "Separar a B01 num PR próprio (Recomendado)" and "Manter upgrade da v0.2
(Recomendado)".

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | Feature 037 US01–US03 implementation | **Authorized to merge** once the independent review blockers are fixed and CI is green. This also adopts Constitution 2.0.0, which changes Principle III only, and accepts ADR-0076. US04 (provider qualification, T037-032/033) stays open. | PR #75, [ADR-0076](adr/ADR-0076-compensation-failure-and-uncertain-effects.md), [sync report](governance/compensation-safety-sync-impact.md) | Merged into `develop` with PR #75 (2026-11-02) |
| 2 | B01 amendment (Principle V secret disclosure to executors, no tenant metric labels, Redis transport baseline) | **Split into its own pull request.** It is not adopted with 037, and its text needs its own owner approval. Because 037 takes 2.0.0, it is renumbered 3.0.0. | PR #77 | **Approved by the owner on 2026-11-01**: “As prs 53, 60, 62, 63 e 77 mergeie em develop”. Adopted when PR #77 merged, on top of 2.0.0 |
| 3 | Upgrade from released v0.2 | **Keep in-place upgrade.** The new rules apply to new commands and planning. Replay MUST still accept journals written by v0.1/v0.2: legacy negative-lookup pairings, privileged unknown retries and defensive compensation plans. The "fresh installation only" wording in the 037 runbook and plan is withdrawn. | PR #75, `docs/runbooks/sagas.md`, 037 `plan.md` | Decided; implementation in progress |

## 2026-11-01 — Compensation safety (B02)

The owner answered **“Sim aprovo”** to the two recommended directions below. This authorizes
recording the decisions and preparing their specification/dependent diffs; it does not authorize
implementation, a final wire contract, constitutional adoption, provider qualification or release.

| Item | Approved direction | Record / remaining work |
|---|---|---|
| S1-B; feature 014 O381 | Human review by default; explicit published `fail` for explicit sagas. Terminal `COMPENSATION_FAILED` plus retained journal/step evidence is the platform's post-terminal tracking, with no active review queue promised. Cancellation/deadline preserves pending review in explicit sagas and scopes. | [Feature 037](../specs/037-compensation-safety/spec.md), [research](../specs/037-compensation-safety/research.md). O381 direction answered; runtime correction pending. O380 scope policy fields remain deferred. |
| S2-B | Consultation/review for unknown external effects, regardless of `highRisk`. Defensive compensation requires pinned, authorized qualification with authoritative provider evidence and adapter tests for applied/absent/late effects. Compensation success requires business finality; safe repeat of an unknown compensation outcome needs its own provider guarantee and stable identity. | Feature 037; contract design, adoption diff and real acceptance evidence pending. SDK integrations and plugins follow identical rules; no provider is qualified by this approval. |

*Superseded 2026-11-01 (item 3 above): in-place upgrade from v0.2 is kept.* Fresh installation/no legacy upgrade was said to be already approved for the engineering reset, but no such entry exists in this file. This entry
introduces no regulatory retention period, financial policy or exactly-once promise.

## 2026-10-29 — ChildRoot capacity coordination

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | O705, idle-holder convergence | “Sweeper no tick do scheduler, em transação própria (recomendado)”. Release idle possession in a level-triggered scheduler sweep, one native transaction per command; worker reports and internal step transactions do not release possession inline. | [Coordinator plan](../specs/014-workflow-primitives/capacity-coordinator.md), `research.md` D714/O705 | Owner approved; implementation and recovery evidence required |
| 2 | O706, possession enforcement | “Apenas concessão de leases aos workers (recomendado)”. Require possession when granting worker leases; internal scheduler transitions remain outside this gate. | [Coordinator plan](../specs/014-workflow-primitives/capacity-coordinator.md), `research.md` O706 | Owner approved; shared eligibility, claim guards and wiring still required |

## 2026-10-28 — version-neutral source names

The owner requested: “Nao quero ter esses arquivos v2 nem v3”. Active upgrade tests, fixtures,
runner scripts and implementation/profile documents use version-neutral names (`release_upgrade`,
`release-upgrade`, `implementation-ledger`). The tested release remains pinned explicitly in the
fixture content. Immutable historical receipts retain their original recorded paths and hashes.

## 2026-10-28 — elastic scaling (feature 036)

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | O1706, who applies replicas and resources when a scaling policy is enforced | Option B, chosen in `/speckit-clarify`: by default the environment's autoscaler or an external controller follows the platform's published desired capacity; in addition an **optional applier** (a separate component, off by default, with only the permissions to change the replicas and resources of the platform's own workloads) carries out enforced decisions in the supported orchestrator, for operators who want a closed loop. | [`specs/036-elastic-scaling/spec.md`](../specs/036-elastic-scaling/spec.md) (Clarifications, FR-036-051 to FR-036-055, US12, SC-036-015 and SC-036-016) | Specified; not built |
| 2 | O1707, where outside data for scaling comes from | Option B, chosen in `/speckit-clarify`, with the owner's words: “O orquestrador deve consultar as métricas (mensagens enfileiradas, cpu, memoria, etc..). Ele dará os sinais ao keda ou outro provider fazer o scaling in ou out.” The platform queries external sources declared by configuration, evaluates, and hands the signals and the decisions to the autoscaling provider. Declaring a source is an operator action (destination allowlist, credentials by reference, bounded read-only queries); tenants publish custom signals instead. | [`specs/036-elastic-scaling/spec.md`](../specs/036-elastic-scaling/spec.md) (Clarifications, FR-036-056 to FR-036-061, US13, SC-036-017 to SC-036-019) | Specified; not built |
| 3 | O1708, programmatic access to signals and policies and the SDKs | Option A, chosen in `/speckit-clarify`: the management interface is the documented, versioned REST contract plus the operator command line. The six official SDKs do not change in this feature (their client covers run operations only, and a new client operation must enter all six with a scenario and a golden). | [`specs/036-elastic-scaling/spec.md`](../specs/036-elastic-scaling/spec.md) (Clarifications, FR-036-019, Assumptions, Out of scope) | Specified; not built |
| 4 | O1703, what a tenant may do about scaling | Option B, chosen in `/speckit-clarify`: a tenant reads the lag, state and decisions of its own connectors and adjusts their in-flight parallelism and rate limit by hand within its quota; every scaling policy is operator-only; a tenant never controls replicas or resources. Consequence recorded as O1704 (not yet confirmed): a custom signal published by a tenant has no automatic effect until an operator admits it to a policy. | [`specs/036-elastic-scaling/spec.md`](../specs/036-elastic-scaling/spec.md) (Clarifications, FR-036-023, FR-036-024, US05) | Specified; not built |
| 5 | O1709, delivery order of feature 036 | Option B, chosen in `/speckit-clarify`: the first delivery is US01, US02, US03, US04, US05, US06, US10 and US13 (consumer lag, balanced Kafka partitions, scaling on lag, policies by configuration and through the management interface, explanation, safety, pulled sources, signals and desired capacity exported to the autoscaling provider). The other roles (US07), the dedicated ingestion role (US09), vertical scaling (US08), the optional applier (US12) and the benchmark on separate machines (US11) follow. | [`specs/036-elastic-scaling/spec.md`](../specs/036-elastic-scaling/spec.md) (Clarifications, Context "Delivery order") | Specified; not built |

## 2026-10-27

Answers to the scope questions of the feature 035 specification (Apache Iceberg write steps): items 1–3 were asked before it was written, items 4–8 at `/speckit-clarify`, items 9–11 when the owner extended the scope to step-output destinations. "Not built" means the decision is taken and the implementation is still to be done; it is not evidence.

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | 035 write modes | **Append, overwrite and upsert** (merge by key) in the first delivery; the recommendation was append + overwrite with upsert later. | `specs/035-iceberg-write-steps/research.md` (D035-001) | Decided; not built |
| 2 | 035 catalogs and storage | **Broad set**: REST, AWS Glue, Hive Metastore and JDBC catalogs; S3-compatible, Google Cloud Storage and ADLS Gen2 storage; the recommendation was REST + S3-compatible first. | `specs/035-iceberg-write-steps/research.md` (D035-002) | Decided; not built |
| 3 | 035 destination model | A **new connection kind of the feature-017 catalog** (revisions, OpenBao secrets, probe, managed executor, network policy, ad hoc admission); no dependency on the PR #53 drafts. | `specs/035-iceberg-write-steps/research.md` (D035-003) | Decided; not built |
| 4 | 035 batch size | At most **2 GiB per step** in at most 32 record blobs of 64 MiB, streamed into files. | `specs/035-iceberg-write-steps/research.md` (D035-013) | Decided; not built |
| 5 | 035 deletes in upsert | An upsert may declare a **delete marker** field; marked keys are removed in the same commit. | `specs/035-iceberg-write-steps/research.md` (D035-014) | Decided; not built |
| 6 | 035 unknown record fields | **Refused by default**; a step may declare that they are ignored (dropped, counted, never written); the table schema is never changed. | `specs/035-iceberg-write-steps/research.md` (D035-015) | Decided; not built |
| 7 | 035 snapshot summary identifiers | Effect key, run id, step id, **workflow name and version**; workflow names become visible to readers of the table's metadata. | `specs/035-iceberg-write-steps/research.md` (D035-016, O035-001) | Decided; not built |
| 8 | 035 table creation | **No**: tables exist beforehand; a missing table fails the step. | `specs/035-iceberg-write-steps/research.md` (D035-017) | Decided; not built |
| 9 | 035 step-output destinations | “Quero que o output dos steps também possam ser salvas no iceberg, postgres, etc...” (example: a finished payment flowing down to the lake). Outputs go to destinations **chosen per destination as synchronous** (inside the run) **or asynchronous** (batched export). | `specs/035-iceberg-write-steps/research.md` (D035-018) | Decided; not built |
| 10 | 035 destination kinds | **Iceberg, customer PostgreSQL, object storage files, ClickHouse and broker streams, all five in both modes** (recommendation for synchronous: Iceberg and PostgreSQL only). | `specs/035-iceberg-write-steps/research.md` (D035-019) | Decided; not built |
| 11 | 035/020 placement | Asynchronous export is **020's declared-record export**, which gains an `ICEBERG` sink reusing 035's connection; 035 specifies the Iceberg write step and the synchronous destinations. | `specs/035-iceberg-write-steps/research.md` (D035-020), 020 `research.md` | Decided; 020 is still a draft in PR #53 |

## 2026-10-26

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | Remaining implementation and release target | “Continue com a implementacao do restante agora para a release v0.3”. Complete the eight remaining feature-014 tasks for release 0.3.0 with their original acceptance criteria and required gates. | [0.3.0 implementation ledger](../specs/014-workflow-primitives/implementation-ledger.md) | Implementation in progress; no capacity or release guarantee inferred |

## 2026-10-25

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | Implemented-task integration and release | “Quero que mergeie todas as tasks já implementadas. Crie as versoes e releases necessárias”. Merge and release implemented work; no approval of pending feature-014 production ceilings, general nesting, unverified work or waived gates is inferred. | [ADR-0040](adr/ADR-0040-release-checkpoints-preserve-incomplete-feature-boundaries.md), [0.2.0 notes](releases/0.2.0.md) | Authorized; merge/publication require actual integrated-head gates |

The implementation agent selected one coordinated **0.2.0** under the existing pre-1.0 breaking-minor policy and synchronized release tooling. The owner did not explicitly name this version. GitHub Releases remains the distribution channel; no environment deployment or registry publication is added.

## 2026-10-26 — nested capacity (lane `feature/014-capacity-protocol`)

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | O701, reduction of `iteration_concurrency` | After a reduction (including to 0) already admitted work is **not paused**: it runs to completion and only new admissions are blocked, as the released contract and runbook say. A tenant may therefore exceed a reduced limit while its admitted work lasts. | [capacity-eligibility.md](../specs/014-workflow-primitives/capacity-eligibility.md), [zero policy](../specs/014-workflow-primitives/zero-iteration-policy.md), `research.md` D704/D705 | Explicit zero in the 0.3 working branch; nested group runtime remains pending. O702–O704 are consequences, open to correction |

## 2026-10-16

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | G42, quota values | The current defaults (10,000 active runs per namespace and 50,000 per tenant; 1,000 workflows; 1,000 worker sessions; 500 secrets; 10 GiB content and 1 GiB logs; 100 API requests/s with burst 200; 1,000 events/s with burst 2,000; 30-day events; 365-day scripts) are the **single default tier**. Commercial tiers wait for a billing model; operators keep adjusting per tenant under the ceiling. | `crates/domain/src/quota.rs`, ADR-0027, 009 `research.md` | Done (no behaviour change) |
| 2 | G42/G80, payload classification | **Per field, four levels** (`public`, `internal`, `pii`, `secret`); an unmarked field is `pii`; `pii` needs a new audited permission; `secret` is never shown. | [ADR-0033](adr/ADR-0033-payload-values-are-classified-per-field.md), 004 and 009 `research.md` | Not built (needs its own feature) |
| 3 | SC-008-001 | **N = 10**: a diagnosis passes with at most 9 counted interactions. | 008 `spec.md`, `docs/validation/human-acceptance.md` | Done; the human sessions are still to run |
| 4 | G21, run result | Reaching `end` evaluates `outputExpr`, stores the result (at most 256 KiB, charged to the content quota) in the terminal transaction and gives it to the parent as `steps.<child>.output`; disclosure follows ADR-0033. | 002 `research.md`, workflow schema | Done (`bugfix/run-result`); API disclosure waits for ADR-0033 |
| 5 | G141, late heartbeat | Option (b): the server answers a heartbeat for an attempt the same worker already completed with a distinct non-error status; SDKs unchanged. | 006 `research.md`, `sdks/SPEC.md` §6.4 | Done (`bugfix/late-heartbeat`) |
| 6 | G7, token revocation | **Accepted risk**: tokens live at most 1 hour, roles are read on every request, and suspending a membership is the incident response. | 001 `research.md` | Closed |
| 7 | G9, denied requests | **Every 403 is audited**, bounded by the existing per-subject rate limit. | 001 `research.md` | Done (`access.denied.permission`; `denials::*` in `integration_security_multitenancy`) |
| 8 | D241, RPO/RTO | Reference objectives for the Helm topology with a synchronous standby: **RPO 0** for acknowledged work, **failover RTO ≤ 5 min**, **PITR restore RTO ≤ 1 h**; the drill is to gate a release. An asynchronous standby states its own RPO. | 010 `research.md`, `docs/runbooks/disaster-recovery.md`, `docs/traceability.md` | Objectives published and enforced by the drill, which gates the `release` workflow; not measured on a real cluster |
| 9 | G48, secret delivery | A secret gets an optional list of **worker pools** allowed to receive it; a lease is not handed to another worker and the refusal is audited. | 009 `research.md`, `docs/runbooks/security-governance.md` | Done (see G48 for what a pool is) |
| 10 | G162/G163, Studio | The provisional `S` stays until the owner picks one of the three geometric logos (G162 open). **Scripts follows Workflows** in the navigation (G163). | 007 `research.md`, `frontend/apps/studio/src/app/Shell.tsx` | G163 done; G162 open |
| 11 | T007-049/T007-051 | The owner reviews the visual corrections of 2026-10-12 before accepting them. | — | Waiting for the owner |
| 12 | G123/G203/G216, publication | **GitHub Releases only**: SDK packages and `sagat` binaries with checksums and signatures; no npm, PyPI, NuGet or Maven for now; Go from the GitHub module path. | 006, 011, 012 `research.md`, `docs/runbooks/release.md` | Workflow built (`bugfix/github-releases`); first tagged release still to run |
| 13 | Two drafts numbered 013 | `013-federated-login` keeps 013; `013-otel-openbao` becomes **015** (`FR-015`, `SC-015`) and its draft ADR is renumbered (ADR-0022 is taken; next free after ADR-0033). Its constitution edits are redone on the ratified constitution. | `docs/roadmap.md` | Decided; the drafts live uncommitted in their own worktrees and are renumbered when they are committed |

## 2026-10-17

Answers to the four product questions of the feature 016 specification (per-field payload classification, [ADR-0033](adr/ADR-0033-payload-values-are-classified-per-field.md)). "Not built" means the decision is taken and the implementation is still to be done; it is not evidence.

| # | Item | Decision | Recorded in | Status |
|---|---|---|---|---|
| 1 | 016 P1, how `payload:reveal-pii` is granted | A **per-namespace grant list** managed by administrators (worker-binding and secret-access-list precedent); the six roles stay six and no role holds the permission; grants and revocations are audited. | `specs/016-payload-classification/research.md` (D275), ADR-0033 addendum | Decided; not built |
| 2 | 016 P2, event inbox and dead-letter values | **Restrict now**: `getInboxEvent` `data` and connector dead-letter `body` and `headers` stop being returned to plain `event:read` holders; they need the same reveal grant, everyone else gets structure only; field classification of events waits for event-type schemas. A breaking change for readers of those fields. | `specs/016-payload-classification/` (US08, FR-016-026 to FR-016-029, D302 to D305, G272, G292), ADR-0033 addendum | Decided; not built |
| 3 | 016 P3, readings of ADR-0033 | The **restrictive** reading: an undeclared field is at least `pii` even under a `public` parent; a schema with no annotation is `pii` throughout; a payload with no schema at all is structure only for everyone. | `specs/016-payload-classification/research.md` (D265, D269) | Decided; not built |
| 4 | 016 P4, self-grant | An administrator **may grant reveal to themselves**, audited. | `specs/016-payload-classification/research.md` (D306) | Decided; not built |
| 5 | Self-contained infrastructure | **Everything the platform needs to run or to be tested lives in this repository.** Local and CI infrastructure is declared in `infra/docker/compose.yaml` (and `compose.observability.yaml`), with pinned images, loopback-only ports and development credentials; nothing may depend on a container, volume or service that belongs to another project or has to be started by hand. When feature 015 adopts OpenBao it is added as a service of that compose file (dev mode, pinned image, healthcheck, initialised by a one-shot job), not borrowed from another stack. Integration tests start their own disposable containers. | `AGENTS.md` (infrastructure), 015 draft | Decided; the stack today needs no OpenBao (secrets use the sealed application vault in PostgreSQL) |

## 2026-10-27 — Official Rust SDK

The owner reported the missing Rust SDK during the active0.3 implementation. Add an official Rust
SDK with the current SDK client/worker/builder semantics and shared conformance, tracked by
[feature034](../specs/034-rust-sdk/spec.md) and [ADR-0070](adr/ADR-0070-rust-sdk-shares-the-canonical-sdk-contract.md).
The active0.3 context is the release target; existing Redis033 and remaining014 work stays in scope.

## 2026-10-27 — Redis Streams for release0.3

The owner requested Redis Streams in communications/callbacks now in v0.3 and selected:
“Alternativa ao NATS e transporte adicional para callbacks, selecionável por configuração”.
Feature033 implements optional internal Redis outbox notifications and Redis callback destinations,
retaining PostgreSQL authority, worker gRPC, NATS compatibility and existing HTTP callbacks.
[Specification](../specs/033-redis-streams/spec.md); [ADR-0069](adr/ADR-0069-redis-streams-share-outbox-and-delivery-authority.md).
Implementation/tests remain required; no production deployment, automatic release or inbound Redis
connector is inferred. This explicitly approves dependencies on the existing004/009/017 services.

## 2026-11-01 — Feature 037 planning boundary

The approved B02 normative wording was applied locally after owner approval; formal PR adoption
remains pending. The subsequent [technical plan](../specs/037-compensation-safety/plan.md) proposes
US01–US03 conservative defaults first, with all qualification exceptions disabled and US04 held
for provider evidence and exact contract design. This is a proposal awaiting detailed implementation
approval, not a new owner decision. The revised proposal explicitly removes the current privileged
Retry shortcut for an unknown original effect: an outcome-bearing resolution is required first.
Known-failure retry remains; no new in-place retry-after-resolution is proposed. This stricter
interpretation is part of the design awaiting approval. Existing approvals for S1-B/S2-B and fresh
installation remain.

## 2026-11-01 — Feature 037 delivery 1 approved

The owner answered “Aprovo” to the reviewed implementation plan: preserve review after stop,
refuse direct retry of unknown original/compensation until explicit outcome resolution, and keep
qualification exceptions disabled. This approves T037-001..031 and T037-034. US04, real provider
qualification, deployment, merge and release remain outside this implementation approval.
Implementation uses feature/037-compensation-safety derived from develop, with the approved
uncommitted documentation copied and byte-verified. The original develop checkout remains intact.


## 2026-11-01 — Feature 037 delivery 1 preserves the feature-009 secret-output contract

Asked whether to retain feature-009's final `SECRET_IN_OUTPUT` behavior or extend 037 to preserve
human review for that rejection too, the owner answered: “Manter a regra da 009 e registrar a
limitação nesta entrega”. Delivery 1 therefore changes successful-result **storage refusal** only;
ordinary secret-bearing output keeps the existing feature-009 final-failure behavior. Do not claim
that every output rejection preserves an external-effect recovery obligation. The secret remains
absent from retained output/logs; a source-aware recovery change for this case requires a later
approved cross-feature contract change. Managed-database protections already present are retained.

## 2026-10-29 — Feature 018 approved for implementation

The owner asked to implement the specified features that are still missing and chose **feature 018
(schedules and bulk run operations)** first. The approval covers this feature only; the other fourteen
parity drafts (019–032) of pull request #53 stay unapproved.

- **ADR-0050 accepted.** [ADR-0050](adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md)
  is `Accepted`. Its decisions that rely on the run permit of proposed ADR-0056 (feature 022) remain gated until 022 is accepted.
- **O1000 (a):** a schedule whose authorizing subject loses access is paused with an audited system note at the next action.
- **O1001:** a closed business id is remembered 30 days by default, namespace-configurable up to 366.
- **O1002 (a):** `run:batch` is held by admin and operator only.
- **Dependencies:** only the existing 004/009/017 services and the six SDKs. Features 019, 020 and 022 are not approved,
  so the tasks that need them (T018-037's permit, T018-051, T018-053's permit, T018-055, T018-077, T018-079) stay blocked.
- **Branching:** `feature/018-schedules-bulk-operations` is cut from `develop`; only the 018 directory and ADR-0050 were
  brought from the parity branch, so pull request #53 is not merged by this delivery.

[Specification](../specs/018-schedules-bulk-operations/spec.md); answers in its `research.md` §5; D1040 adds the Rust SDK to its scope.
