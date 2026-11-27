# Targeted security re-read of design 050 for approval, round 13 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 1854e0bca46de49b47d983fbc250d678911af029
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the changes since round 11 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found admission before the cursor, no on-demand key read, the `SELECT`-only grant and its lock, the cursor replay after a lost permission, the metric labels, the Studio's clearing on `401`, `403` and `404`, Architecture decision 0144 decision 5 and the Complexity Tracking row sound.
Counts: Blocker 0, Major 0, Nit 1

1. **Nit. Technical Plan 6, "Cursor sealing".** "At most 19 digits" lets key ids overflow the `bigint` and leaves the answer (`400` or `500`) to the builder; the encoding of the HKDF `info` and of the associated data, and what "caller" is, are unstated, so different tuples could concatenate alike. Fix: a positive `i64`, else the malformed-cursor `400`; length-prefixed fields; the caller is the principal's `Subject`; the overflow case in `an_unknown_key_id_is_a_malformed_cursor_without_a_read`.
