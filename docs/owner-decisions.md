# Owner decisions

Decisions that only the product owner could take, with what each one unblocks. Each entry is also
recorded where the open item lived (gap register, ADR, runbook or specification). "Not built"
means the decision is taken and the implementation is still to be done; it is not evidence.

## 2026-11-24 — Windmill-parity readiness of features 021, 023 and 026, decided under the delegation

The session that prepared the Windmill-side parity features (branch `docs/windmill-parity-readiness`) re-read the
approved drafts 021, 023 and 026 against `develop` and took these acts in the owner's place under Owner decisions
2026-11-20-6 and 2026-11-24-4. Each is recorded with its options and reasoning where it lives; the rows below are the
index. They are delegated acts, reported to the owner, who may overrule any of them.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-24-300 | Architecture decision 0055 accepted with its amendment of 2026-11-24 and the new clause 11(e), which admits design 047's control-plane token broker as the placement for token requests | delegate (OD-2026-11-24-4) | Decided | docs/adr/ADR-0055-integration-resources-and-inbox-connector-kinds.md |
| OD-2026-11-24-301 | Feature 021's owner questions: 1120, a generic relay plus Mailgun first; 1121, developers hold `resource:effect` by default; 1122, no namespace-defined resource types, curated hub types through the provenance extension point; 1123, clauses (a) to (d) plus (e) | delegate (OD-2026-11-24-4) | Decided | specs/021-integration-resources-triggers/research.md |
| OD-2026-11-24-302 | Feature 021's requirements checklist: "written for non-technical stakeholders" and "technology-agnostic success criteria" accepted as stated deviations; the owner-answer and ADR items closed by the two rows above | delegate (OD-2026-11-24-4) | Decided | specs/021-integration-resources-triggers/checklists/requirements.md |
| OD-2026-11-24-303 | Architecture decision 0057 accepted with its amendment of 2026-11-24 (bundle versions, closed catalogs narrowed by Architecture decision 0120 for TypeScript and Python, lake mode, provenance and SBOM through design 046, fleet activation, warm containers) | delegate (OD-2026-11-24-4) | Decided | docs/adr/ADR-0057-additional-script-languages-reuse-the-sandbox-and-signed-artifacts.md |
| OD-2026-11-24-304 | Feature 023's owner questions: 1200, a reviewed starter catalog per language of at most 32 root packages in 0.3.0; 1201, every built language enabled by default; 1202, Deno and Bun not adopted, the spike kept for the benchmark; the capability `script-languages-v1` gates the new languages | delegate (OD-2026-11-24-4) | Decided | specs/023-script-languages/research.md (Decisions 1234 and 1626 to 1627) |
| OD-2026-11-24-305 | Feature 023's requirements checklist: "no implementation details" (two items) and "technology-agnostic success criteria" accepted as stated deviations; "written for non-technical stakeholders" rejected and fixed by an "In short" summary; the ADR, coordinator-integration and migration items closed | delegate (OD-2026-11-24-4) | Decided | specs/023-script-languages/checklists/requirements.md |
| OD-2026-11-24-306 | Architecture decisions 0061 and 0068 accepted with their amendments of 2026-11-24 (the public-link run marker, the run-input gate, the attempt fence instead of the step lock, a storage class of its own, the purge pass) | delegate (OD-2026-11-24-4) | Decided | docs/adr/ADR-0061-link-bearers-hold-one-narrow-capability-and-never-decide.md, docs/adr/ADR-0068-workflow-streams-are-a-fenced-classified-log-beside-the-journal.md |
| OD-2026-11-24-307 | Feature 026's owner questions: 1320, no decision by holding a link in 0.3.0; 1321, public links off by default everywhere through a quota of 0; 1322, channel messages carry personal data only under feature 020's rule, gated until Owner question 1081 is answered | delegate (OD-2026-11-24-4) | Decided | specs/026-invocation-human-surfaces/research.md |
| OD-2026-11-24-308 | Feature 026's requirements checklist: three items accepted as stated deviations; "written for non-technical readers" rejected and fixed by an "In plain words" section | delegate (OD-2026-11-24-4) | Decided | specs/026-invocation-human-surfaces/checklists/requirements.md |
| OD-2026-11-24-309 | Boundaries: design 046 owns and builds first the script policy table, the lease-time block check and the provenance and SBOM module, which feature 023 extends; feature 021's OAuth scheme names a design 047 credential and is refused until design 047's broker is on | delegate (OD-2026-11-24-4) | Decided | specs/046-script-dependencies/research.md (Decision 046-033), specs/023-script-languages/research.md (Decision 1627), specs/021-integration-resources-triggers/research.md (Decision 021-001) |

## 2026-11-24 — Release 0.3.0 contains everything that is missing to surpass Temporal and Windmill

The coordinating session compared the platform with Temporal and Windmill (gaps that have a specification, gaps that have
none, work in progress, performance). The owner answered: "Vamos terminar tudo que falta. Inclua tudo na 0.3.0. Quero que
os melhores desenvolvedores e arquitetos façam o melhor para superarmos o windmill e temporal."

In English: let us finish everything that is missing; include all of it in 0.3.0; the best developers and architects are
to do their best so that we surpass Windmill and Temporal.

Asked the two things the delegation of OD-2026-11-20-6 does not cover, the owner chose the recommended option of each:
the constitution is amended to remove what blocks the scope, and workflow as code enters 0.3.0.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-24-1 | "Inclua tudo na 0.3.0": release 0.3.0 contains every approved draft (019 to 032, 036), the rest of 022 beyond its throughput slice, the trigger operators of 042 (slices 2 to 5, which OD-2026-11-13-15 had left for a later release), designs 038, 039 and 040, the 10,000-item collection and loop ceilings that OD-2026-11-13-1 deferred (published only after the acceptance run on a separate database host), and new designs for the gaps that have no specification: workflow as code, activity timeouts and non-retryable errors, package dependencies for scripts, OAuth resources and an integration hub, a reference high-availability topology with rolling N and N-1 upgrades, partitioning of the journal tables, the Studio catch-up, federated login (013), OpenTelemetry and OpenBao (015) and a published benchmark against Temporal and Windmill. Because they are now 0.3.0 features, their open questions fall under OD-2026-11-20-6. The release is published only when all of it is built and the release-candidate run passes on that commit (OD-2026-11-04-8) | owner | Decided; not built | docs/roadmap.md, docs/releases/unreleased.md, every feature named |
| OD-2026-11-24-2 | "Aprovo a emenda": the constitution is amended (MAJOR, 5.0.0) to remove what blocks OD-2026-11-24-1: "reimplementing Temporal" leaves the out-of-scope list, "single region" becomes one writable region per cell with fenced failover (032), low-code internal apps (027) and a curated integration and template hub (021, 028) are admitted, and the declarative IR is joined by a procedural code-first mode (OD-2026-11-24-3). The coordinating session drafts it with its Sync Impact Report, the three repository reviewers read it, and it merges without returning to the owner unless a reviewer finds a Blocker | owner | Decided; amended in 5.0.0 (pull request #275) | docs/constitution.md, docs/architecture/overview.md, docs/governance/constitution-5.0.0-sync-impact.md |
| OD-2026-11-24-3 | "Sim, entra na 0.3.0": workflow as code (a workflow function in an SDK, replayed deterministically against the journal, Temporal's programming model) is built in 0.3.0 in all six SDKs, under a new design that starts with a determinism study per language before any code. The declarative graph stays a first-class model beside it | owner | Decided; not designed | a new design doc in docs/designs/, docs/architecture/workflow-ir-and-sdk-modes.md |
| OD-2026-11-24-4 | "Quero que os agents continuem por mim. Tomem a melhor decisão" (the agents carry on in the owner's place and take the best decision). This widens OD-2026-11-20-6 for release 0.3.0: besides the open questions, the coordinating session and the agents it briefs now also accept or reject ADRs, settle contested or unfixed review findings in writing, accept or reject unticked `checklists/*.md` items (OD-2026-11-03-33), approve design docs once their review is complete, and merge pull requests into `develop` once their CI is green. Each such act is weighed over every option, decided for the most architectural and business advantage (never for simplicity alone), recorded where it lives as "delegate (OD-2026-11-24-4)" with its reasoning, and reported to the owner afterwards. Read by the coordinating session as still the owner's: acts that need the owner's own hands or credentials (for example the `.env*` files the harness refuses to agents), spending money or creating accounts, publishing release 0.3.0 (the tag, the GitHub release and the merge into `main`) and any act outside this repository that cannot be undone | owner | Decided | every 0.3.0 feature, ADR and design doc; docs/constitution.md (approval and acceptance clauses) |

## 2026-11-21 — Design 041 (step logs): the third round of the approval run

After the third round of short re-reads of design 041's approval run (OD-2026-11-20-17; the reports in
`docs/designs/041-step-logs.reviews/2026-11-20-approval/round-3/`) the coordinating session put three questions to the
owner. Each row gives the question as asked, then the answer, then what it means. The final confirmation read of the
fixes (`docs/designs/041-step-logs.reviews/2026-11-20-approval/final/`) left six Majors and no Blocker; each is accepted
under OD-2026-11-20-17's cap by one row from OD-2026-11-21-200 upward, in the range reserved for design 041. After the
approval the coordinating session put three more questions (OD-2026-11-21-8 to -10).

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-21-1 | Asked: "041 Blocker: at 8×128, the eight valid rounds of the tail gate (2b) were -312.6 / -25.4 / -17.0 / -2.2 / +6.3 / +11.5 / +24.7 / +112.9 ms over the allowance. As a regression gate for later runs, the build you accepted would pass only about half the time. How should the 8×128 gate be set?" Answer: "Derived floor, like 1×16". The 8 × 128 bar of gate (2b) comes from the fourth run's measured distribution plus a tolerance derived from its spread, the same method as at 1 × 16, fixed in the document before any later round: the median of the three rounds used, −17.0 ms, and the worst valid round, +112.86 ms, give a tolerance of 129.9 ms and a bar of at most +112.9 ms over the allowance. It relaxes the allowance of OD-2026-11-20-1 at this size. The derivation and the false-stop rates (at most 3.85% per gate with seven to twelve valid rounds) are in SC-041-007 and decision D041-136 | owner | Decided | O041-045, D041-136 of specs/041-step-logs/research.md; docs/designs/041-step-logs.md SC-041-007, T041-9; docs/runbooks/step-logs.md |
| OD-2026-11-21-2 | Asked: "041 Z-3: ADR-0095 decision 5 (accepted) keeps a re-run tier when a guard raises during a carry. Reviewers recommend keeping it as is, with an owner row, rather than removing it. Keep it?" Answer: "Keep it". The re-run tier of ADR-0095 decision 5 stays: a guard's raise during a carry re-runs the report once with a mark-only carry whose reason is `failed`. This closes review finding Z-3 | owner | Decided | O041-046 of specs/041-step-logs/research.md; docs/adr/ADR-0095 decision 5; docs/designs/041-step-logs.md section 2a, T041-36; docs/designs/041-step-logs.reviews/2026-11-20-approval/zoom_out_report.md (Z-3) |
| OD-2026-11-21-3 | Asked: "041 I-6: the 0.1% busy-loss reopening threshold (OD-2026-11-20-14) is counted in every measured regime except the two fault-injection regimes that hold the usage row past 200 ms on purpose. Is that the right reading?" Answer: "Yes, exclude those two". The threshold of OD-2026-11-20-14 counts every regime task T041-37 measures except the fan-out's held rows and tenant A of the two-tenant regime, whose busy answers are induced; the tenant beside them counts. This closes review finding I-6 | owner | Decided | O041-047, D041-131, D041-137 of specs/041-step-logs/research.md; docs/designs/041-step-logs.md T041-37; docs/designs/041-step-logs.reviews/2026-11-20-approval/intermediate_zoom_out_report.md (I-6) |
| OD-2026-11-21-8 | Asked: "041: you set a derived floor for the tail gate (2b) at 8×128. Gates (1) rate and (3) `{}` rate at 8×128 still use the fixed 90%, and they contribute about 15 points of a ~34% chance (at 8 valid rounds, an upper bound) that an unchanged build fails some gate in a later run. Apply the same derived floor to (1) and (3) at 8×128?" Answer: "Derive them too". Gates (1) and (3) at 8 × 128 move from the fixed 90% to floors derived by the method of OD-2026-11-21-1 (decision D041-136) from the fourth run's eight valid rounds: the median of the three rounds used to the worst valid round, the floor that worst round rounded in the build's favour. Gate (1): median 93.1%, worst valid round 88.63% (round 11), tolerance 4.5 points, at least 88.6%. Gate (3): median 91.5%, worst valid round 83.39% (round 9), tolerance 8.2 points, at least 83.3%. Every gate but (4) at 8 × 128 (bar 10, twice its worst round) now sits at its worst valid round, so an unchanged build fails one with at most 3.85% per gate and the release, over the seven, with at most 7 × 70/1,820 = 26.9% at eight valid rounds and 16.7% at twelve (a union bound, no longer an estimate). Written in design 041's follow-up task T041-38, since the approved body of SC-041-007 cannot change | owner | Decided | O041-048, D041-138 of specs/041-step-logs/research.md; docs/designs/041-step-logs.md T041-38 (amending SC-041-007 and T041-9); specs/041-step-logs/validation-report.md (fourth run); docs/runbooks/step-logs.md |
| OD-2026-11-21-9 | Asked: "041: the agent amended ADR-0097 in place with decisions D041-134 and D041-137 (how 'top up after the commit', OD-2026-11-20-18, is built: the transaction owns its connection, rolls back and returns it on drop, the top-up is the last statement before the connection goes back). Confirm?" Answer: "Confirm the amendments". ADR-0097's delegate amendments by D041-134 (decisions 1, 3b and 4) and D041-137 (decisions 3b, 4 and 8) stand as the owner's | owner | Decided | O041-049 of specs/041-step-logs/research.md; docs/adr/ADR-0097 (header and the amended passages) |
| OD-2026-11-21-10 | Asked: "041: you excluded the two fault-injection regimes from the 0.1% busy-loss rule (OD-2026-11-21-3). The agent also excluded tenants A1 to A3 of the three-tenant regime, held the same way on purpose. Confirm?" Answer: "Confirm". Tenants A1, A2 and A3 of task T041-37's three-tenant regime, whose usage rows are held on purpose, are not counted by OD-2026-11-20-14's 0.1% threshold; tenant B beside them is | owner | Decided | O041-050, D041-137 of specs/041-step-logs/research.md; docs/designs/041-step-logs.md T041-37; docs/designs/041-step-logs.reviews/2026-11-20-approval/resolution.md |
| OD-2026-11-21-200 | "Full review, capped" (OD-2026-11-20-17): the final read's stages 5 to 8 finding 1 of design 041's approval run, the owner report on the p99 judged against the largest of the three rounds used while its statistic is the median of all valid rounds (so it fires on the accepted build), is accepted as is (the report does not stop the release; task T041-9 records the eight valid rounds' figures from which a threshold of the same statistic is derived) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-20-approval/final/egm-review.md#1 |
| OD-2026-11-21-201 | "Full review, capped" (OD-2026-11-20-17): the final read's stages 5 to 8 finding 2 of design 041's approval run, the pooler's server-pool sizing rule (Σ (⌊N / 2⌋ + *Q*) plus one a replica) that leaves little room for multiplexing and moves task T041-37's pooler rows from 40 to 128 server backends, is accepted as is (the residual, heavy multiplexing behind a transaction pooler, goes to the owner with task T041-37's record) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-20-approval/final/egm-review.md#2 |
| OD-2026-11-21-202 | "Full review, capped" (OD-2026-11-20-17): finding D-12 of design 041's approval run (first round; the final read's stages 5 to 8 finding 3), the revision notes kept in the Overview as the amendment rule requires, is accepted as recommended | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-20-approval/final/egm-review.md#3 |
| OD-2026-11-21-203 | "Full review, capped" (OD-2026-11-20-17): finding Z-2 of design 041's approval run (first round, stage 5), the exact per-writer quota leases kept over a simpler unlocked ceiling with a proved overshoot (ADR-0097 option 4), is accepted as recommended (the leases kept, OD-2026-11-20-15; Complexity Tracking names the rejected alternative) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-20-approval/final/egm-review.md#4 |
| OD-2026-11-21-204 | "Full review, capped" (OD-2026-11-20-17): the architecture reviewer's final-read finding 1 of design 041's approval run, the same pooler sizing rule (written with the default Logs cap rather than the configured one, not cited by `docs/runbooks/scaling.md`, and changing what the pooler rows measure), is accepted as is, with OD-2026-11-21-201 | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-20-approval/final/architecture.md#1 |
| OD-2026-11-21-205 | "Full review, capped" (OD-2026-11-20-17): the correctness reviewer's final-read finding 1 of design 041's approval run, the same owner-report threshold as OD-2026-11-21-200, is accepted as is, with that row | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-20-approval/final/correctness.md#1 |

## 2026-11-20 — Design 041 (step logs): the fourth carry spike, the busy loss, ADR-0095, ADR-0097 and the amendment of ADR-0027

After task T041-35's fourth run (run 44834024933, decision D041-122 of `specs/041-step-logs/research.md`; the record in
`specs/041-step-logs/validation-report.md`) the coordinating session put four questions to the owner. Each row gives the
question as asked, then the answer, then what it means.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-20-13 | Asked: "041 step logs, gate at 1×16: on a saturated 4-core runner, capturing step logs costs about 18% of the rate (82%) and +8.6 ms of p99 over the allowance, while 8×128 passes every gate. How should SC-041-007 be settled?" Answer: "Accept the measured cost". SC-041-007 records the measured figures per size of the fourth run (decision D041-122): at 1 × 16 the rate of workload (a) 82.0% of capture off, gate (2b) +8.6 ms over the allowance, the rate of workload (f) 87.1%, 7 tickets; at 8 × 128 every gate passes. The 1 × 16 cost is documented in the step-log runbook and the release notes. Task T041-35 is ticked on this decision, with the fourth run as its evidence. This is the answer to the environment question of decision D041-109, so task T041-5 no longer waits for it | owner | Decided | docs/designs/041-step-logs.md T041-35, T041-5, SC-041-007; D041-109, D041-122 of specs/041-step-logs/research.md |
| OD-2026-11-20-14 | Asked: question O041-041, "When a carry's fallback refill cannot get the tenant's usage row within its 200 ms budget, what should happen to its records (gap G041-026)?" Answer: "Accept the visible loss": option (a), as built (decision D041-119). The records are dropped and counted `tenant_quota_busy`, the stream and Studio say the log service was busy, an alert fires on its rate, and the report always lands. The reopening threshold already declared stands: the question is put again if carries answered `mark_only:busy` exceed 0.1% of carries in any regime that task T041-9 measures | owner | Decided | O041-041, G041-026, D041-119, D041-121 of specs/041-step-logs/research.md; docs/designs/041-step-logs.md T041-9 |
| OD-2026-11-20-15 | Asked: "Accept ADR-0097 and ADR-0095?" Answer: "Accept both". ADR-0097 (the step-log quota charged to exact per-writer leases, with its residual gaps G041-026 and G041-027) and ADR-0095 (the last step-log records of an attempt ride on its report) are Accepted on 2026-11-20 in the text of this branch; ADR-0084 and ADR-0027 are marked partly superseded by them. This answers questions O041-042 and O041-038 | owner | Decided | O041-042, O041-038 of specs/041-step-logs/research.md; docs/adr/ADR-0097, docs/adr/ADR-0095, docs/adr/ADR-0084, docs/adr/ADR-0027; docs/designs/041-step-logs.md T041-3, T041-5, T041-36 |
| OD-2026-11-20-16 | Asked: question O041-040, "Accept the wording of the proposed amendment of ADR-0027's Amendment (step-log rows charged once per insert statement, one flush per transaction, release per row)?" Answer: "Amend ADR-0027", option (a): ADR-0027's Amendment gains the statement-level measurement, with the charge going to the writer's lease as ADR-0097 states it, and the per-row release, as a section of ADR-0027 (an exception to `docs/adr/README.md` rule 2, as OD-2026-11-15-16 granted for the first Amendment); the deferred flush on insert is replaced by ADR-0097 | owner | Decided | O041-040 of specs/041-step-logs/research.md; docs/adr/ADR-0027 (Amendment of the Amendment, 2026-11-20); docs/designs/041-step-logs.md T041-5, T041-36 |
| OD-2026-11-20-17 | Asked: "How should design 041's revision be approved? Full review, capped: run the full egm-review on the changed text plus the three repository reviewers at the final commit, and if Majors remain after the first round, apply the approval path of OD-2026-11-16-7 (fix the 'fix' items, short re-read, accept the rest as recommended). Or: accept the rereview-2/rereview-3 closure instead of the egm-review." Answer: "Full review, capped". The egm-review runs on the changed line range of the design (since the approved commit `f9d4a65f`) and the architecture, security and correctness reviewers on the diff, all at the final commit; every Blocker and Major is answered, documents first; if Majors remain after that first round, the findings the reviewers mark "fix" are fixed and re-read briefly, and the rest are accepted as recommended, one row per accepted finding citing this row; a Blocker is never accepted, only fixed | owner | Decided | docs/designs/041-step-logs.md (the approval of its revision of 2026-11-19 and 2026-11-20); docs/designs/constitution-4.0.0-egm.md section 4 |
| OD-2026-11-20-18 | Asked: "041 Blocker A-1: the store can't learn which database connection (backend) a pooled sqlx connection is without running a statement, so the 'top up the lease before the report' step (ADR-0097 decision 3b, which you accepted) can't work as written. How should it change?" Answer: "Top up after the commit". After a carrying report commits, the same connection tops up its lease from the room that carry returned, off the run lock; no backend id is needed in Rust; there is no wait before the report. ADR-0097 decision 3b is amended accordingly (the ensure before the report and the store's per-connection room cache are removed; the budget table is restated), and ADR-0095 decision 4 (capture never makes a report late) holds again, with a Note on the fallback refill's bounded wait; this also answers review findings Z-1 (drop the ensure) and I-3, and Major C-6 | owner | Decided | O041-043, D041-124 of specs/041-step-logs/research.md; docs/adr/ADR-0097 decision 3b, docs/adr/ADR-0095 decisions 2 to 5; docs/designs/041-step-logs.md sections 2, 2a and 2b, T041-5, T041-36, T041-37 |
| OD-2026-11-20-19 | Asked: "041 Major A-2 (constitution principle V, gap G041-018): a reader with the logs permission but without a namespace's PII grant can watch the tenant's step-log quota usage move, a side channel on what other runs log. How should it be closed?" Answer: "Hide it without the grant". Step-log quota usage is shown only to callers holding the PII grant; others see that a quota exists, not its live usage (fail-closed): `used` of `tenant_step_log_bytes` needs `run:logs:read` and the PII reveal grant in every namespace of the tenant. Gap G041-018 and review finding D-4 are closed by it; the contract and Studio follow, and a test shows that a caller without the grant gets no usage figure | owner | Decided | O041-044, G041-018, D041-133 of specs/041-step-logs/research.md; specs/041-step-logs/contracts.md (Quota.used); docs/designs/041-step-logs.md T041-3, T041-4, T041-39 (D041-141, the grant and the usage in one snapshot) |


## 2026-11-20 — Documentation first, words in full, identifiers as words

The owner gave three instructions on 2026-11-20, one after the other. They are amended into the constitution together
(version 4.7.0) and carried out by [`docs/governance/identifier-scheme.md`](governance/identifier-scheme.md).

First: "Adicione nova regra. Quero que o docs seja extremamente organizada e escrita a humanos. Ali estará toda
docuentação, especificação, diagramas etc... Mantenha sempre atualizado. Qualquer alteração ou nova regra, sempre
atualizar o documento antes do código. O código deve se guiar pelo documento e nao o contrario"

In English: `docs/` is to be extremely well organized and written for people; it holds all the documentation, the
specifications and the diagrams; it is always kept up to date; any change or new rule updates the document before the
code; the code follows the document, never the reverse.

Second: "Incrementar regra. Nas documentações e comentários de código, evitar siglas como como R para Regra, T para
Test. Sempre usar completo. Rule, Test e assim por diante"

In English: extend the rule; in documentation and code comments, avoid abbreviations such as R for Rule or T for Test;
always write them in full: Rule, Test and so on.

Third, asked whether the stable identifiers (FR-022-035, T022-064, ADR-0097 and the others) are renamed too or kept:
"Rename everything".

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-20-8 | A new rule, "Documentation first": the documents (constitution, architecture decision records, design doc or spec, contracts, runbooks, reference) are the source of truth. A change of behaviour or a new rule is written in its document first and the code then follows it, in the same pull request or an earlier one; when code and document disagree, the wrong one is fixed and they are never left apart. `docs/` is organized and written for people: plain language, an index in every folder, diagrams where they help. Enforced on every pull request by `scripts/check-docs-first.py`: a pull request whose own commits change behaviour code and no document fails, unless each such commit says why no document changes (`Docs: none, <reason>`). Commits written before the rule existed are not judged. Moving `specs/` under `docs/` is planned separately and done after the large open feature branches merge | owner | Decided | docs/constitution.md 4.7.0 (Documentation first), docs/governance/constitution-4.7.0-sync-impact.md, AGENTS.md rule 23, scripts/check-docs-first.py, .github/workflows/ci.yml (job `documentation`) |
| OD-2026-11-20-9 | "Words in full": documents and code comments write words in full ("Rule 8", "test", "Blocker 1", "Major 2", "minor 7", "slice 5", "mutation 3"), never the abbreviations `R8`, `T` or `UT`, `B1`, `M2`, `m7`, `S5`, `W3`. Read by the coordinating session as: code, paths, literal output and quotations stay as they are; the abbreviations that are not identifiers are refused from now on in the lines a pull request adds, by `scripts/check-plain-words.py`, whose list of patterns is curated from the abbreviations actually used in this repository and requires the words around them (a reviewer, "rule", "slice", "mutation"), so that proper names such as Amazon S3, Cloudflare R2 or an Apple M3 are not refused. The repository's own tools follow it: `scripts/check-design-docs.py` prints "Rule 8", not "R8". The owner may correct this reading | owner | Decided | docs/constitution.md 4.7.0 (Words in full), docs/governance/identifier-scheme.md, AGENTS.md rule 24, scripts/check-plain-words.py, scripts/check-design-docs.py |
| OD-2026-11-20-10 | "Rename everything": the stable identifiers become words, a space and the number. The scheme, chosen by the coordinating session under the delegation (OD-2026-11-20-6) and open to the owner's correction before the sweep: Task 022-064 (was T022-064), Requirement 022-035 (FR-), Success criterion 022-012 (SC-), Non-functional requirement 010-001 (NFR-), User story 022-02 (US02, always with its feature, so that it is unique and has the shape of the other kinds), Decision 041-119 (D), Gap 022-015 and Gap 708 (G), Owner question 041-040 (O), Architecture decision 0097 (ADR-0097), Owner decision 2026-11-20-10 (OD-); commit footers `Refs: Task 022-064, Requirement 022-035`. The file names of the architecture decision records (`docs/adr/ADR-NNNN-slug.md`) do not change: a path is not prose, and links from outside the repository (release notes, pull request descriptions, commit messages) cannot be rewritten. Past commits keep the old forms; `docs/governance/identifier-scheme.md` maps old to new. Until one mechanical sweep renames the existing text and the tools that parse it (after #237, #241 and #226 merge, before release 0.3.0, planned with the move of `specs/` under `docs/`), footers and new prose use the new forms and the places a tool reads keep the old ones | owner | Decided | docs/constitution.md 4.7.0 (Identifiers), docs/governance/identifier-scheme.md, AGENTS.md rule 15 and "Start here" |

## 2026-11-20 — The coordinating session decides the open questions until release 0.3.0

The owner said: "Quero que fique no loop e não pare até terminar todo trabalho. Nas decisões avalie todas as
possibilidades e escolha sempre a que dá vantagem arquitetural e de negócio, e não selecione a mais simplista só por conta
de simplicidade. Quero que o código seja altamente flexível, coeso, adaptável, e sem erros de cálculo principalmente."

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-20-6 | The open product, scope and design questions of release 0.3.0 are delegated to the coordinating session. Each is weighed over every option and decided for the most architectural and business advantage, never for simplicity alone. Each is recorded as "delegate (OD-2026-11-20-6)" with its reasoning where the question lives, and reported to the owner afterwards. Not covered: acts that need the owner's own hands or credentials, and acts outside this repository that cannot be undone | owner | Decided | every open O-question of the 0.3.0 features; AGENTS.md "Only the owner decides" |
| OD-2026-11-20-7 | "Yes, merge #254" (owner, 2026-11-20, asked because this narrows the owner's OD-2026-11-06-2 and amends the constitution, which the delegation does not cover). Proposed on architecture review finding m7 of design 022: a pull request that changes only a contract file (`specs/<feature>/contracts/`) selected no suite, so a contract regression could merge untested. A contract change is a change: it now selects the contract suites (`tests/contract/`) and the workspace packages whose Rust sources read that file (and the packages that depend on them), derived by `scripts/select-test-targets.py` from the sources, never from a hand list. Weighed: every reader including the integration and e2e suites (an OpenAPI change would run twenty suites, five of them integration or e2e, against the delivery time OD-2026-11-04-1 protects); a hand table (rots when a suite is added); nothing (the hole stays). This narrows OD-2026-11-06-2 for contract files only; every other shared input still selects no suite. Reversible by a later amendment; to be reported to the owner | owner | Decided | docs/constitution.md 4.6.0, docs/governance/constitution-4.6.0-sync-impact.md, scripts/select-test-targets.py, docs/test-strategy.md |

## 2026-11-20 — Design 041 (step logs): the carry's tail after its first spike

The spike T041-35 of design 041 passed its rate and ticket gates and failed the completion p99 (+40.6% and +23.7% at the
medians, against +10%; `specs/041-step-logs/validation-report.md`). The assistant asked O041-039
(`specs/041-step-logs/research.md`).

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-20-1 | O041-039, "Cortar custo, depois limite absoluto" ("cut the cost, then an absolute limit"): (a) cut the carry's cost (its statements in one round trip, and the step-log quota charged once per transaction instead of per row, an amendment of ADR-0027's Amendment whose wording is put to the owner as O041-040) and run the spike again; (b) if it still misses +10%, the p99 criterion for completions that carry records is the capture-off p99 plus 2 ms, at both sizes, fixed before the rounds, the other gates unchanged | owner | Decided | docs/designs/041-step-logs.md T041-5, T041-35, T041-36 |
| OD-2026-11-20-5 | "Confirmo" ("I confirm"), before any round of the second carry spike: the coordinator's reading of OD-2026-11-20-1 (D041-101) holds; per size, a carrying completion's p99 must be at most the capture-off p99 plus the larger of 10% of it and 2 ms, and the report says which allowance decided each size | owner | Decided | docs/designs/041-step-logs.md T041-35 |

## 2026-11-19 — Design 041 (step logs): the redesign of the transport after the ingest spike

The spike T041-31 of design 041 failed SC-041-007 on a quiet machine: with an `AppendStepLogs` transaction per batch and
a final batch before every report, the rate with capture on was 79% and 87% of the rate with capture off (medians of
three rounds, criterion 90%), and 89.4% and 89.9% with the lines in the final batch itself
(`specs/041-step-logs/validation-report.md`). The assistant asked O041-036 (`specs/041-step-logs/research.md`) with five
options.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-19-1 | O041-036, "Redesenhar (CompleteStep)" ("redesign (CompleteStep)"), option (d): cut what a step pays; the lines travel in the step's own `CompleteStep` (and `FailStep`) so that a short step needs no extra transaction, a long step still streams before its completion, and a new spike with the same criteria (rate at least 90%, completion p99 within +10%, tickets within half the pool) runs before the schema. O041-037 follows from it (T041-3 and T041-4 stay held; T041-3's 1.7 contract changes), recorded as decision D041-091 | owner | Decided | docs/designs/041-step-logs.md T041-3, T041-4, T041-5, T041-6, T041-8, T041-9, T041-14, T041-15, T041-16, T041-17, T041-18, T041-19, T041-20, T041-32, T041-35, T041-36 |

## 2026-11-19 — Design 043: the last three owner questions

Asked together with the other open recommendations on 2026-11-19 ("Há seis decisões pendentes, todas com recomendação
… Aceitas todas como recomendado?"); the owner answered "Sim, como recomendado". The three of design 043 below; the
three of feature 022 are recorded on its branch.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-19-5 | O043-005 (a), "Sim, como recomendado": an `inputSchema` with tuple positions (`prefixItems`, or `items` as an array) stays outside the supported subset in 0.3.0; in `enforce` such a version is refused at publication and refuses its starts, the inventory lists it, `report` publishes it, the 016 contract and the release notes say so, and tuples are designed into the subset later | owner | Decided | O043-005, D043-095, D043-110, specs/043-run-input-validation/research.md |
| OD-2026-11-19-6 | O043-006 (a), "Sim, como recomendado": a repeat, trigger-now or private rerun refused on its input answers `/` and `other` to every caller, as built; no graded disclosure in 0.3.0 | owner | Decided | O043-006, D043-121, D043-144, D043-148, G043-040 |
| OD-2026-11-19-7 | O043-008 (a), "Sim, como recomendado": ADR-0092 decision 6 is accepted: a release that makes the subset stricter ships `run-input-validation-v2`, required by the operator as v1 is, never a silent edit | owner | Decided | O043-008, ADR-0092 decision 6 |

## 2026-11-16 — Design 041 (step logs): the approval path and the answers to Q1 to Q8

After the third approval run of design 041 the assistant listed every open finding with a question to the owner
(`docs/designs/041-step-logs.reviews/2026-11-15/resolution.md` section 11; O041-028 to O041-035 in
`specs/041-step-logs/research.md`). The owner chose an approval path and answered Q1 to Q3; Q2 goes against the
recommendation (OD-2026-11-16-9). The acceptance of each remaining finding is one row from OD-2026-11-16-200 upward, in
the range reserved for design 041.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-16-7 | "Corrigir e aceitar o resto" ("fix and accept the rest"): fix the Blocker (T041-25's claim about how CI runs the alert rule tests) and the defects F1 to F15 of resolution.md section 11, including the final batch's wait for a ticket and the scan bound that ended a filtered read early; Q4 to Q8 accepted as recommended (the script marker in its own transaction after the report commits; the ingest reads the binding and the membership without the namespace secrets lock; O041-025 restated to the code's stop rule; G041-017 accepted in writing; a Note on ADR-0085); then one short re-read of the fixes by the architecture, security and correctness reviewers, with no full review round; every other remaining finding of the third run accepted as recommended, one row each | owner | Decided | docs/designs/041-step-logs.md T041-6, T041-7, T041-13, docs/adr/ADR-0084-*.md, docs/adr/ADR-0085-*.md, docs/adr/ADR-0027-*.md (Note of 2026-11-16) |
| OD-2026-11-16-8 | "Sim, sempre" ("yes, always"): every step-log read needs `run:logs:read` and the namespace's PII reveal grant, in every namespace, and so does the `log` of a run's script execution; this changes OD-2026-11-13-18, which required the grant only in a namespace with classified schemas | owner | Decided | docs/designs/041-step-logs.md T041-11, T041-12, T041-13, T041-22, T041-29, docs/adr/ADR-0085-*.md |
| OD-2026-11-16-9 | "Configuráveis" ("configurable"), against the recommendation of constants: the attempt ceilings stay configurable per namespace, and the per-namespace capture switch is kept | owner | Decided | docs/designs/041-step-logs.md T041-4 |
| OD-2026-11-16-10 | "Retirar" ("remove"): the run ceiling is dropped entirely; the attempt ceilings and the step-log quota bound the total; this answers O041-020 and O041-027 | owner | Decided | docs/designs/041-step-logs.md T041-4, T041-5, T041-6, T041-10 |
| OD-2026-11-16-11 | O041-025, "Sim, como os reports" ("yes, as the reports"), option (a) as recommended: a step-log batch is accepted when a report of its attempt would be (the step not cancel-requested, the run not finished, the run not stopping unless the node is a compensation or reconciliation node), one domain function `report_fence_allows` used by `attempt_authority_fence` and by the ingest's lease read (T2); and the shared policy fences of the namespace's step-log resources come first in the lock order (L1, before the stream row), as ADR-0027 asks of every authoritative limit read. The owner gave the answer on 2026-11-16 to the coordinating session, which relayed it, with the words quoted here, to the session that builds design 041 on 2026-11-18 | owner | Decided | docs/designs/041-step-logs.md T041-33, T041-6, T041-34; specs/041-step-logs/research.md O041-025; ADR-0084 Revision Note of 2026-11-18 |
| OD-2026-11-16-200 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 5 Z3-1 finding of the third approval run of design 041, four namespace-tunable ceilings and the capture switch as a ceiling of 0, is accepted as is (kept as the owner decided: "Configuráveis", OD-2026-11-16-9; accepted as is) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#1 |
| OD-2026-11-16-201 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 5 Z3-2 finding of the third approval run of design 041, the exact run ceiling (a table, a lock, two owner questions), is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: no run ceiling, "Retirar", OD-2026-11-16-10) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#2 |
| OD-2026-11-16-202 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 5 Z3-3 finding of the third approval run of design 041, the script marker inside record_execution's transaction, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: the marker in its own transaction after the report commits (Q4)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#3 |
| OD-2026-11-16-203 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 5 Z3-4 finding of the third approval run of design 041, the Technical Plan written at statement level, is accepted as is (accepted as is (F15; stage 8 rejected it as a Major)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#4 |
| OD-2026-11-16-204 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 6 C3-1 finding of the third approval run of design 041, ADR-0085 describes mechanisms the design dropped, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: a Note in ADR-0085 (Q8)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#5 |
| OD-2026-11-16-205 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 6 C3-2 finding of the third approval run of design 041, the state of a script item with no marker stream, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: the state comes from the execution (F5)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#6 |
| OD-2026-11-16-206 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 K3-1 finding of the third approval run of design 041, the ingest's secrets lock delays leases, heartbeats and database admission, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: binding and membership read without a lock (Q5)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#7 |
| OD-2026-11-16-207 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 K3-2 finding of the third approval run of design 041, one tenant can fill the Logs class, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: no call waits for a ticket (F1)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#8 |
| OD-2026-11-16-208 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 K3-3 finding of the third approval run of design 041, redaction cost on the activity's thread, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: a doomed record is dropped before formatting (F6)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#9 |
| OD-2026-11-16-209 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 K3-4 finding of the third approval run of design 041, ingest overload slows execution, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: a risk row and the spike's saturated workload (F7)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#10 |
| OD-2026-11-16-210 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 K3-5 finding of the third approval run of design 041, tenant quota sizing, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: a risk row, runbook sizing, explicit quota in the benchmarks (F7)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#11 |
| OD-2026-11-16-211 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 K3-6 finding of the third approval run of design 041, O041-025 (a) wider than the report's stop rule, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: O041-025 restated to the code's rule (Q6)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#12 |
| OD-2026-11-16-212 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 K3-7 finding of the third approval run of design 041, ADR-0085 text against the design, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: a Note in ADR-0085 (Q8)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#13 |
| OD-2026-11-16-213 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 K3-8 finding of the third approval run of design 041, a step's view scans the whole run, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: a stream index and the Studio's paging (F4)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#14 |
| OD-2026-11-16-214 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 S3-1 finding of the third approval run of design 041, Constitution Check V names the rejected trigger, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#15 |
| OD-2026-11-16-215 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 S3-2 finding of the third approval run of design 041, G041-017 missing from the Constitution Check, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: G041-017 accepted and listed (Q7)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#16 |
| OD-2026-11-16-216 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 T3-1 finding of the third approval run of design 041, section 10's switch sentence never says "when false", is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F15)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#17 |
| OD-2026-11-16-217 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 T3-2 finding of the third approval run of design 041, Constitution V "trigger", is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#18 |
| OD-2026-11-16-218 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 T3-3 finding of the third approval run of design 041, Constitution VIII gap range, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#19 |
| OD-2026-11-16-219 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 T3-4 finding of the third approval run of design 041, O041-025 does not say what the ADR says now, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: O041-025 restated (Q6)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#20 |
| OD-2026-11-16-220 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 T3-5 finding of the third approval run of design 041, FR-041-006 in one paragraph, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: split into a list (F15)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#21 |
| OD-2026-11-16-221 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 7 T3-6 finding of the third approval run of design 041, the Overview's last sentence, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F15)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#22 |
| OD-2026-11-16-222 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge K3-1 finding of the third approval run of design 041, the secrets lock held by the ingest, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (Q5)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#23 |
| OD-2026-11-16-223 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge K3-2 finding of the third approval run of design 041, the final batch's wait for a ticket, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F1)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#24 |
| OD-2026-11-16-224 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge K3-6 and T3-4 finding of the third approval run of design 041, O041-025's predicate, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (Q6)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#25 |
| OD-2026-11-16-225 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge K3-7, C3-1, S3-1, T3-2 finding of the third approval run of design 041, ADR-0085 and Constitution V, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (Q8)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#26 |
| OD-2026-11-16-226 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge S3-2 and T3-3 finding of the third approval run of design 041, G041-017 for written acceptance, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: accepted by the owner (Q7)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#27 |
| OD-2026-11-16-227 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge C3-2 finding of the third approval run of design 041, a script item's state from the execution, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F5)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#28 |
| OD-2026-11-16-228 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge K3-8 finding of the third approval run of design 041, a stream index and empty-page paging, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F4)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#29 |
| OD-2026-11-16-229 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge Z3-2 finding of the third approval run of design 041, the exact run ceiling, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: removed, OD-2026-11-16-10) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#30 |
| OD-2026-11-16-230 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge Z3-3 finding of the third approval run of design 041, the marker in its own transaction, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (Q4)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#31 |
| OD-2026-11-16-231 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge-partly Z3-1 finding of the third approval run of design 041, constant attempt ceilings, is accepted as is (kept as the owner decided (OD-2026-11-16-9); accepted as is) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#32 |
| OD-2026-11-16-232 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge-partly K3-3 finding of the third approval run of design 041, drop before formatting, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F6)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#33 |
| OD-2026-11-16-233 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge-partly K3-4 finding of the third approval run of design 041, an overload risk row and workload, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F7)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#34 |
| OD-2026-11-16-234 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 merge-partly K3-5 finding of the third approval run of design 041, quota sizing, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F7)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#35 |
| OD-2026-11-16-235 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 8 missed finding of the third approval run of design 041, the per-batch statement count, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: 14 statements stated (F8)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#36 |
| OD-2026-11-16-236 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 IR3-1 finding of the third approval run of design 041, raising the minor breaks three tests, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: T041-7 lists them (F10)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#37 |
| OD-2026-11-16-237 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 IR3-2 finding of the third approval run of design 041, CI runs one promtool file, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (the Blocker)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#38 |
| OD-2026-11-16-238 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 IR3-3 finding of the third approval run of design 041, metric ownership and definitions, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F11)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#39 |
| OD-2026-11-16-239 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 IR3-4 finding of the third approval run of design 041, micro-benchmark harness and matcher, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F11)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#40 |
| OD-2026-11-16-240 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 IR3-5 finding of the third approval run of design 041, the spike's decision rules, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F11)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#41 |
| OD-2026-11-16-241 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 IR3-6 finding of the third approval run of design 041, a timing assertion in a CI suite, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: moved to T041-9 (F11)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#42 |
| OD-2026-11-16-242 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 IR3-7 finding of the third approval run of design 041, the forced database error, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: a test-installed trigger (F11)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#43 |
| OD-2026-11-16-243 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 IR3-8 finding of the third approval run of design 041, T041-9 and the private tickets, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: the spike's model (F11)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#44 |
| OD-2026-11-16-244 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 IR3-9 finding of the third approval run of design 041, T041-32's SDK paths, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F12)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#45 |
| OD-2026-11-16-245 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 IR3-10 finding of the third approval run of design 041, no runner has an unknown-behaviour refusal, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: the six unknown-op refusals (F11)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#46 |
| OD-2026-11-16-246 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 IR3-11 finding of the third approval run of design 041, the Studio's script-execution view, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: ScriptLogPanel (F11)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#47 |
| OD-2026-11-16-247 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 CD3-B1 (Blocker) finding of the third approval run of design 041, T041-25's false claim about CI's rule tests, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: the CI step runs every rule test (the Blocker)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#48 |
| OD-2026-11-16-248 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 CD3-1 finding of the third approval run of design 041, no integration job in ci.yml, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: the rust job and its condition (F13)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#49 |
| OD-2026-11-16-249 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 CD3-2 finding of the third approval run of design 041, the digest predicate, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: both keys (F13)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#50 |
| OD-2026-11-16-250 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 CD3-3 finding of the third approval run of design 041, the migration tests' files, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F13)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#51 |
| OD-2026-11-16-251 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 CD3-4 finding of the third approval run of design 041, T041-32's SDK path set, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F12)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#52 |
| OD-2026-11-16-252 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 CD3-5 finding of the third approval run of design 041, eventually's 5 s bound, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: eventually_within and bounded holds (F13)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#53 |
| OD-2026-11-16-253 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 ST3-1 finding of the third approval run of design 041, owner tasks with no pull request, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: documentation pull requests (F14)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#54 |
| OD-2026-11-16-254 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 ST3-2 finding of the third approval run of design 041, no hold after T3, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: six hooks (F14)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#55 |
| OD-2026-11-16-255 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 ST3-3 finding of the third approval run of design 041, T041-6 polls reads that do not exist, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: stored rows asserted by SQL (F14)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#56 |
| OD-2026-11-16-256 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 ST3-4 finding of the third approval run of design 041, T041-10 tests reader behaviour, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: moved to T041-11 (F14)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#57 |
| OD-2026-11-16-257 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 ST3-5 finding of the third approval run of design 041, T041-28 before T041-11, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: T041-28 withdrawn (F14)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#58 |
| OD-2026-11-16-258 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 ST3-6 finding of the third approval run of design 041, T041-32's path set, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F12)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#59 |
| OD-2026-11-16-259 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 ST3-7 finding of the third approval run of design 041, T041-3's done condition, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed (F14)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#60 |
| OD-2026-11-16-260 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 ST3-8 finding of the third approval run of design 041, T041-13's Options field, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: one field (F14)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#61 |
| OD-2026-11-16-261 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 ST3-9 finding of the third approval run of design 041, the release-note line, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: written by T041-29 (F14)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#62 |
| OD-2026-11-16-262 | "Corrigir e aceitar o resto" (OD-2026-11-16-7): the stage 9 ST3-10 finding of the third approval run of design 041, the canary in route bodies, is fixed in `f9d4a65f` on the owner's path and settled without a new egm-review round (fixed: T041-11 and T041-29 (F14)) | owner | Decided | docs/designs/041-step-logs.reviews/2026-11-16/egm-review-final.md#63 |

## 2026-11-15 — Design 041 (step logs): the answers to O041-014 to O041-023

After the reviews of 2026-11-15 the assistant put ten questions of design 041 to the owner, each with a recommendation
(the design's "Owner decisions needed"; `specs/041-step-logs/research.md`). The owner answered eight; O041-020 and
O041-021 stay conditional on the spike T041-31. Two answers go against the recommendation (OD-2026-11-15-5, -9). The
owner then answered O041-024, the text of the ADR-0027 Amendment (OD-2026-11-15-16), and accepted the design with
constitution 4.5.0 (OD-2026-11-15-17).

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-15-4 | "Aceitar as revisões" (O041-014 (a)): the revisions of 2026-11-13 and 2026-11-15 of ADR-0084 and ADR-0085 are accepted in place | owner | Decided | docs/adr/ADR-0084-*.md, docs/adr/ADR-0085-*.md, docs/designs/041-step-logs.md T041-3, T041-4, T041-5, T041-6, T041-7, T041-10, T041-11, T041-12, T041-14, T041-28, T041-29, T041-30 |
| OD-2026-11-15-5 | "Quota separada" (O041-015 (b), against the recommendation (a)): step logs get a tenant quota resource of their own, `tenant_step_log_bytes`, separate from `tenant_log_bytes`; it needs an amendment of ADR-0027 | owner | Decided | docs/adr/ADR-0027-*.md (Amendment of 2026-11-15), docs/designs/041-step-logs.md T041-4, T041-5, T041-6 |
| OD-2026-11-15-6 | "Cobrar 128 bytes/linha" (O041-016 (a)): every stored step-log row is charged 128 bytes on top of its text, so each row costs at least 128 bytes (the same ADR-0027 amendment) | owner | Decided | docs/adr/ADR-0027-*.md (Amendment of 2026-11-15), docs/designs/041-step-logs.md T041-5, T041-6 |
| OD-2026-11-15-7 | "Toda a vida do processo" (O041-017 (a)): a worker process keeps every secret value it resolved for its whole life, bounded; capture stops for the process when the set is full | owner | Decided | docs/designs/041-step-logs.md T041-14, T041-15, T041-16, T041-17, T041-18, T041-19, T041-20 |
| OD-2026-11-15-8 | "041, com registo" (O041-018 (a)): feature 041 holds worker-protocol minor 1.7; a register of minors in `sdks/SPEC.md` records each one, and drafts 020, 021, 025 and 026 take the next ones when they are built | owner | Decided | sdks/SPEC.md §9, docs/designs/041-step-logs.md T041-3, T041-7 |
| OD-2026-11-15-9 | "Sim, com marcador" (O041-019 (b), against the recommendation (a)): a script step's output appears in the whole-run Logs tab through a marker row, written when the script runner's report is recorded, that points to the script execution | owner | Decided | docs/designs/041-step-logs.md T041-3, T041-5, T041-13, T041-22 |
| OD-2026-11-15-10 | "Emendar o gate de CI" (O041-022 (a)): the constitution's CI gate is amended so that the step-log suites run when step-log code changes, the SDKs' capture code and the worker protocol included | owner | Decided | docs/constitution.md 4.5.0, docs/governance/constitution-4.5.0-sync-impact.md, docs/designs/041-step-logs.md T041-32 |
| OD-2026-11-15-11 | "Aceitar" (O041-023 (a)): the cursor of the step-log routes, which shows the storing transaction's id, is accepted as it is (G041-008) | owner | Decided | docs/designs/041-step-logs.md T041-11 |
| OD-2026-11-15-16 | "Emenda no ADR-0027" (O041-024 (a)): the Amendment of 2026-11-15 stays a section of ADR-0027 and is accepted as written. This is an exception, by the owner, to rule 2 of `docs/adr/README.md` (an accepted ADR is not edited in substance; a change of mind is a new ADR) | owner | Decided | docs/adr/ADR-0027-*.md (Amendment of 2026-11-15), docs/designs/041-step-logs.md T041-3, T041-4, T041-5, T041-6, T041-13 |
| OD-2026-11-15-17 | "Aceitar e construir" (design 041): the step-logs design, with constitution 4.5.0, is accepted for release 0.3.0 and is to be built | owner | Decided | docs/designs/041-step-logs.md, docs/constitution.md 4.5.0, docs/designs/041-step-logs.md T041-2 |

## 2026-11-16 — Design 043: the approval path after the second review round

The second complete review round of design 043 (2026-11-16, at `b9b0f1b5`) ended with no Blocker and 52 Majors, each
listed with a question and a recommendation in `docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/owner-questions.md`. The owner answered on 2026-11-16 (relayed by
the coordinating session): fix the findings marked for a fix, have the fixes re-read by the repository reviewers, accept
every other Major as recommended, accept ADR-0092, add an inventory flag, and keep the cache share.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-16-3 | Asked how design 043 reaches approval after round 2 (fix and re-read, a third full round, or accept): "Corrigir e aceitar o resto". Every finding marked **fix** in the round-2 owner questions is fixed (the pre-check through the start permits, a pin test of the subset's limits and charges, a test of the down migration's guards, the sentinels of rows 2 and 13, the refinement of ADR-0050 decision 2 recorded in ADR-0092), each test seen failing where it guards a defect; one short targeted re-read of those fixes and of D043-173 to D043-175, no full round; every other Major accepted as recommended, one row each (OD-2026-11-16-100 upward, a range reserved for design 043) | owner | Decided | docs/designs/043-run-input-validation.md, docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/owner-questions.md, the re-read reports `docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/reread-*.md` |
| OD-2026-11-16-4 | O043-007, whether ADR-0092 is accepted: "Aceitar ADR-0092". ADR-0092 is accepted as O043-007 described it (decisions 1 to 5; decision 7 follows from OD-2026-11-16-3); moving design 042's blocking parse behind a port stays gap G043-052 (T043-25). Decision 6, added later, is not covered and is O043-008 | owner | Decided | docs/adr/ADR-0092-*.md, O043-007, docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/architecture.md#1 |
| OD-2026-11-16-5 | The inventory exits 1 while any immutable stored version is outside the subset (round 2, critic 1, intermediate zoom-out 4): "Flag que lista sem falhar". By default a superseded version that nothing starts any more does not make `control-plane input-schema-inventory` exit 1; a flag (`--all`) lists them all, without changing the exit status; CUJ 5 is narrowed to match | owner | Decided | FR-043-018, CUJ 5, T043-24, docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/critic_report.md#1, docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/intermediate_zoom_out_report.md#4 |
| OD-2026-11-16-6 | The per-tenant share of the run-input cache (round 2, critic 2, intermediate zoom-out 3): "Manter, limite documentado". The share stays for 0.3.0; its limit is documented in the design and the runbook, and a follow-up task measures it | owner | Decided | NFR-043-BOUND, Technical Plan 3 (Capacity), T043-28, docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/critic_report.md#2, docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/intermediate_zoom_out_report.md#3 |
| OD-2026-11-16-100 | Accepted as it stands ("Corrigir e aceitar o resto", OD-2026-11-16-3): the Technical Plan and several requirements read as a record of what was built, with symbols, constants and migration names. Why it stands: the owner asked for the design to be built before approval (OD-2026-11-14-15), so the plan records the build | owner | Decided | docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/reread-egm-review.md#1 (carried from docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/zoom_out_report.md#1) |
| OD-2026-11-16-101 | Accepted as it stands ("Corrigir e aceitar o resto", OD-2026-11-16-3): the tenant share of the run-input cache is not work-conserving: a tenant alone with more than 64 hot schemas recompiles. Why it stands: kept with the limit documented (OD-2026-11-16-6); T043-28 measures it | owner | Decided | docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/reread-egm-review.md#2 (carried from docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/critic_report.md#2) |
| OD-2026-11-16-102 | Accepted as it stands ("Corrigir e aceitar o resto", OD-2026-11-16-3): inputs inside the subset that their schema refuses are not visible before the upgrade, and the Risks table cited the inventory. Why it stands: the Risks row now says the inventory finds only schemas outside the subset and the choice of `report` rests on the operator; `report` counts and audits those refusals | owner | Decided | docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/reread-egm-review.md#3 (carried from docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/critic_report.md#4) |
| OD-2026-11-16-103 | Accepted as it stands ("Corrigir e aceitar o resto", OD-2026-11-16-3): an engine pass can wait on permits inline, head of line. Why it stands: each wait is bounded at 5 s and each validation by the counter and the backstop; the pass leaves the work for the next one | owner | Decided | docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/reread-egm-review.md#4 (carried from docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/critic_report.md#5) |
| OD-2026-11-16-104 | Accepted as it stands ("Corrigir e aceitar o resto", OD-2026-11-16-3): the per-tenant cache share is unmeasured. Why it stands: kept with the limit documented (OD-2026-11-16-6); T043-28 measures it | owner | Decided | docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/reread-egm-review.md#5 (carried from docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/intermediate_zoom_out_report.md#3) |
| OD-2026-11-16-105 | Accepted as it stands ("Corrigir e aceitar o resto", OD-2026-11-16-3): the phase order of the tasks could not build green as first written. Why it stands: the tasks record what was built, in the order of the commits, and the build is green | owner | Decided | docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/reread-egm-review.md#6 (carried from docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/implementation_readiness_report.md#1) |
| OD-2026-11-16-106 | Accepted as it stands ("Corrigir e aceitar o resto", OD-2026-11-16-3): the gate's API signatures are elided in the plan. Why it stands: the plan records built code; the signatures are in `crates/application/src/run_input.rs` | owner | Decided | docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/reread-egm-review.md#7 (carried from docs/designs/043-run-input-validation.reviews/2026-11-16/round-2/implementation_readiness_report.md#7) |

## 2026-11-14 — Schedules on by default: quotas, input schema, upgrade order and the handover

Before schedules are switched on by default (OD-2026-11-13-8) the assistant asked four questions, each with a
recommendation. The owner took two recommendations and chose otherwise twice: the input of every run start is validated
against the target version's `inputSchema` in 0.3.0, and the handover of the cron triggers to schedules is done
automatically during the upgrade instead of by an operator's command. Both need design work before code, and both are
in release 0.3.0.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-14-1 | "Manter 200 e 4": a namespace has at most 200 schedules and 4 backfills running at once by default in 0.3.0; a namespace changes them through its quotas | owner | Decided | docs/runbooks/schedules.md, crates/domain quota defaults, T018-007 |
| OD-2026-11-14-2 | "Validar todos os starts já" (against the recommendation to defer G1955): the input of every run start (API, SDK, CLI, triggers, child runs, schedules, batch starts) is validated against the `inputSchema` of the target version in 0.3.0 through one validator port, and schedules use the same check (`SCHEDULE_INPUT_INVALID`, `NOMINAL_TIME_FIELD_INVALID`). A behaviour change: an input that does not match is refused where it was accepted | owner | Decided | G1955, G2081, FR-018-016, T018-016, docs/releases/0.3.0.md, T043-1, T043-2, T043-3, T043-4, T043-5, T043-6, T043-7, T043-8, T043-9, T043-10, T043-11, T043-13, T043-14, T043-15, T043-16, T043-17, T043-18, T043-20, T043-21, T043-23 |
| OD-2026-11-14-3 | "Migrate, engines, control": the documented upgrade order to 0.3.0 is migrate, then the execution engines, then the control planes | owner | Decided | docs/runbooks/schedules.md, docs/runbooks/scaling.md, docs/releases/0.3.0.md |
| OD-2026-11-14-4 | "Automático no upgrade" (against the recommendation to keep it manual): the handover of the cron triggers to trigger-owned schedules happens without an operator command once every live engine lists `schedules-v1` and the schedules have converged; `schedule-trigger-handover` stays for an operator who wants to do it first or check it. The handover stays one-way, and its accepted losses (G2005, G2020, G2021) go in the release notes | owner | Decided; not designed | T018-061, ADR-0083, docs/runbooks/schedules.md section 6 |
| OD-2026-11-14-5 | After the architecture, security and correctness reviews of the first draft of ADR-0091 (a definer function the runtime role calls: it can forge most of the conditions it checks), "Job pós-deploy": the automatic handover is a one-shot job with the owner's credentials, as `migrate` is (a compose service and a Helm `post-upgrade` hook), that runs the tested `schedule-trigger-handover` command and waits, bounded, until its conditions hold. No new privilege for the runtime role. A deployment of its own runs the command | owner | Decided; not built | docs/adr/ADR-0091-*.md, T018-082 to T018-087, infra/docker/compose.yaml, infra/helm |
| OD-2026-11-14-6 | "Sim, com valor de config" (O1954): an operator keeps the handover manual with `schedules.autoHandover: false` (Helm) or its compose twin, which leaves the job out; on by default | owner | Decided; not built | O1954, infra/helm/values.yaml, docs/runbooks/schedules.md section 6 |
| OD-2026-11-14-7 | "Sim, uma linha por tenant" (O1955): every tenant that has a cron trigger gets one audit record of the handover, written after the handover commits and keyed by its epoch, not inside its locks | owner | Decided; not built | O1955, D2042, T018-061 |
| OD-2026-11-14-8 | After the reviews of design 043, "Enforce, com modo report" (O043-001): input validation refuses by default (`enforce`); a deployment may run in `report` (record and measure, refuse nothing) while it fixes its callers. The mode goes through the instance registry so that replicas cannot answer differently; the release notes carry a warning and a query that lists the versions at risk | owner | Decided | O043-001, docs/designs/043-run-input-validation.md, ADR-0090, T043-3, T043-12, T043-19, T043-22 |
| OD-2026-11-14-9 | "Validar também essas" (O043-004, against the recommendation): the children and continuations of runs created before the upgrade are validated like every other start. It is a recorded deviation from Constitution principle IV (runs in progress must not break), accepted by the owner: an old run can fail at the step that creates a child or continues | owner | Decided | O043-004, docs/designs/043-run-input-validation.md Complexity Tracking, docs/releases/0.3.0.md, T043-8, T043-9, T043-19 |
| OD-2026-11-14-10 | "Sim, recusar sempre" (D043-005, against the recommendation): a Signal-with-Start (and Update-with-Start) whose holder is open is refused when its start input does not match the schema, although no run is created | owner | Decided | D043-005, docs/designs/043-run-input-validation.md, T043-5 |
| OD-2026-11-14-11 | "Aceitar as duas" (O043-002, O043-003): a published `inputSchema` that does not compile refuses every start of that version (`RUN_INPUT_SCHEMA_UNUSABLE`), a permanent refusal for the creators nobody waits for; a rerun or reset of a run created before 0.3.0 is validated like every start | owner | Decided | O043-002, O043-003, T043-2, T043-5, T043-7, T043-8, T043-9, T043-10, T043-11, T043-21 |
| OD-2026-11-14-14 | After four reviews of design 043 showed that the cost of validating an arbitrary JSON Schema with the present library cannot be bounded beforehand (exponential `unevaluated*`, a `$ref` resolved differently by the walk and the library), "Subconjunto suportado": 0.3.0 accepts a documented subset of JSON Schema for `inputSchema` (`type`, `properties`, `required`, `items`, `enum`, `const`, numeric and length bounds, a bounded `pattern`, `additionalProperties`, `allOf`/`anyOf`/`oneOf` without recursion, local `$ref` without cycles), validated by a validator of the platform with a work counter, so the bound holds by construction; a schema with any other keyword is refused at publication (`report` mode only warns), and the release-notes inventory lists the versions it affects | owner | Decided | docs/designs/043-run-input-validation.md, ADR-0090, O043-001, T043-2, T043-3, T043-13, T043-19, T043-24 |
| OD-2026-11-14-15 | After a summary of design 043 and ADR-0090 (seven rounds of security and correctness review and two of architecture, each ending ready): "Aceitar e construir". ADR-0090 is accepted and T043-1 to T043-24 are to be built | owner | Decided | docs/adr/ADR-0090-*.md, docs/designs/043-run-input-validation.md, T043-1 to T043-24 |
| OD-2026-11-15-3 | G043-045, asked whether each start operation of the OpenAPI (`POST /runs`, rerun, reset, Signal- and Update-with-Start, ad hoc) describes its `422 RUN_INPUT_INVALID`: "Atualizar e voltar a fixar". The operations gain the text, and the OpenAPI baselines that pin them (features 016 and 035) are re-pinned after reading the diff, the one time this decision covers | owner | Decided | G043-045, T043-14, the pinned OpenAPI baselines of features 016 and 035 |

## 2026-11-13 — What release 0.3.0 must contain: feature 018, schedules on by default, step logs per run

Before the release pull request (219) was merged the owner asked whether anything else belonged in it. The assistant
listed what was not built and recommended shipping as it was; the owner chose otherwise, then named the features the
release must have ("Pause, Play, Replay, Sagas, Outbox, Inbox, Scheduler, Workflow, Logs"). The assistant mapped them:
seven exist; schedules were off by default and there was no capture of step logs per run. It asked both, recommending
the smaller option each time. The owner chose the larger. Release 0.3.0 is therefore not published until these are
built and the release-candidate run is repeated on the commit that has them (OD-2026-11-04-8).

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-13-7 | "Incluir a 018": the rest of feature 018 (pull requests 91 and 98) is part of release 0.3.0 | owner | Decided | specs/018-schedules-bulk-operations/tasks.md, docs/releases/0.3.0.md |
| OD-2026-11-13-8 | "Ligados por omissão": schedules are on by default in 0.3.0 (`SAGAT_SCHEDULES_ENABLED` defaults to true). The open tasks of 018 that justify it (benchmark, observability gates, runbooks, the cron-trigger handover) are closed first | owner | Decided; not built | crates and apps of feature 018, docs/runbooks/schedules.md |
| OD-2026-11-13-9 | "Logs de passo por run na 0.3.0": the lines a step's code logs are captured per run, stored with retention and limits, and readable through a route and a Studio screen, in release 0.3.0. It is a new feature (041) with its own design | owner | Decided | docs/designs/041-step-logs.md |
| OD-2026-11-13-10 | "Atualize o rust para a ultima versao", then "1.100": the pinned toolchain moves to Rust 1.100.0 (stable of 2026-11-12), one day old; the two-week rule (D138) is about dependencies and the owner named the version | owner | Decided; pull request 221 | rust-toolchain.toml, infra/docker/Dockerfile |
| OD-2026-11-13-11 | "Controle de througput para fluxos que tenham limitações de execução, buffering", answered "Concorrência + taxa": release 0.3.0 has throughput control: concurrency limits by key with a durable queue (feature 022, US02) and a rate limit per time window with buffering, added to feature 022 as new requirements | owner | Decided; not built | specs/022-flow-control-step-policies/spec.md |
| OD-2026-11-13-12 | "Tudo na 0.3.0": release 0.3.0 waits for feature 018 with schedules on, step logs per run (041) and throughput control (022), against the assistant's recommendation to ship 0.3.0 first and the rest in 0.4.0 | owner | Decided | docs/releases/0.3.0.md |
| OD-2026-11-13-13 | "Debounce, Window, Map, FlatMap, Filter, IgnoreElements, Skip, Distinct entre outros": reactive stream operators are wanted in release 0.3.0. Opened as feature 042 (Draft design); where they apply (event streams before a run, collections inside a workflow, or both) and the first operator set are questions the design puts to the owner | owner | Decided; not designed | docs/designs/042-stream-operators.md (to be written) |
| OD-2026-11-13-14 | "Aceitar todas" (the recommendations of design 041, step logs): O041-001 to O041-011 as the design recommends (capture on at `info` with a namespace switch and a worker opt-out; `run:logs:read`; reads audited once per subject and run per 300 s; PostgreSQL rows; the stated ceilings; 14-day retention; first lines kept with a marker; a new SSE tail; script steps as a marker; server, six SDKs, routes and Studio together; the stated limit of redaction) | owner | Decided | docs/designs/041-step-logs.md T041-4, T041-5, T041-6, T041-7, T041-10, T041-11, T041-12, T041-13, T041-21, T041-25 |
| OD-2026-11-13-15 | "Aceitar todas" (design 042, stream operators): O042-001 to O042-009 as the design recommends; release 0.3.0 carries only the collection operators (S0 and S1); trigger operators are a later release | owner | Decided; not built | docs/designs/042-stream-operators.md T042-1, T042-5, T042-6, T042-7, T042-18, T042-20, T042-39, T042-43 |
| OD-2026-11-13-16 | "Aceitar todas" (feature 022, the 0.3.0 slice): O1163 to O1168 as recommended (a strict sliding window without burst; FIFO per key with skip-ahead; a full buffer refuses a synchronous start and keeps event and cron starts pending; a retry takes a new slot; no rate limit on collection body nodes in 0.3.0; a reset takes a slot or is refused) | owner | Decided; not built | specs/022-flow-control-step-policies/research.md O1163..O1168, T022-066 |
| OD-2026-11-13-17 | "Aceito tudo": ADR-0056, ADR-0084, ADR-0085, ADR-0086, ADR-0087 and ADR-0088 are accepted | owner | Decided | docs/adr/ADR-0056-*.md, docs/adr/ADR-0084-*.md to docs/adr/ADR-0088-*.md, T042-4, T022-066, T041-2 |
| OD-2026-11-13-18 | After the security review of design 041: "Exigir a concessão PII", reading step logs in a namespace with classified schemas also requires the namespace PII reveal grant (changed on 2026-11-16 by OD-2026-11-16-8: the grant is required in every namespace); and "Fechar ao viewer", the `log` field of a script execution requires `run:logs:read`, so viewers no longer read it | owner | Decided | docs/designs/041-step-logs.md T041-5, T041-11, T041-12, T041-22, T041-25, T041-28, T041-29 |
| OD-2026-11-13-19 | After the reviews of designs 022 and 042: "Aceitar o ADR-0089" (a gated step waits outside READY and one fenced grant routine, driven by key, serves every waiter whatever its gates; it supersedes ADR-0056 decisions 3 and 5) and "Confirmar a revisão" (the revised ADR-0087: the switch at publication, the chain bounded at 1,000 elements, 256 KiB per element and 4 MiB per stage, a failed chain fails the run) | owner | Decided | docs/adr/ADR-0089-*.md, docs/adr/ADR-0087-*.md, T022-107, T042-4 |

## 2026-11-13 — Feature 018: the open questions, the audit of business-id conflicts and ADR-0081 to ADR-0083

The assistant put the decisions that held pull requests 91 and 98 to the owner in three questions, each with a
recommendation. The owner chose the recommended answer of each.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-13-2 | "Aceitar todas": O1950 (a), only a holder of `run:operate` may write a `CANCEL_OTHER` schedule (as built, D1953); O1951 (b), the effect on a run and the record of its batch item become one transaction, through a hook into the single-run use cases, before the feature is switched on (ADR-0050 decision 2, G1961); O1952 (a), T018-074 closes on the journeys it has, with the four states it cannot show covered by `ResetRun.test.tsx` and the REST suites (G2034, G2035); O1953 (a), a batch job survives ten failed selection rounds, about 25 minutes, before it ends `SELECTION_FAILED` (D2197) | owner | Decided; O1951 built 2026-11-13 (D2219) | specs/018-schedules-bulk-operations/research.md |
| OD-2026-11-13-3 | "Nova ação de audit": business-id conflicts are audited (G2086). An audit action `business_id.conflict` is written where the refusal is decided (the `409` of a person's start, and the decision of a schedule that finds the id held), with a closed reason and without the id; FR-018-044 stands as written | owner | Decided; built 2026-11-13 (D2218) | specs/018-schedules-bulk-operations/spec.md FR-018-044 |
| OD-2026-11-13-4 | "Aceitar os três": ADR-0081 (the cancel source of a reset is switched on per process, engine first, until the capability registry exists), ADR-0082 (the instance registry: only the schema owner records requirements, a refused process stays up with a closed gate) and ADR-0083 (the cron triggers are handed to the schedule pass once, and only when they have converged) are accepted | owner | Decided | docs/adr/ADR-0081-*.md, docs/adr/ADR-0082-*.md, docs/adr/ADR-0083-*.md |
| OD-2026-11-13-5 | "Aceitar como está": the `business_id.conflict` record of a person's refused start is written in a transaction of its own after the start rolls back. It is best effort, and a lost write is counted by `audit_write_failures_total{action}`. Identical repeats within 300 s collapse into one record, like `database.denied`. Conflicts the platform decides (a schedule, a batch item, a reset completion) are recorded in the decision's own transaction. This accepts D2221 as the reading of FR-018-044 | owner | Decided | specs/018-schedules-bulk-operations/research.md D2221 |

## 2026-11-13 — OCSF as the standard for security and audit events (feature 040)

The owner asked in the Claude session for one more specification. The assistant opened feature 040 as a Draft design
and read the request as: the security and audit events the platform hands out take the OCSF shape, as a projection of
the native record. That reading and the open questions O040-001 to O040-007 are the owner's to confirm.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-13-6 | "Adicione mais uma especificacao de ter o https://schema.ocsf.io/ como padrao": a specification that adopts OCSF as the standard is added as feature 040 | owner | Decided; Draft design, not built | docs/designs/040-ocsf-event-standard.md |

## 2026-11-13 — Release 0.3.0 ships with the fallback ceilings

The assistant reported that the T014-042 acceptance method (OD-2026-11-04-7: release build, runtime role, a separate
database host, two repetitions) cannot run on the shared laptop, which has no separate database host, and offered two
paths: provide a host, or ship 0.3.0 with the fallback ceilings of OD-2026-11-04-5. The owner answered "publicar a 0.3",
then "Vamos seguir com todo restante das tasks que ainda faltam".

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-13-1 | "publicar a 0.3": release 0.3.0 publishes `maxItems` 1,000 and `maxIterations` 1,000, the fallback values of OD-2026-11-04-5, without the T014-042 acceptance run. Validation keeps accepting up to 10,000, and the tenant quota still bounds a collection. The 20,000 pending waits per replica of OD-2026-11-04-6 is published with its metrics and alerts. Raising the published ceilings to 10,000 needs the acceptance run on a separate database host and is deferred to a later release. This supersedes OD-2026-11-04-7 for 0.3.0 only. | owner | Decided | specs/014-workflow-primitives/tasks.md T014-042; docs/runbooks/workflow-primitives.md; docs/releases/0.3.0.md |

## 2026-11-12 — Feature 014 closes on root collections; nesting becomes feature 039

On the owner's "Vamos terminar as primitivas", the assistant mapped the eight open tasks of feature 014. Root
collections, loops, approval, switch, declared operations and the restricted continue-as-new are built; what holds the
eight tasks open is general **nesting** (a collection inside a collection or beside a loop or an approval), which the
codec refuses on purpose and which needs the whole capacity program first: the owner's acceptance of ADR-0078 and of
its open items (the creator, `SCOPED_PATH`, sweeper coverage), then the sweeper, the creator with its migration, G715,
G724 and G725, the barrier proofs and an activation flag. It recommended (A) closing 014 on what is built and moving
nesting to a feature of its own, over (B) keeping the original criteria. The owner answered "Siga as recomendacoes".

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-12-6 | "Siga as recomendacoes": feature 014 closes on root (unnested) collections, loops and the other built primitives; general nesting, the capacity program it needs and the nested acceptance criteria move to feature 039 | owner | Decided; not built | specs/014-workflow-primitives/tasks.md T014-014..017, T014-040, T014-042..044; docs/designs/039-nested-collections.md |

What changes, as the recommendation said it:

- This narrows row 1 of 2026-10-26 ("the eight remaining feature-014 tasks … with their original acceptance criteria"):
  each of the eight tasks keeps its criteria for what feature 014 ships and hands its nested part to feature 039.
- OD-2026-11-04-7 kept the nested 100,000-instance workload in T014-042; it moves to feature 039 with the rest of
  nesting. Its other criteria stay in T014-042.
- Unchanged: OD-2026-11-04-3 (the ChildRoot guard stays off), OD-2026-11-04-4 (ADR-0078 before any production caller),
  OD-2026-11-04-5/6 (ceilings), OD-2026-11-04-8 (one release-candidate commit for T014-040, T014-042 and T014-043).
- The codec keeps refusing nesting; nothing that is refused today is enabled by this decision.

## 2026-11-12 — Merge the rest of the design 038 stack

OD-2026-11-12-2 covered pull requests 203 and 205 to 210. The assistant then built the remaining tasks of design 038 in
pull requests 211 to 215 (the budget suite in CI with constitution 4.4.0, the window order, the scoped kind check, the
claim's session filter, the K-poller test and the dated report section), said that merging them needed the owner's word,
and asked "Integro os PRs 211 a 215 no develop quando o CI de cada passar?". The owner answered in the Claude session.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-12-5 | "Sim, integrar": merge pull requests 211, 212, 213, 214 and 215 into `develop`, one at a time, each after its CI | owner | Decided; merged 2026-11-12 | pull requests 211-215, constitution 4.4.0 |

Merging 211 adopts constitution 4.4.0, whose Sync Impact Report named OD-2026-11-12-4 as the decision it rests on.

## 2026-11-12 — Merge the hot-path stack, let polls try their window in their own order, budget the hot path in CI

The assistant had stopped its loop with seven pull requests stacked and three questions open, and on the owner's
question "O que prciso decidir?" set out each with options and a recommendation: (1) merge pull requests 203 and 205
to 210 into `develop` one at a time once 210's CI passed (recommended); (2) let each poll try the shared candidate window
from its own starting point, no candidate removed, a lone worker still served in `ready_at` order and the oldest step
always in the window, with the SDK conformance scenario that states the order narrowed to a lone worker (recommended);
(3) run the budget target on a pull request that changes the hot-path files, about a minute and a half of CI on those
(recommended). The owner answered in the Claude session.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-12-2 | "Seguir as recomendações": merge pull requests 203, 205, 206, 207, 208, 209 and 210 into `develop`, one at a time, each after its CI | owner | Decided | pull requests 203, 205-210 |
| OD-2026-11-12-3 | "Seguir as recomendações": a poll tries its window from a per-session starting point within FR-038-004's limits; the conformance scenario's ordering holds for a lone worker | owner | Decided; not built | docs/designs/038-claim-transition-efficiency.md T038-8, FR-038-004 |
| OD-2026-11-12-4 | "Seguir as recomendações": the budget target runs on a pull request that changes the hot-path files | owner | Decided; not built | docs/designs/038-claim-transition-efficiency.md T038-10, FR-038-008 |

One sentence answered three questions; each row is the recommendation it accepted, as it was put to the owner. It
widens the 2026-11-04 rule that a suite runs on a pull request only when its own files change, for one target, and
does not change that rule otherwise.

## 2026-11-12 — Design 038 (claim and transition efficiency) accepted for implementation

The assistant measured the transitions scenario on `develop` (368 transitions per second against 1,513 on 2026-10-11),
removed two advisory-lock mutexes (pull request 203, 763 afterwards on a busy machine) and profiled what remains: 34 probe
transactions and 7.5 full run loads per lease. It drafted [038-claim-transition-efficiency.md](designs/038-claim-transition-efficiency.md)
(pull request 204), said it was new scope and asked "Aceita o desenho 038 (PR 204) para implementação?".

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-12-1 | "Aceito, implementar" | owner | Decided; not built | docs/designs/038-claim-transition-efficiency.md |

Read as the answer to the question asked and no more. It does not answer the design's second owner question, whether a
bounded departure from oldest-first claiming is acceptable: the task that needs it waits (T038-8 and FR-038-004 in the
second draft, which renumbered them after the reviews; it also asks whether the budget target may run on pull requests
that change the hot path, T038-10). It does not
make the design `Approved` either: `egm-review` and the architecture, security and correctness reviews still have to end
with no Blocker or Major finding open.
## 2026-11-11 — Drafts 019 to 032 and 036 approved for implementation

The assistant had presented an assessment of the distance to Temporal and Windmill
([roadmap-assessment-2026-11-11.md](roadmap-assessment-2026-11-11.md)) and said that drafts 019 to 032 and 036 needed the
owner's explicit approval before anything of theirs was built. The owner answered in the Claude session.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-11-1 | "Aprovo os rascunhos 019–032 e 036, pode implementar" | owner | Decided; not built | specs/019-worker-deployments-dispatch to specs/032-multi-region-failover, specs/036-elastic-scaling, docs/roadmap.md |

Read narrowly, which is the assistant's reading and the owner can widen it: the fifteen specifications may be implemented,
in the order the assessment recommends unless the owner names another. The sentence does not by itself:

- accept the proposed ADRs of those features (ADR-0051 to ADR-0068, ADR-0072 to ADR-0074): the ADR rules say a person
  accepts each one, and the assistant asks when the first task that depends on one is reached;
- answer the open owner questions of each feature (among them O1081 on `pii` leaving the platform, O1360 on the app
  builder, O1700 to O1702, O1704 and O1710 of 036), which keep their recommended default as a proposal until answered;
- amend the constitution, which 032 needs (O1560: the Delivery Constraints fix a single region) and 020 needs for
  retention under principle I;
- tick the `checklists/*.md` items of a feature, which block its implementation until the owner accepts them
  (OD-2026-11-03-33).

It unblocks the 018 tasks that waited for 019, 020 and 022 to be approved (T018-037's permit, T018-051, T018-053's permit,
T018-055, T018-077, T018-079) as far as approval was what held them; they still need those features built.

## 2026-11-10 — A plain-`http` storage or token endpoint is a development allowance that the host and the runner honour (O035-010)

The assistant asked the owner in the Claude session which layer owns the allowance, with three options (specs/035-iceberg-write-steps/research.md, G035-095): (a) a TLS front for the fixtures and a CA the host
trusts, (b) the host and the runner accept an `http` storage or token endpoint on the loopback only when the runner is a development one, (c) the server refuses a plain-`http` endpoint at save. The owner answered with one letter.

| ID | Decision | Decided by | Status | Covers |
|---|---|---|---|---|
| OD-2026-11-10-1 | "B" | owner | Decided; the host, the runner and the server's refusal built | specs/035-iceberg-write-steps/research.md (G035-095, O035-010), tasks.md T035-062 |

Read as the assistant recommended it with the option: the host and the runner accept `http` for the S3 endpoint and the OAuth2 token endpoint **on the loopback, and only when the runner says it is a development
one** (a flag the runner sets and the connection can never set), which makes true what the server already says of a development namespace; **and the server also refuses, at save, an `http` host that is not the
loopback**, since nothing could run it. The second half is the assistant's reading of "B" together with its recommendation, and is built as a separate change so that the owner can correct it (built 2026-11-10: `PLAIN_HTTP_OUTSIDE_LOOPBACK`). It unblocks T035-062.

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
| OD-2026-11-06-2 | "Sim. Rodar os testes somente dos arquivos alterados e suas dependencias." (the answer to: should a shared input, that is a contract, `tests/support`, `tests/fixtures`, `runners/`, a dependency of `tests/Cargo.toml`, select no suite in a pull request and leave all to the release?) | owner | Decided; narrowed for contract files by OD-2026-11-20-7 | scripts/select-test-targets.py, docs/test-strategy.md, tests/AGENTS.md |

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

*Note (2026-11-25) on OD-2026-11-03-22 and OD-2026-11-03-36:* OD-2026-11-24-4 narrows them for release 0.3.0. A row
decided by exactly `delegate (OD-2026-11-24-4)` now counts as the owner's for an `Owner-gate:` line and for accepting a
review finding, because the owner delegated those acts for the release. The rows above, decided by a plain `delegate`,
still do not count, and a delegate row may not accept a finding of a security review. The delegation ends when the
tag `v0.3.0` exists: a delegate row counts only as the tag holds it, so rows recorded before it stay valid and rows
added or edited after it are refused. `scripts/check-design-docs.py` (`DELEGATIONS`) enforces this, and
[sdd-workflow.md](engineering/sdd-workflow.md) ("Who decides") explains it.

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
