# Chain post-wait scheduler fencing reviews

Claim under review: existing chain mutation ports must evaluate scheduler expiry after
acquiring their current own-authority row lock, within the same transaction.

Two fresh-context read-only gpt-6-astra/xhigh reviews; no nested agents/tests/external CLI.
Owner earlier selected current review; no external CLI authorized/invoked.

1. review_primitive_postwait_fences: actionable proposal gaps:
   - helper normalizes oversized u64 generation to None, unlike chain's existing storage error;
     retain number(...) conversion and test prior error compatibility.
   - PgConnection generalization would admit autocommit between guard and clock; retain
     Transaction argument, including private chain wrappers.
   - private fixture silently used owner pool despite runtime option; use execution_pool and
     require observed sagat_app wait when requested. Exercise evaluated and finalization ports.
   All reconciled. Existing evaluated transaction_now after incoming link remains untouched.
2. review_chain_postwait_fence_code: no concrete defect in actual five-file production/test diff;
   own authority, generation error, transaction typing, unchanged-row barrier, fixture pool and
   retry/replay oracles examined. Reviewer ran no tests and inferred no GREEN result.

No full B0/whole nested protocol acceptance. Other collection/loop/approval helpers stay pending.
