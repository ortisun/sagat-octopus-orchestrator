# Targeted re-read 4 of design 052: security (2026-11-26)

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: ff8681c434bd2a4fdc9f77d8605d869f590cc7f9
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation of the
fixes of `reread-3-security.md` (both resolved), not a full review. The reviewer read files only and ran nothing.

No Blocker, no Major.

## Nit

1. The egress and credentials-refusal checks are named in a target whose selector ignores the chart's files, so a pull
   request changing only the network policy template would pass. Fix: the checks in the chart's contract test, and a
   selector rule for those templates, with its test.

Counts: Blocker 0, Major 0, Nit 1

Fixed at the next commit.
