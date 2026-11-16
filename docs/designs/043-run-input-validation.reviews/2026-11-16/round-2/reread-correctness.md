# Targeted re-read of design 043 for approval: correctness (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: 4b6eb1a649b281660c4622e20d0b348322a7d8a4
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The fourth targeted
re-read of the changes since 2606e1d5, on the approval path the owner chose (OD-2026-11-16-3: fix, re-read the fixes,
accept the rest), not a full review. The reviewer read files only and ran nothing.

The history of this re-read: `reread-1-correctness.md` (Majors 5), `reread-2-correctness.md` (1), `reread-3-correctness.md`
(0), each fixed before the next; this is the last. The catalog probe, the text fixes and the new unit test are fine; no
Blocker, no Major.

## Nit

1. Re-read 3 Nit 1 is half resolved: the new unit test checks the exit status and the printed line of an `interrupted`
   version, but no test runs the scan arm that fills the list (`inventory.rs`, `Err(PrecheckRefusal::Interrupted)`).
2. D043-188 says an implicit panic prints numbers only; a `str` slice off a character boundary quotes the string. The one
   slice of schema text (`pattern.rs` `translate`) cuts at the spans of a one-byte `.`, so it cannot fire today.
3. The inventory's module doc still gives "the command itself failed" as the only cause of exit 2.

Counts: Blocker 0, Major 0, Nit 3
