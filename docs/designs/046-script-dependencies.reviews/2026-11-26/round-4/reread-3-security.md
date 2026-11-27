# Targeted re-read of design 046, third pass: security (2026-11-26)

Doc: docs/designs/046-script-dependencies.md
Reviewed-commit: 1c855ebb1d5b1d1ad76e882382e73adc66e2273e
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output, condensed by the author; every finding and severity is kept. A confirmation of the fixes of
`reread-2-security.md`; the reviewer read the diff of the fixes, the design and its contracts, and ran nothing.

## The previous findings

All six answered: the column-level UPDATE grant; the claim-time ceiling and `forwardedBytes` on failure; only the creating
principal may fail a materialization; extracted entries in a bounded cache with 2 parses per grant; seeded snapshot rows
and `CACHE 1`; the hold at author-lock validation and on the file route.

## New findings

1. Nit. The completion "settles" the claim's charge without saying on which day's row; the job row stores neither the
   claim day nor the amount, so an implementation crediting the completion day would let resolves claimed before
   midnight credit the new day. Fix: `charged_day` and `charged_bytes` on the job, settle on that row only, never below
   zero; a test across midnight.
2. Nit. Parses are capped per grant but not per tenant: one tenant's four concurrent resolves of large documents fill all 8
   slots and other tenants' resolves get `503` and fail. Fix: a per-tenant cap (for example 4) and a retryable `503`.

Suspicion, not verified (Nit): the private file route takes no namespace while the hold depends on the resolving
namespace; an implementation picking any namespace of the tenant would let a namespace with a hold of 0 release the file
for all. Fix: the grant's namespace; a test with two namespaces of different holds.

Counts: Blocker 0, Major 0, Nit 3

## The author's answers (all fixed; Decision 046-072)

1. `charged_day` and `charged_bytes`, settled on that row only, never below zero;
`a_claim_before_midnight_settles_on_its_own_day`. 2. 4 parses per tenant, the `503` retryable;
`one_tenant_cannot_hold_every_parse_slot`. 3. The grant's namespace; `the_hold_is_the_grant_namespaces_with_two_namespaces_of_different_holds`.
