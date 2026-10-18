# B0 bounded source reviews, cycle1

Three independent fresh-context gpt-6-astra/xhigh reviewers, read-only/no tests/subagents.
Artifacts are separate A/B/C source-inventory sections, not a fourth whole-protocol proposal.
All findings accepted as actionable source-document corrections, not proved runtime failures.
No external CLI; owner's selected current review applies.

## A — review_b0_execution_edges

1. Missing chain-addressed signal/cancel (begin_chain_head chainU→resolvedcurrentRunU) and
   chain Update (chainU→receipt→current or prior resolvedRunU), not covered by run-addressed row.
2. Outgoing link/legacy-child flag UPDATEs retain NK, not U (execution.rs4963/4996/5045).
3. Continue locks expected_head before comparison; historical expected segment can be replayed.
4. Ordinary persist terminal projection updates own chain/segment and incoming linkNK; operator
   terminal writes are not scheduler finalize. Approvalcancel stepU→requestNK→timerNK suffix.
5. Initialchild parentNK→treeX→reservationstepNK beforeadmission; parentShardS only after
   create/admission on still-open creation TX. Full/path scheduler ownShardS conditional.
6. Private retained/adHoc definition WorkflowsquotaX precedes managed/root admission.

## B — review_b0_authority_edges

1. Managedreportreceipt replay SecretsX→dedicatedMembershipS→BindingS, noSession, different
   from live owner. Candidate inversion withregistration/claim is not reproduceddeadlockproof.
2. Ordinarysecretdelivery afterworkerauthority retains secretsS→publisherMembershipS→publisherACL S.
3. SecretACL/pool edits have noNamespaceSecretsX. ACLinsert/delete andsecretU poolrewrite are
   separate from connection/provider/version mutations. No new guards requested byinventory.
4. Namespacepolicy edit tenantS→namespaceX; exactactivity tenantS→namespaceS→activityX,
   not an all-X hierarchy.
5. Claims retain earliernode activity/script/catalog locks whileprocessingnextnode, giving
   activityA X→connectionB S andscriptpolicyS→activityB X edges. Sortingactivities is narrower.
6. Derivedscript leases consumepriorpins (legacyREADY fallback), no currentretentionpolicyS.

## C — review_b0_implicit_edges

1. Rerun INSERT takes existingterminalsourceKS via runs.rerun_of, aftermanagedSecretsX and
   activequota/payload retention; reverses terminalcleanup sourceU→SecretsX. Sourceinversion
   only; focused proof/remedy separate. FK20261015000300_operator_actions.
2. BlobCatalog.reserve ownTX blobadmissionX→STEP_OUTPUT INSERT existingrun/stepFKKS→deferred
   quota. Separate fromownednewRun commit_input andunchangedidentityupdates.
3. Every nonzero retaineddelta, includingnegative deletion, locks retainedusageU; positivegrowth
   alone adds content/logpolicyS. take_log_prefix locks contentS→logS→usageU beforeexplicitlogwrite.
4. Revealgrant authority is namespace advisoryS payload-reveal-grants:tenant:namespace seed0,
   includingabsentgrants, notgrantrowFORSHARE.

Corrected sectionA cycle2 underway. B/C cycle2 waits for the focused inversions' evidence so
the same final source need not be re-reviewed unchanged. Complete B0 remains unchecked.

## A cycle2 — review_b0_execution_edges_v2

Both findings accepted after rereading SQL/migration.
1. Priorcycle1's approval request NK suggestion was wrong: cancel changes outcome_outbox_id,
   a nonpartial unique key, so request U rather than NK. Correct ownerstepU→requestU→timerNK.
2. Immutable original root ancestry applies to child chains; root successors start their own
   root_run_id/tree. Correct qualification, retaining original-parent identity guard scope.
Updated sectionA goes to third/final bounded source review. No runtime change in this artifact.

## A cycle3 — review_b0_execution_edges_v3

No remaining substantive issues in current sectionA/ancestry/scheduler paragraphs after
source examination at56044f75. Read-only, no tests. B/C and groupdesign excluded. StopA at3.

## B cycle2 — review_b0_authority_edges_v2

Both findings accepted after source inspection:
1. Conditional half-open compensation breaker NK follows activity quota and persists through
   script retention/next-task admission; outcome writers retain breaker U.
2. replace_sealed is direct exact secret_versions NK→audit without Secrets/head/membership
   guard, distinct from secret put/delete/provider activation. Narrow the shared mutation row.
Managed replay's prefix correction has its own actual RED and bounded code review; final B
source review checks updated current code and the corrected inventory together.

## B cycle3 — review_b0_authority_edges_v3

No substantive inaccuracies or omissions in current sectionB after source examination,
including the uncommitted managed replay correction. Read-only/no tests. StopB at3.
The replay code itself has separate bounded security review and actual RED/GREEN evidence.

## C cycle2 — actionable source omissions

Both findings confirmed in current source and incorporated: immediate run_active_counts
shared slot NK locks (including predecessor finish → successor references/slot), and declared
Query chain S → resolved run S / direct run S through audit/commit. No counter deadlock
is inferred as an executed result. Final bounded C review follows rerun correction.

## C cycle3 — review_b0_implicit_edges_v3

No remaining substantive source inaccuracies, omissions, unstated constraints or dangerous
false claims within bounded sectionC/evidence boundary. Read-only source examination, no tests.
StopC at3. Early rerun identity correction separately completed3bounded plan/code cycles;
actual SQLdeadlockRED/GREEN recorded in its own evidence. This inventory does not accept B1,
a new group lock order, old writers, global deadlock freedom or nested runtime.
