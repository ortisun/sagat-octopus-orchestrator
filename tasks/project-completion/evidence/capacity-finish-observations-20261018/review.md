# B2f bounded observation review

Claim: returned native Finish outcomes are observed with a closed safe vocabulary and confirmed
first-commit transition counting; original transaction semantics remain unchanged.

Fresh Astra/xhigh cycle1: P2 valid/actionable. The observation ACK-loss test permitted the fast
server to reach ReadyForQuery before the relay's required disconnect. Added a test-only deferred
receipt advisory gate held before invocation and released only after lose_reply confirms EOF.
Also strengthened exact per-group span outcome counts and explicitly prohibited transition labels.
The initial passing run did not demonstrate absence of that race and is not treated as final.

Cycle2: P2 valid/actionable. Span::current recorded on the caller when filtering disabled the
Finish child. A new real closed-runtime-pool regression reproduced caller_pending becoming
storage_error (RED,2.20s). Fixed by recording on the captured Finish span handle and instrumenting
its clone. The filtered child cannot mutate caller outcome/status and metrics remain enabled.

Cycle3: no remaining concrete correctness/security/false-proof findings under the bounded contract.
Stop: three cycles completed with final no findings; no fourth review. All reviews read-only,
root sole writer/test executor. Sol/high supplied independent read-only source/API draft. Existing
user selection retains current review; no external CLI. No production caller or nested activation.
