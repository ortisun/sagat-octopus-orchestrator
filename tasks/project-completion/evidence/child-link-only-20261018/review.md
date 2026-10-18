# Child-chain link settlement review

Two fresh-context read-only GPT-6 Astra/xhigh cycles. No external CLI; the owner selected the
current review. Reviewers ran no tests; the root executed the recorded commands.

Cycle1 inspected the proposed link-first correction and initial link insertion, parent creation
serialization, finality, tenant/parent association and legacy fallback. No production issue was
found. The promised audit oracle was missing: verify_run checks journal/projections/results,
not audit_records. Actionable: add explicit parent run.started/run.cancel_requested counts.

Cycle2 inspected the actual helper, both callers and three trigger regressions, including the
explicit audit assertions; no concrete issues were found. Cancellation must retain exactly one
request audit. Automatic final child notification keeps its existing audit behavior and has one
child.finished journal event; the tests do not pretend it emits a new dedicated audit action.

The regression guard rejects every legacy-flag UPDATE of the already-initialized first child
segment, even an unchanged-value write. The three original failures prove the mixed protocol
writes on real service/scheduler paths; they are not a PostgreSQL deadlock reproduction. The
subsequent correction leaves chained child flags unchanged and preserves legacy fallback.

Existing hold_parent metadata KEY SHARE remains unchanged. Its comment/contract now state the
actual chain→parent key-share→head/link order. This narrow correction removes the legacy flag
mutation mixture, not the strict ADR-0030 guard deviation, full B0 graph or nested capacity gate.
