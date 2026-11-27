# EGM re-read of design 040 (2026-11-26)

Doc: docs/designs/040-ocsf-event-standard.md
Reviewed-commit: d70ba587a14f0eed3d2517afc37285a23d1f80aa
Reviewer: egm-implementation-readiness
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 3, Nit 3

Round 1 readiness: 12 fixed (1 to 7 and 9 to 13), 1 partly fixed (8). The DDL, the module paths and the split of Task 040-11 into 040-11, 040-23 and 040-24 are now in the plan. The port signatures are still missing and the operator-side ports were never added; see finding 2.

Round 1 critique: 7 fixed (1 to 7). One thing left over from 4: the Terms entry still disagrees with Requirement 040-013 (Nit 4).

1. **Major: The restore guard is specified and tested, but no task builds it.**
   - Where: section 4, lines 415-427; Success criterion 040-007; Tasks 040-9, 040-11, 040-13 and 040-23.
   - Task 040-13 only tests the restore. No task builds the `control-plane audit-export rebase` command, its `max(tx_id) >= xmax` refusal or the time-based reset of each cursor. No task builds the `SECURITY DEFINER` function that returns `system_identifier`, or adds the `cursor_cluster` check to the pass.
   - Fix: add one task for the function, the check in the pass and `rebase` covering sinks, operator streams and the discovery cursor. Order it before Task 040-13.

2. **Major: The ports are incomplete, and the operator-side work has two owners or none.**
   - Where: lines 579-583, 524-539 and 572-576; Tasks 040-11, 040-24 and 040-12.
   - `AuditExportStore` is a list of names with no signatures. It has no write for a retryable failure (failure count, `next_attempt_at`, `STALLED`), the return to `ACTIVE`, or the refused-in-window counter, although step 4 lists those writes. No port covers `audit_operator_config` (the start-up lock, revision and fingerprint compare, discovery cursor). `audit_operator_export` has no refused-window columns.
   - Tasks 040-24 and 040-12 both claim the operator's row in `audit_operator_export`, and neither owns the config row or discovery.
   - Fix: write the trait signatures, add an operator store port, and give each operator artifact to exactly one task.

3. **Major: When the operator sink applies `start` is ambiguous and can skip records.**
   - Where: lines 533-537; Requirement 040-009; research Decisions D040-016 and D040-017.
   - The text says an engine with a higher configured revision "applies `start` … once". The decisions say `start` applies only when it changes (or only on a real change of destination).
   - Read literally, raising the revision to rotate a credential while `start: now` is configured skips every tenant's backlog. That breaks at-least-once delivery.
   - Fix: state the condition (the configured `start` differs from the stored `start_at`) and test it.

4. **Nit: Stale cross-references in the changed text.**
   - The Terms entry for the authorizing subject (lines 45-46) leaves out the home namespace and key rotation that Requirement 040-013 includes.
   - Requirement 040-010 (line 136) still names `has_effective_payload_reveal_grant`, while section 5 names `holds_reveal_grant_in_every_namespace`.
   - Task 040-4 still says "`unmapped` keys closed per action". Task 040-9 says "two migrations" where section 4 now describes five files.
   - Success criterion 040-002 says "contract job", while Task 040-6 says "contract suites".

5. **Nit: Task 040-25 has no error code for a `startFrom` creation refused under `rebase`'s guard (line 413).** It also does not say what a sink in `CURSOR_INVALID` answers to `/resume` or `PATCH`.

6. **Nit: The halving order is implied but not written (lines 512-518).** After a lone left record is refused, the plan should say:
   - that the right sibling is sent next;
   - that the refusal and the cursor move past both commit in one transaction;
   - whether "accepted" means the whole sibling part when that part is itself split.

Only the `rebase` task (finding 1) blocks a clean handoff to two builders; findings 2 and 3 are contract gaps that would make them guess.
