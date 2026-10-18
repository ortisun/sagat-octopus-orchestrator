# Scoped ADR-0037 review

Claim under review: the existing early parent identity KEY SHARE can be retained without
granting parent aggregate mutation authority, under the stated current-writer preconditions.

Two fresh-context, read-only gpt-6-astra/xhigh reviewers; no nested agents or external CLI.
The owner previously selected the current review, so no external CLI was invoked.

1. review_parent_identity_guard_adr: actionable missing ancestry precondition. The guard resolves
   the initial segment while successor creation copies the current head; initial/current/link
   parent identities must agree and remain immutable. FKs alone do not enforce this. Accepted:
   ADR now states this as a writer invariant, names current writers and requires revisiting for
   reparenting, migration or another initializer. Added positive depth-one/depth-two assertions.
2. review_parent_identity_guard_ancestry: no remaining concrete precondition, authority, ordering,
   wording or test-oracle defects in the revised scoped ADR and assertion blocks.

Reviewers ran no tests. The root ran the recorded controls and ancestry tests. No full B0 DAG,
capacity SQL design, rollout, runtime eligibility or general nesting approval is inferred.
