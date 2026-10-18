# Direct run Update chain prefix reviews

Two fresh-context read-only gpt-6-astra/xhigh reviews; no nested agents, tests or external CLI.
Owner selected current review previously; no additional external review invoked.

1. review_direct_update_chain_prefix: no concrete implementation issue in scoped membership JOIN.
   Two actionable test issues: replay audits separately (expect two audits after apply+replay),
   and chain_id==run_id singleton cannot prove successor membership lookup. Both reconciled.
2. review_direct_update_chain_code: no implementation defect in actual three-file diff. Optional
   oracle weakness: UPDATE NOWAIT also conflicts with a shared chain guard. Accepted improvement:
   final chain lifetime probe uses SHARE NOWAIT, requiring an exclusive mutation guard. This
   one-line probe refinement was run in the final focused test; production code is unchanged.

The initial failing test observed no original-chain wait, not a PostgreSQL deadlock. The final
protocol preserves pinned Run addressing, earlier-segment errors, receipt/CAS/audit/outbox and
tenant scope. No whole B0 or group ordering accepted. Reviewers did not run tests.
