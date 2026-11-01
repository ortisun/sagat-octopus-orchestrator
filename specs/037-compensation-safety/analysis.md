# Planning analysis — feature 037

Status: approved delivery-1 implementation is locally validated, with all 32 delivery tasks evidenced.
No release, merge or formal Constitution/ADR adoption is claimed. US04 evidence/design tasks remain
open and do not substitute for later implementation/acceptance tasks after approval.

## Requirement coverage

| Requirement | Tasks | Named evidence purpose (task supplies path/verification) | Boundary |
|---|---|---|---|
| FR-037-001 | T037-001, T037-011, T037-012, T037-014 | Default review across explicit/collection/loop; stop and restart | D1 |
| FR-037-002 | T037-002, T037-015, T037-016, T037-026 | Explicit immutable fail only, no inferred scope policy | D1 |
| FR-037-003 | T037-004, T037-015, T037-016, T037-017 | Known failure vs unknown; truthful terminal retained evidence | D1 |
| FR-037-004 | T037-011, T037-012, T037-014 | Before/after cancel/deadline, no terminal reopen | D1 |
| FR-037-005 | T037-015, T037-016, T037-017 | Independent/dependent order and terminal quiescence | D1 |
| FR-037-006 | T037-003, T037-004, T037-009, T037-010 | False/absent highRisk and all report sources | D1 |
| FR-037-007 | T037-011, T037-012, T037-013, T037-014 | Lookup/review persists across stop and continuation | D1 |
| FR-037-008 | T037-003, T037-004, T037-010, T037-032, T037-033 | Every negative reviews now; authoritative final-absence automation later | Partial: final-absence contract gate |
| FR-037-009 | T037-003, T037-004, T037-007, T037-008, T037-010 | No direct original retry from unknown, even privileged; outcome-bearing resolution required | D1 |
| FR-037-010 | T037-005, T037-006, T037-026, T037-032, T037-033 | No defensive action now; pinned qualified positive path later | Partial: qualified path open |
| FR-037-011 | T037-032, T037-033 | Real provider applied/absent/late/repeat/finality matrix | US04 open |
| FR-037-012 | T037-032, T037-033 | Authoritative provider evidence, adapter release and approver | US04 open |
| FR-037-013 | T037-032, T037-033 | Publication/use current authority, integrity and scoped identity | US04 open |
| FR-037-014 | T037-005, T037-006, T037-008, T037-026, T037-032, T037-033 | No exception now; qualified-reference rejection and revocation later | Partial: runtime qualification open |
| FR-037-015 | T037-002, T037-014, T037-018, T037-019, T037-020, T037-021, T037-022, T037-023, T037-024, T037-025, T037-030 | Final-result contract, pending-as-unknown, human provenance | D1 contract; actual provider truth not proven |
| FR-037-016 | T037-003, T037-004, T037-007, T037-008, T037-032, T037-033 | Unknown compensation reviews now; qualified safe-repeat later | Partial: qualified repeat open |
| FR-037-017 | T037-018, T037-019, T037-020, T037-021, T037-022, T037-023, T037-024, T037-026, T037-032, T037-033 | Language/plugin gives no exception now; qualified identity parity later | Partial: qualification open |
| FR-037-018 | T037-014, T037-027, T037-032, T037-033 | One durable lookup/manual winner; qualified action races later | Partial: qualified action open |
| FR-037-019 | T037-014, T037-017, T037-025, T037-030, T037-031 | Existing views, failed/skipped distinctions and human provenance | D1 |
| FR-037-020 | T037-030, T037-031 | Evidence location, retention controls or their absence and operator follow-up | D1 docs |
| FR-037-021 | T037-002, T037-003, T037-004, T037-026, T037-028 | Every leased Activity external-capable; internal controls and child lifecycle unchanged | D1 |
| NFR-037-001 | T037-009, T037-014, T037-027, T037-032, T037-033 | Atomic state/audit/outbox; qualification-use transaction later | Partial: qualification open |
| NFR-037-002 | T037-008, T037-014, T037-026, T037-028, T037-032, T037-033 | Current manual authority, isolation and secret-free audit; qualification later | Partial: qualification open |
| NFR-037-003 | T037-008, T037-025, T037-029, T037-030 | Typed code/refusal, bounded metrics/traces, authorized queries | D1 |
| NFR-037-004 | T037-005, T037-010, T037-013, T037-014, T037-027, T037-032, T037-033 | Replay/identity/restart; pinned qualification replay later | Partial: qualification open |
| NFR-037-005 | T037-009, T037-017, T037-029 | Recovery retained under quota, no busy waiting/lease holding | D1 |
| NFR-037-006 | T037-001, T037-002, T037-030, T037-031 | Fresh coordinated installation, no live-version reinterpretation/unsafe rollback | D1 |
| SC-037-001 | T037-011, T037-012, T037-013, T037-014 | Full stop/deadline and restart matrix | D1 |
| SC-037-002 | T037-015, T037-016, T037-017 | No active/runnable/waiting terminal compensation | D1 |
| SC-037-003 | T037-003, T037-004, T037-005, T037-006, T037-009, T037-010, T037-026, T037-028 | Every tested unqualified unknown holds | D1 |
| SC-037-004 | T037-032, T037-033 | Provider-specific qualification matrix, later implementation required | US04 open |
| SC-037-005 | T037-014, T037-027, T037-032, T037-033 | Current races/isolation; qualified-exception races later | Partial: qualification open |
| SC-037-006 | T037-018, T037-019, T037-020, T037-021, T037-022, T037-023, T037-024, T037-025, T037-030, T037-031, T037-034 | Six SDKs plus operator demo distinguish outcomes | D1 |
| SC-037-007 | T037-031, T037-032, T037-033 | All requirements have acceptance evidence before full completion | Open until US04 also implemented/tested |

## Open gates and limitations

- The owner approved delivery-1 implementation (T037-001..031 and T037-034) on 2026-11-01.
  Delivery-1 implementation and local validation are complete; formal Constitution/ADR adoption through PR remains pending.
- Delivery 1 can satisfy conservative defaults without a provider registry. It cannot close
  positive qualification, actual-use revocation, provider-finality evidence or SC-037-004.
- A contractually final CompleteStep is still a worker assertion. Unit tests and simulated
  conformance cannot prove the provider or worker truth. No financial readiness is claimed.
- Arbitrary SDK-handler purity is deliberately not certified. Engine controls retain their own
  semantics; external-capable activities follow the conservative default.
- Existing feature-014 activation gaps and unrelated reset work remain open. This plan does not
  claim their completion or imply approval to change their scope.
- Real runtime, SDK and browser checks have run; exact passing and failed checkpoints are in
  [validation-report.md](validation-report.md). Final composed gates remain open until recorded.

## Review evidence

Two read-only research agents inspected recovery paths and qualification authority separately.
Their proposals informed the design; that is research, not independent acceptance. A separate
fresh-context EGM Design Critic reviews the assembled plan against the installed Heilmeier standard.
The completed R1/R2 findings and resolutions are retained in [planning review](evidence/planning-review.md).
R2 found no remaining blocker for human design approval at that planning checkpoint.
Subsequent implementation evidence is recorded separately in the validation report.
A selected design-critique stage is not all five EGM review phases or a full drift-report rerun.


### Delivery-1 correction checkpoint (2026-11-01)

The minimatch audit blocker is closed by a reviewed local import bridge to pinned upstream
11.0.1, consumer-scoped resolution, actual-tool regressions and verified Node 22 filtered Docker
installation. No matcher fork or advisory exclusion. Two independent runtime review findings
(scoped Retry after stop/ancestor drain and successful-output refusal losing recovery) have
RED/GREEN evidence and fixes; see [correction summary](evidence/d1-review-correction-summary.md).
The 1/100/1,000 review measurement passed with query plans and host/budget context. Full composed
workspace discovery completed with retained REDs and separately verified corrections. Compose and
browser gates passed. The broader SDK CI corrections, executable examples, native-sandbox checks
and final static gates passed through the recorded correction retests. US04 remains open.


The owner explicitly retained feature-009 final secret-output failure for this delivery;
ordinary `SECRET_IN_OUTPUT` is a documented compatibility limit, not an unresolved approval.
Storage-refusal preservation must not be generalized to every rejected report.


T037-031 closed with the final composed ledger in [validation-report.md](validation-report.md):
original workspace/SDK REDs are retained, all affected cases were corrected and retested, and
unchanged cases retain their broad-run evidence. This is not a claim that the original broad
commands exited zero. All six SDKs are represented. The owner-confirmed 009 boundary, fresh-install
restriction, retention gaps and absence of real-provider qualification remain explicit.
