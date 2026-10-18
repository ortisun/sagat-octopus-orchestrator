# Bounded native source transaction decision

Claim: the contract fixes bounded source metadata reads, exact original-run guards, replay/CAS,
typed refusal and atomic metadata recovery without claiming actual producer preservation.
Fresh Astra/xhigh cycle1: P2 valid/actionable. LIMIT10 bounds rows, not bytes: steps.activity has
no database length CHECK. Add SQL-side node64/activity128 projections and explicit validity flags;
reject oversize as ContradictorySource, not NULL-compatible. Lease/wait absence is projected as
booleans, not transferred capability fields. Journal scalar/type/length/cardinality guards remain.
Cycle2: no concrete issue found in bounded contract. Stop at no findings. No tests by reviewers.
Root sole writer; existing user choice keeps current review, no external CLI. Source implementation,
producer barriers, genuine uncertain-COMMIT/restart and observations are still required. Release
schema stays SELECT-only; disposable native tests may explicitly grant INSERT to actual sagat_app.
