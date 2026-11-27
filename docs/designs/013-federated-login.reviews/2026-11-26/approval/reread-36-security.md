# Targeted security re-read of design 013 for approval, round 36 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: fb2994054b68d7edad00b970c6fd6627708781ae
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-35 fix (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the sentence consistent with the rest of the document and the type list matching the material-bearing types.
Counts: Blocker 0, Major 0, Nit 2

1. **Nit. Technical Plan 12, *Errors and printing*.** The search for the non-base64 material looks only for encodings of the distinctive bytes, so a parse error copying the malformed stored text verbatim passes. Fix: build that material as the base64 of distinctive bytes with one invalid character inserted, and search for the stored text verbatim and any 16-character window of it.
2. **Nit. Same place.** The search covers only the bytes the test chose, never the derived `cli-refresh` purpose keys that do the work. Fix: derive them independently in the test and search for them in every listed form.
