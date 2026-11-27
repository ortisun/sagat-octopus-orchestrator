# Targeted architecture re-read of design 050 for approval, round 4 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 98609559009f7b0d30699273f9c602559f831a1a
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a re-read of the changes since round 3 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept.
Counts: Blocker 0, Major 0, Nit 4

1. **Nit. Technical Plan 6, "Cursor sealing".** A failed timer read keeps the cached keys and emits nothing, so a replica sealing under a deleted key is indistinguishable from tampering. Fix: `inbox_cursor_key_reads_total{outcome}`, a gauge of the time since the last successful read, a failed read logged once per run of failures, a named test.
2. **Nit. Technical Plan 6, the port `InboxCursorSealer`.** The `key` and `scope` types are unnamed; a method generic over the key would not be object-safe. Fix: `InboxKey { at, id }`, `InboxScope { list: InboxList, tenant, namespace, subject }` with a closed `InboxList` picking the label; `an_approval_cursor_does_not_open_on_the_publication_list`.
3. **Nit. Risks and Mitigations, row 2.** It still speaks of a per-tenant key. Fix: a key derived per cursor (random salt, tenant in `info`).
4. **Nit. Technical Plan 6 against design 013's attempt cookie.** Two security-critical copies of one envelope in two crates, and `hkdf` added twice. Fix: `hkdf` in the workspace's dependencies; one shared codec, or two copies tested against one malformed-input table.
