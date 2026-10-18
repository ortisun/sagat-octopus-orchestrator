# Exact-path chain prefix review

Scope: PathExecutionStore's two mutation entry points acquire their applicable chain before own
run, same transaction. No new public composition or chain finality authority.

Two read-only fresh-context gpt-6-astra/xhigh reviews; no nested agents, tests or external CLI.
The owner selected the current review earlier; no external CLI was authorized/invoked.

1. review_exact_path_chain_prefix identified three actionable oracle/precondition details:
   real initialization requires a deadline and increments the journal revision, so set deadline
   and load snapshot after initialization; SkipLocked post-return NOWAIT only proves no retained
   run lock, not acquisition order; probe the chain while returned transaction is alive to prove
   lifetime. All accepted: use real initializer, observed Wait barrier per entry, independent
   chain/run contention while alive and successful probes after commit.
2. review_exact_path_chain_code inspected actual final production/test diff and found no concrete
   issues. Reviewer ran no tests and did not claim pending green results.

A trusted internal scope+chain fixture exercises the storage invariant. Publication remains
closed for that composition, and PathExecutionTx's supported event/finality bounds do not expand.
Other primitive adapters/direct-run Update and post-wait primitive fences remain separate B0 work.
