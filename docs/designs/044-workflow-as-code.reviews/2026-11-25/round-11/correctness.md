# Correctness review of design 044, round 11 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 9bdb707d84f3fef3c190979cde128256f2b8159a
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 10's Majors 1, 3
and 4 are closed and 2 partly closed.

## Major

1. A run near its 4 MiB cannot end or continue as new (a terminal output over the bound reviews with no retry, and a
   child cannot be reset): do not judge a terminal output or a continue-as-new input against the run's bytes; an update
   result over the bound fails the update only; a test.
2. Command-value blobs count against the tenant's 64 uploaded-not-committed blobs, so one fan-out or a few held
   deferrals block every upload in the tenant: exempt them and bound them per run; pack or spill only large values;
   name the cause; a test.
3. Naming a history blob cannot pass the descriptor check unless its bytes are canonical: allow only blobs written
   under the run's workflow lease (or held), or require the SDK to verify; a test.
4. A replay-check lease that is claimed and expires has no end, so a poison item keeps a move job open: a lease length,
   an end after three expiries, a lone retry; a test.

## Nit

5. Residue of round 10's Major 2: the blob list and tests for `InlineValuesExceeded`.
6. `COMMAND_INVALID`'s retry in the cause text but not in the Remedies column.
7. The 3 MiB headroom rule in three places.
8. The SDK's allowance counts one consumption and omits update completions.
9. The first history page and a 256 KiB inline payload.
10. The activation-window test's oracle comes from the same projection.
11. The activation deadline's default against the minimum workflow task timeout.
12. Step 2 should say the completion takes the chain lock before the run, as every run transaction does.

Counts: Blocker 0, Major 4, Nit 8
