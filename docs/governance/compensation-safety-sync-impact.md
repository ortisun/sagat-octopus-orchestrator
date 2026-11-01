# Compensation safety — proposed Constitution sync impact

Status: PROPOSED adoption patch. Owner approved product direction on 2026-11-01, not this final
wording, implementation, provider qualification or release. Baseline is the locally applied B01
Constitution 2.0.0; normal PR approval/merge governs adoption.

## Version and force

Proposed version 3.0.0: principle III materially changes the absolute intervention rule and permits
narrow qualified compensation exceptions. All other principles and governance authority remain
unchanged. The preparation date is 2026-11-01; any actual-adoption-date substitution must be recorded.
If B01 and this amendment are combined before adoption, rebase the version/date and review the final
combined diff; do not pretend either draft version was already formally ratified.

## Coordinated artifacts

- ADR-0076: proposed refinement, not wholesale supersession of ADR-0004/0005/0010/0030. Additive
  relationship notes preserve historical rationale and the previous partial supersession in ADR-0030.
- Feature 037: approved direction expressed as draft requirements; 003/014 dependent specs point
  to the target and preserve old evidence. Detailed plan/tasks are still absent intentionally.
- Saga architecture/runbook and SDK specification: current versus target explicitly separated.
- Architecture principles: future checks identified as unenforced, with no fake runtime guarantee.
- Owner decision, research approval notes, index, roadmap and traceability are recorded separately
  in the working tree under the owner's direction-recording authorization.

## Remaining gates

Effect classification, qualification identity/authorization/revocation, pending-result reporting,
contracts/client shapes, storage/transaction changes and exact tests need a reviewed plan. No provider
is qualified. New tests must fail for the right reason before behavior changes. The baseline 94
passing domain tests are characterization only, not new acceptance or provider proof.

## Risks and reversal

Review can retain resources; fail leaves operator follow-up against retained evidence; provider
qualification can become stale. No new infrastructure, production cost or SLO is assumed. Retain
fresh-install cutover and irreversible external-effect limits. Before adoption this exact patch is
reversible without rewriting historical content; after adoption use an explicit reviewed amendment.
Reverting prose does not undo runtime behavior or provider effects. No CI/hook restriction is removed.
