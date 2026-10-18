# Actual later saga planner preservation review

Focused source inventory by GPT5.6Sol/high /root/inventory_leaf_effectful_later_saga: supplied
public other-body effect fixture, actual drain/planning/refund sequence, positive saga assertions
and current planner owner filter. Read-only; no tests/edits. Root implemented and ran the unit.

Fresh-context Astra/xhigh /root/review_leaf_saga_c1 found an actionable stage oracle gap:
certifier-first initially observed only the scheduler's preliminary restore transaction. That
read transaction drops before actual apply_addressed_if. Corrected with observed Run queue:
preliminary restore FIRST, certification SECOND behind held Run. Release hold and poll both until
certification reaches actual deferred COMMIT, proving the restore dropped its Run; then observe
the planner's second full-Run acquisition behind that actual committing backend. Premature planner
completion fails the test. Planner-first already waits inside actual deferred saga planning COMMIT.
Fresh-context Astra/xhigh /root/review_leaf_saga_c2: no concrete remaining stage/order/oracle issue.
Stop after2bounded cycles. Both reviewers read-only, no tests/edits. No external CLI; user declined.

Scope: real public ordinary selected source plus known completed compensatable work in later
collection; fail/READYcancel; actual header/entry/event and refund to terminal, selected source/
saga absence/canonical identity/count stable. No all-producer/chain/C0/release acceptance.
