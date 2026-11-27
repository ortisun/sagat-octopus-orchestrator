# Targeted re-read 2 of design 054: security (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: 9f803d8c78da063d0953a412574ebe3b75041a1c
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. A targeted re-read of the
answers to re-read 1 and of the text changed since, not a full review. The reviewer read files only and ran nothing.

Re-read 1 Nits 1 to 3: resolved (3 in the design's text, not yet in the tasks: finding 1). The new refusal order opens no oracle
across tenants; the version tolerance accepts only `signal.buffered`; a mislabelled allowlist entry still fails closed on the
check on `runs`.

Counts: Blocker 0, Major 0, Nit 1

## Nit

1. The tenant aggregate is designed but no task builds or tests it; the metric label and the log name only the namespace; the
   default is ambiguous. *Answer: fixed:* Task 054-4 builds both resources and the trigger charges both; Task 054-8 tests two
   namespaces over the tenant's ceiling; the label is `refused_pending_quota` and the log names the scope; the default is the
   smaller of 1 GiB and a tenth of `tenant_content_bytes`.
