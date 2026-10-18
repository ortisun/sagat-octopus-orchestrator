# Original scanned lease expiry: bounded decision review

CLAIM: A public binding narrowing can preserve the actual report activity while native session invalidation expires its original lease; current full Live(false) validation must reject that stale evidence after checked-path drop.
WHY: Previously checked metadata alone cannot establish current lease validity.

The55-line extracted decision and accepted contract were adversarially reviewed in fresh GPT-6 Astra xhigh context: None. Plan by GPT-6 Astra, implementation draft assigned to GPT-5.6 Sol in temporary mirror only. Root owns application and execution. User previously declined optional external second review; none invoked. No tests/builds were run for this decision.

This deliberately uses the actual full checkpoint restore barrier AFTER Run lock acquisition, not a literal before-begin race. It proves expiry through a new DB-time lease query while token/status/attempt/generation remain identical and report activity remains bound. Generation replacement, terminal scope, no-secret LeaseMatch policy and broader current-fence matrix remain separate.
