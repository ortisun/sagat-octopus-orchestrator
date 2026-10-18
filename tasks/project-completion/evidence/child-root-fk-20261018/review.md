# Bounded admission correction review

Two fresh-context read-only GPT-6 Astra/xhigh cycles; no external CLI, following owner selection.

Cycle1 initially reviewed parent reservation NO KEY UPDATE alone. Findings: input/state retention
before the tree lock can block a handoff that owns the tree; retention before active-run quota can
also block a handoff across trees or against root creation. Reconciliation: move both existing
retention/verification blocks after ensure_room_for_run. Reviewer inspected remaining parent
writes/triggers and found no late parent lock upgrade in the final candidate. The adjustment
preserves the tree-first/subordinate-resource requirement and adds no ADR divergence for this
specific correction. Existing parent-guard/legacy-settlement questions remain documented separately.

Cycle2 inspected the actual execution.rs patch, schema, helper dependencies and both barrier tests;
reported no substantive issues. Reviewers did not execute tests. Root executed the recorded RED,
GREEN and broader gates; their results are independent of the review verdict.
