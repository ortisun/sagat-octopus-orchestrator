# Bounded test-oracle reviews — cycle1

Two independent fresh read-only Astra/xhigh reviews, split by artifact. All findings classified
valid/actionable; draft corrections are in progress, no final acceptance claimed.

Binding/variant group:
- Exercise each nullable or forbidden variant field independently; several simultaneous wrong
  values can mask a missing predicate.
- Use a real non-chain original Run so the positive cannot pass a wrongly targeted chain FK;
  independently test existing foreign tenant/namespace and absent Run plus every receipt qualifier.
- Check lower/upper index and owner bounds with valid endpoint controls, and scoped revision0.

Guard/permission group:
- Assert the exact effective INSERT column set is the original20, rather than checking only two
  of four denied new columns (combined denied insert can mask accidental partial grants).
- Mutate only FINISHED.finished_at to isolate retained-row immutability from irreversible-state
  rejection; ACTIVE-field identity and replica checks remain separate.

Before this cycle, root also corrected the draft's FINISHED-only identity mutation oracle: old
finished-row immutability masked missing new ACTIVE identity protection. Tests now target ACTIVE
rows; this cycle adds the further precise controls above. SQL pair review independently found no
bounded issue. No reviewer ran tests or edited files; user chose current review, no external CLI.
