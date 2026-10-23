# C1 bounded reviews

Fresh GPT-6 Astra xhigh witness-extraction review: None. Fresh gate/transaction-selection review: None. Fresh recovery/cleanup review found two actionable witness gaps, reconciled against native source: inspector compared Run revision only with telemetry head, not journal max(seq), before deriving ACK; audit compared with post-commit Run trace without preserving the original trace. Both can permit correlated bad metadata and make the pre-replay ACK oracle weaker than its contract.

C2 adds explicit journal head equality in the independent committed inspector, compares the Run trace with the original snapshot, and binds that original trace for the audit comparison. Gate protocol and child recovery code are unchanged. C1 strict Clippy and unchanged baseline test are retained as preliminary checks; no C1 fault runtime or production RED is claimed.
