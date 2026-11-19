# Targeted re-read 2 of design 043: egm-review lenses (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: d890b9263cd8b84a7156edf66c3f08224b0d1617
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the author's summary of two lenses run on the changed text (egm-implementation-readiness and egm-comprehension), a
second targeted re-read, not a complete run of the five stages.

## egm-implementation-readiness

The three Majors of re-read 1 are resolved; every cited file and test exists. Nits: 1. T043-15 says the Rust field/rule
test is in `operations.rs`, it is in `src/error.rs`; 2. T043-13 does not cite the CLI diagnostics test; 3. T043-5 names no
problem-shape test; 4. the suite rule leaves the mode, the Studio and `docs/reference` unmapped; 5. "their down migrations'
guards" still plural.

## egm-comprehension

Major: 1. Technical Plan 5 and the gate's sketch still say the recheck counts only a refusal, against Technical Plan 7.
Nits: 2. G043-057 outside the declared range; 3. "their down migrations" in three places; 4. Technical Plan 3's new
sentence hard to follow; 5. the stored parse paths of T043-13 against Technical Plan 8; 6. "count", "lately" and "live
schedule" in FR-043-018; 7. "O-question" undefined.

Counts: Blocker 0, Major 1, Nit 11
