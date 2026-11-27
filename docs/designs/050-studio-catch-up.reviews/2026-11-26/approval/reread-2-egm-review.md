# egm-review re-read of design 050 for approval, round 2 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: e1537935
Reviewer: egm-review (re-read for approval, egm-design)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: a design-critic re-read of the text changed since round 1 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found its round-1 findings closed and the client, query settings and `KeyRing` claims accurate.
Counts: Blocker 0, Major 2, Nit 4

1. **Major. Requirement 050-010, Technical Plans 6 and 9, Architecture decision 0144.** `SAGAT_SECRETS_KEYS` also turns on secret storage and, outside development, a TLS condition at start-up, and Helm feeds it to the engine too; turning the inbox on would turn those on, and the decision record weighs only "one more key". Fix: record the coupling, or reconsider a cursor-only key.
2. **Major. Task 050-4, Technical Plan 6, with design 013.** The shared `derive_purpose_key` has a closed label enumeration here and a free label in design 013, so its signature depends on which task lands first. Fix: one signature in both designs.
3. **Nit. Technical Plan 6, Architecture decision 0144 decision 5.** The cursor's wire form, and that opening derives from the cursor's own key id, are unstated. Fix: state them.
4. **Nit. Technical Plan 6, Requirement 050-010.** The ring check is not ordered after admission, so another tenant's namespace could answer `503` instead of missing. Fix: after admission; a battery case without a ring.
5. **Nit. Constitution Check, Complexity Tracking.** `aes-gcm-siv` and a direct `hkdf` dependency are new and unrecorded; the sealed cursor has no complexity row. Fix: record them.
6. **Nit. Requirement 050-010, Technical Plan 6.** A broken requirement sentence, an unparseable sentence about a `400`, and an overloaded cursor sentence. Fix: rewrite.
