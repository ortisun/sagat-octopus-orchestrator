# B2a bounded schema and operation reviews

Schema cycle1 fresh-context Astra/xhigh read-only: no concrete constraint/implicit-lock/retention/
privilege/projection/API false claim found in the schema decision. Decision review, not tests or
implementation acceptance. No changes to the schema artifact after that review.

Operation cycle1 fresh-context Astra/xhigh read-only: actionable recovery wording. Same-command
reconciliation may wait on retained own chain/run before G NOWAIT, so an unresolved first attempt
does not necessarily produce Busy. Timeout/cancellation before acquiring guards leaves original
commit Unknown. Corrected contract and split same-command prefix wait from distinct-member G Busy
barriers. No edits/tests by reviewers; owner's current-review choice applies, no external CLI.

Operation cycle2 fresh-context Astra/xhigh read-only: no remaining concrete issues, including
revised step5 timing (validity at postlock sample, not continuously until commit). Stopoperation
at2; schema at1. No edits/tests or external CLI. No whole-protocol/production acceptance.
