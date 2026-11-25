# Resolution of round 1 (2026-11-24)

Doc: docs/designs/044-workflow-as-code.md
Reviewed commit of round 1: 44133eb732bb25f93904c0d144a7b5af1fa26d9f

How each Blocker and Major of round 1 was answered, by the author under the owner's delegation (OD-2026-11-20-6 for the
design questions, OD-2026-11-24-4 for settling review findings). "Fixed" means the doc, the ADRs or the register changed;
the decisions are Decision 044-042 to Decision 044-068 of `specs/044-workflow-as-code/research.md`. Every Nit was applied
unless listed under "Nits not applied". The correctness, architecture, implementation-readiness, codebase-details and
intermediate zoom-out reviews had not run when this round's fixes were made (the session's limit of concurrent reviewers
refused them), so they read the fixed text in round 2.

Counts of round 1, before fixes: Blocker 5, Major 63, Nit 80 (zoom-out 0, 11, 5; comprehension 0, 10, 14; design 1, 17,
13; structural 1, 6, 10; style 0, 5, 32; security 3, 14, 6; several findings are the same defect seen by two lenses).

## Fixed

| Report and finding | Answer |
|---|---|
| security 1 (Blocker): secrets not tied to activities | Envelope entries per activity with their own secrets; the fragment stored immutably; `ResolveSecrets` reads from it (Decision 044-046) |
| security 2 (Blocker): history export of raw payloads | Export through feature 016's redaction and reveal rules, never `secret`, admin and developer roles, `INCONCLUSIVE_REDACTED` (Decision 044-050, Architecture decision 0108) |
| security 3 (Blocker): publisher-only fields in fragments | Code profile per node kind; output schema and approval policies from the envelope (Decision 044-045) |
| design 1 (Blocker): a move changes `workflow_version_id` | A separate `code_version_id`; the start version never changes (Decision 044-042) |
| structural 1 (Blocker): Constitution Check against an uncommitted 5.0.0 | Checked against 4.7.0 and 5.0.0, the 4.7.0 clauses named, Task 044-1 depends on the merge of 5.0.0 |
| zoom-out 1, comprehension 5, design 7, structural 4: the statement budget | Statements renumbered, 10 for an ordinary completion, terminal and handoff paths stated, the batched path named as new code (Decision 044-055) |
| zoom-out 2: hold duplicates step review | The step review of feature 003 is the review (Decision 044-044) |
| zoom-out 3: a second claim path | The reserved activity name routes `~workflow` through the existing claim (Decision 044-047) |
| zoom-out 4: `code_nodes` duplicates | Dropped; the fragment in `payloads`, its digest in the step and the journal |
| zoom-out 5: bespoke move route and table | A batch action of feature 018 plus a single-run route (Decision 044-048) |
| zoom-out 6, design 26: promotion replay check | Dropped; a dry-run move (Decision 044-049) |
| zoom-out 8: eager dispatch before measurement | Built after the non-eager path as a deferrable task; both commit counts stated (Decision 044-060) |
| zoom-out 9, design 16: compiled-language deploys | Recommended topology, the pipeline of journey 2, the unserved alert |
| zoom-out 10: export bypasses feature 016 | As security 2 |
| zoom-out 11, design 14: constitution precondition | As structural 1 |
| comprehension 1: binding undefined | Defined in Terms, with its file |
| comprehension 2: features and decisions not linked | The table "What this design builds on" |
| comprehension 3: "sequence" ambiguous | Journal sequence, command number and visible count named and used |
| comprehension 4, design 21: the sticky window drops events | The window `(S(k − 1), S(k)]` defined once and used for every lease |
| comprehension 6, design 2: `run_version` against invisible events; a lease in flight during a move | The visible count; a move waits while the step is leased; `RUN_BUSY` after three; every completion checks the digest (Decision 044-042) |
| comprehension 7, design 11, structural 8: causes | One table of causes, behaviour and remedies |
| comprehension 8, design 9: no exit from `HISTORY_LIMIT` | The closing window (Decision 044-057) |
| comprehension 9, design 6: a reset with a new seed | A reset keeps its source's seed (Decision 044-054) |
| comprehension 10: the manifest | Defined, with the command per language |
| design 3, security 22: the chain is computable from the history | Keyed chain over fragments the worker is not sent (Decision 044-043) |
| design 4: the digest misses dependencies, runtime, locale | Runtime and dependency closure hashed; UTC and the root locale (Decision 044-052) |
| design 5: Python module-level imports pass the finder | The finder is active at load; Python 3.11 (Decision 044-052) |
| design 8: the 4 MiB retained bytes | Technical Plan 10.4; the suggestion at 3 MiB |
| design 10: terminal commands with outstanding work | Requirement 044-031 (Decision 044-056) |
| design 12: saga plans are per version | Run-scoped plans, compensation identifiers from the start version, the policy values (Decision 044-061) |
| design 13: the registry gates only an activated requirement | The first publication activates it (Decision 044-062) |
| design 15, security 13, security 14: bounds | Requirement 044-032 and the bounds of Decision 044-058 |
| design 17: abandoned coroutines | The cap and the withdrawal (Decision 044-063) |
| design 18: the Temporal comparison is out of date | Rewritten against Worker Deployment versioning and the replayers |
| structural 2: owner decisions not on this branch | "Owner decisions needed" names them and where they are recorded; they reach `develop` before this design's pull request |
| structural 3, style 12: the runtime task range | Task 044-10 and Task 044-15 to Task 044-19 |
| structural 5: the command set and the search-attribute event | The command is added with feature 020 (Decision 044-066) |
| structural 6: no `Refs:` names a non-functional requirement or success criterion | Added; Tasks 044-28 and 044-25 hold Success criteria 044-002 and 044-003 |
| structural 7: the study's facets | Every language has Runtime, Hazards, Isolation, Detection, Precondition and Verdict |
| style 1 to 5 | New identifier forms in prose, "journey" in prose, the unsourced sentence replaced, the rollback sentence rewritten |
| security 4: expressions in fragments | Literals or refused (Decision 044-045) |
| security 5: script and reserved names | Prefixes refused; scripts by digest (Decision 044-046) |
| security 6: `*` bindings and the reserved name | Exact bindings only; the name reserved (Decision 044-047) |
| security 7: permissions | Named for every new route (Technical Plan 7.3, 7.4, 8) |
| security 8: moves need more than `run:operate` | `version:activate`, `run:batch`, an approved target (Decision 044-048) |
| security 9: worker queries and update results against Architecture decision 0031 | Disclosure, closed result schemas, validation, `no-store`, the receipt rule (Decision 044-067) |
| security 10: worker free text | Closed causes and a bounded type name; the expected command from the server (Decision 044-068) |
| security 11: bindings re-checked only at completion; tokens in replays | Re-checked on every call; no lease in a replay, no token stored (Decision 044-060) |
| security 12: history reads of checks and blobs | Per-run read capability; `ReadBlob` limited to the run's history; commands reference only those blobs |
| security 15: a trigger is not a privilege boundary | No update privilege for the runtime role; a `SECURITY DEFINER` function |
| security 16: approval cannot review code | Said in the doc and the Studio; Gap 044-014 |
| security 17: a known Python seed | The operator's own fixed seed (Decision 044-051, Owner question 044-013) |

## Contested

| Report and finding | Why it stands |
|---|---|
| zoom-out 7: defer worker queries | Kept as a deferrable task (Task 044-29), with a user story and a journey: an answer that exists only in the function's memory would otherwise be published into the journal at every iteration. Owner question 044-014, decided by delegation |

## Nits not applied

- Zoom-out 15 (spike numbers repeated in the study): the study is a deliverable of this design (owner decision
  2026-11-24-3), so its numbers stay where a reader of the study reads them; the register keeps the exact versions.
- Style 18 (British and American spelling): the repository's documents mix both; the doc keeps the spelling of the
  words it shares with the code it names.
- Structural 17 (the template's stale comment): outside this design, left for a separate fix.
