# Targeted re-read 8 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: f3180b01e59237b1732133cd7d8138b2412f1309
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. An eighth targeted re-read
of the fixes of re-read 7, not a full review. The reviewer read files only and ran nothing.

Re-read 7's three Nits are resolved. The edits add no tenant, authorization, secret, injection or race exposure; the
constant answer now covers the authenticator and the webhooks.

No Blocker, no Major.

## Nit

1. (Suspicion.) The design does not say how PostgreSQL picks up the drill's file: an `hba_file` replaces the whole
   policy and needs a restart through Patroni's parameters; removing a line without a reload leaves it in force. Fix:
   `include_if_exists` in the reference's `pg_hba`, before any rejecting line; the trap removes, reloads and asserts a
   refusal from the probe's subnet.

Counts: Blocker 0, Major 0, Nit 1

Fixed at `f2330653` and worded at the next commit.
