# Targeted security re-read of design 050 for approval, round 3 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 2d71be9c
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the text changed by the architecture, correctness and egm-review re-reads (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the ended-run guard, the per-run flag, the single policy resolution and the capability names sound.
Counts: Blocker 0, Major 1, Nit 2

1. **Major. Technical Plan 6, Architecture decision 0144 decision 5.** Sealing a request-driven cursor under the `SAGAT_SECRETS_KEYS` key spends the nonce budget the secrets key relies on ("far below 2^32 encryptions"), with known plaintexts and no key separation. Fix: a purpose key derived by HKDF-SHA256 with labels for the two lists.
2. **Nit. The same places.** Without a key ring, and for a cursor that does not open, nothing is specified. Fix: `503` and never an unsealed cursor; bound the parameter; one `400` for every unopenable cursor.
3. **Nit. Technical Plan 6, Architecture decision 0144 decision 4, Risks.** A non-full page with a cursor tells a folder-restricted caller how many hidden rows its window held; allowed as an aggregate by feature 029 but stated as nothing. Fix: say so.
