# Targeted security re-read of design 050 for approval, round 12 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: e2601032
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the fixes of the third correctness and egm-review re-reads (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found no finding on the tenant binding of cursors, the table lock, the `SELECT`-only grant, the printing rules, the read rates or the metric labels.
Counts: Blocker 0, Major 0, Nit 3

1. **Nit. Technical Plan 6, "Cursor sealing"; Architecture decision 0144 decision 5.** One on-demand read budget per replica per minute, shared by every tenant, can be spent by any caller with a forged key id; after a `rotate` a replica that already read the new key seals with it, and its cursors fail with `400` on replicas that have not, for up to a minute, degrading other people's inbox; the key id's parsing bound is unstated. Fix: seal only with the newest key older than one timer period plus a margin, so every replica knows a key before a cursor uses it and an unknown id is a plain `400` with no on-demand read; parse the key id as a bounded positive integer before any lookup.
2. **Nit. Technical Plan 6, "Cursor refusals"; Task 050-4.** Whether the cursor is opened before or after namespace admission is unstated, so an unauthorized caller could drive key reads and the `503`. Fix: admission runs before the cursor is decoded and opened; `a_cursor_refusal_never_precedes_the_namespace_refusal`.
3. **Nit. Technical Plan 6, "Studio"; Task 050-10.** "Keeps the last list" on a failed refresh does not say what a `401`, `403` or `404` does, so a builder may keep showing items the person can no longer read. Fix: `401`, `403` and `404` clear the list; `a_lost_permission_clears_the_inbox`.
