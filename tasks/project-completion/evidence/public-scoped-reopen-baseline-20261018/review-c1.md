# P0b C1 reconciliation

Three fresh-context GPT-6 Astra xhigh reviewers assessed bounded helper/canonical, public setup and first-report/replay units. No external second review was requested again: owner declined it earlier.

Actionable findings:

1. Setup and report reviewers independently found the same missing active-writer witness: an idle supplied-backend probe does not tie that backend to public CompleteStep. Counted once; add exact receipt advisory gate and active blocked PID observation.
2. Witness reviewer found omitted worker_sessions.expires_at and tenant_iteration_usage. CompleteStep does not call use_session; preserve the entire session row including unchanged live expiry, plus the genuine active_iterations=1 ledger and complete row equality.
3. Report reviewer found candidate-digest-only checks permit another event payload/reference or orphan payload. Require exact scoped completion event payload_digest NULL, full payload/retained-usage equality and preserved sixteen original output references.
4. Root initially suspected a `version` alias/column collision. Reinspection on2026-10-23 disproves it: workflow_versions has version_number, not version (unlike secret_versions). Withdrawn as a root context error, not an actual defect. Use an explicit version_row alias and a full document/id object assertion for readability and a stronger positive witness; no runtime failure is attributed to the old alias.
5. Root extends current-authority capture with binding-pool grants and worker drains. No allowed session-renewal delta is needed for this operation.

Contract misread: setup prompt ambiguously mentioned original JWT. The accepted contract requires the exact original CompleteStep/capability/session/subject with a CURRENT JWT signed by the reopened test IdP, not preservation of the original signing key. Reviewer withdrew this finding after clarification; no code change for it.

Canonical receipt scope/fingerprint, bounded scheduler setup and cleanup received no other findings. Corrections are a TMP-only C2 draft; C1 strict compile is preliminary, not behavioral acceptance. No C1 runtime RED or COMMIT-loss result is claimed.
