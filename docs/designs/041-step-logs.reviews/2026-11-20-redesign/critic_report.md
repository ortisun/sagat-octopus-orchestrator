# EGM stage 7 (critic review) of the 2026-11-19 revision of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 4c027adffaa889109a4b776c148d0f11263f9130
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 2, Major 37, Nit 25
Note: stage 7 only (the owner's brief asked for stages 7 and 9), FeatureRange lines 1-1499 of commit 4c027adf, three goldfish dispatched directly (egm-design, egm-structural, egm-style) with the feature-spec standard; recorded condensed by the author. Counts are the sum of the three (design Blocker 1, Major 8, Nit 5; structural Blocker 0, Major 11, Nit 7; style Blocker 1, Major 18, Nit 13).

## egm-design

- K-1 Blocker. A full carry (500 records) holds the run row and the tenant row for long; the gate never measures it. K-2 Major. The carry's lock waits have no timeout under the run lock. K-3 Major. Four round trips under the run lock, measured only on a local database. K-4 Major. The fan-out (scoped path) is not measured by the gate. K-5 Major. Purge load on the tenant row in steady state is unmeasured. K-6 Major. Capacity quotes the 1 × 16 S only and hides the failed ticket gate; defaults contradict. K-7 Major. FINAL_RECORDED cannot reach a batch. K-8 Major. A lost carry leaves "no log" with no count. K-9 Major. The carry skips the ingest's role refusal. K-10 to K-14 Nit: O041-026 against Alternatives; equality counts; the re-run missing from timing claims; stale wording on completions with output and the rollout; the carry's idempotency.

## egm-structural

- ST-1 Major. Three or six equalities; 1 + k or 2 + k rows. ST-2 Major. The re-run missing from cost and timing statements. ST-3 Major. A short step whose records exceed one carry has nowhere to put them. ST-4 Major. FR-041-004's FINAL_RECORDED. ST-5 Major. The Overview's "only". ST-6 Major. Alternatives against O041-026. ST-7 Major. Expiry defined by the last batch. ST-8 Major. O041-021 filed as open. ST-9 Major. data-model §6 names T041-31. ST-10 Major. G041-018 and G041-019 missing from the design. ST-11 Major. The rollout mitigation no section backs. ST-12 to ST-18 Nit: one function against four methods; section 5 misquotes FR-041-008; Capacity's ten tickets read as default; a broken "Cost" reference; ADR-0095 missing from ADR lists; the revision paragraph lists no sections and cites a folder dated later; batch-only wording in FR-041-018/020, sections 3 and 4.

## egm-style

- SY-1 Blocker. Short steps and overflow batches contradict each other. SY-2 to SY-19 Major: five wordings of "short step"; "It" pointing to the RPC; the Overview's "only"; the carry's size and its derivation; a garden-path sentence in FR-041-004; two definitions of "final mark"; three or six equalities; the re-run missing from cost claims; "costs nothing else"; "the bound in time" names no bound; O041-026's trigger stated three ways; Open owner questions against their header; unclear references in the first Open Question; the resend rule; Capacity's numbers are not defaults; ADR decision 1's exceptions; one function or four methods; the Risks row's lock timeout bounds a wait, not a hold. SY-20 to SY-32 Nit: wording.
