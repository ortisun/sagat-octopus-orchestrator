# Constitution 2.0.0 (compensation safety) — Sync Impact Report

Status: **adopted when PR #75 merges.** The owner approved the product direction (S1-B/S2-B) on
2026-11-01. The same day the owner authorized the implementation and its adoption, split the
unrelated B01 amendment into its own pull request, and kept in-place upgrade from v0.2
(`docs/owner-decisions.md`, 2026-11-01). The baseline is Constitution 1.1.0.

## Version and force

1.1.0 → 2.0.0, MAJOR. Principle III replaces the absolute manual-intervention rule. It makes review
the default for unknown effects. It allows an explicit, published `fail` for known permanent
compensation failure. It reserves defensive compensation and the safe repeat of an unknown
compensation outcome for a qualified integration. That qualification is not built (US04), so delivery 1
has no exception path. The ratification date (2026-10-11) is kept; the amendment date is 2026-11-01.

## Clause impact

| 1.1.0 unit | Action | Result |
|---|---|---|
| III-1 no exactly-once; critical effect needs idempotency or lookup | KEEP | Unchanged |
| III-2 an unknown outcome is looked up before it is repeated | STRENGTHEN | Unknown outcomes default to consultation or review in every activity, including compensation outcomes and stop/drain paths; risk metadata cannot authorize a repeat or a defensive compensation; repeating an unknown original needs consultation or an audited human decision |
| III-3 compensation is its own step, reverse order, same machinery | KEEP + clarify | Reverse *dependency* order; defensive compensation of an unknown effect needs a version-pinned qualification (none exists yet) |
| III-4 an effect that cannot be undone ends `PARTIAL` | KEEP | Unchanged |
| III-5 a failing compensation waits for a person, never assumed done | RELAX narrowly | The default stays review. Only an explicit saga's published `fail` may end `COMPENSATION_FAILED`, for a known permanent failure, after independent compensations settle. Unknown, accepted and pending outcomes still go to review. Cancellation and deadlines keep a pending review |
| III rationale, sources | UPDATE | Adds ADR-0076 and feature 037 |
| Metadata | UPDATE | 1.1.0 → 2.0.0 |
| All other principles and governance | KEEP verbatim | Including principle IV: committed journals keep their meaning, and v0.2 journals replay (D1810) |

## Coordinated artifacts

- **ADR-0076** is accepted for decisions 1–3, 5, 7 and 8. Decisions 4 and 6 (qualification) remain
  proposed. ADR-0004, ADR-0005, ADR-0010 and ADR-0030 carry additive notes; their history is unchanged.
- **Feature 037** delivery 1 (US01–US03) is implemented, with its plan, tasks and validation report.
  Specs 003 and 014 point to it.
- **Canonical contracts**: the OpenAPI `operateRun` description and the workflow schema describe
  the behavior. No shape changes. `docs/releases/unreleased.md` lists the breaking behavior.
- **Fitness rows** in `docs/engineering/architecture-principles.md`. They include the parity test
  that keeps the unknown-outcome code lists equal across domain, SQL and Studio.
- **Runbooks**: sagas (installation, rollback, review) and scaling (drain workers before a restart).

## Remaining gates

US04: effect classification, qualification identity/authorization/revocation, pending-result
reporting and real provider evidence (T037-032/033). No provider is qualified.

## Risks and reversal

Review can retain resources and grow operator load. The most common source is worker restarts that
cut off in-flight work, so drain workers first. `fail` leaves the follow-up to the operator, who
works from the retained evidence. A qualification can go stale. To reverse after adoption, use a
reviewed amendment. Reverting the prose does not undo runtime behavior or provider effects. Rolling
back to v0.2 is safe only before the first new decision is journaled.
