# Targeted re-read of design 044 for approval: security, confirmation (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 14513c8533b0a0a475d77fe21f3886c4e07c52cc
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The approval path of
OD-2026-11-25-2: one targeted re-read of round 11's fixes, every security finding fixed (never settled), re-read until
a pass found no Major (passes 1 to 6 beside this file); this confirmation reads the text changed since 006e435d. All
three Nits of the sixth pass are closed. No finding on the tenant boundary, authorization, secrets, least privilege,
injection or races. The two Nits below stay open for the build (Task 044-9 and Task 044-2), recorded in the register.

## Nit

1. No test holds that a counting refusal reason is not replaced within a lease; the later refusal should move
   `unavailable_at` only.
2. `COMMAND_VALUE_RELEASED` is missing from the error-code table of section 13.3, so from the error catalog.

Counts: Blocker 0, Major 0, Nit 2
