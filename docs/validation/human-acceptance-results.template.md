# Human acceptance results — template

Copy this file for one participant/session. Do not replace `NOT RUN` with a result until the session
has occurred. Follow [`human-acceptance.md`](human-acceptance.md); do not add tokens, cookies, secrets,
payloads, raw environment dumps or unredacted terminal output.

## Record control

| Field | Value |
|---|---|
| Overall status | **NOT RUN** |
| Protocol commit | `<git SHA>` |
| Product commit | `<git SHA>` |
| Session date/time and timezone | `<not run>` |
| Observer | `<study identifier, not personal data>` |
| Participant ID | `<pseudonymous study ID>` |
| Result file visibility | `<private/sanitized-public>` |

Allowed outcome labels are `NOT RUN`, `NOT RUN — PRECONDITION FAILED`, `INVALID — INFRASTRUCTURE`,
`INVALID — ASSISTED`, `INVALID — OUTSIDE SOURCE`, `COMPLETED — UNSUCCESSFUL` and `COMPLETED —
SUCCESSFUL`. A completed result is evidence for review; it does not by itself authorize changing a
success criterion to done.

## Participant qualification

| Question | Answer |
|---|---|
| Evaluation(s) assigned | `<SC-008-001 / SC-012-001 / both>` |
| Current role or relevant experience band | `<record a band, not employer or identity>` |
| Used Sagat Studio before? | `<not recorded>` |
| Used Sagat CLI or SDK before? | `<not recorded>` |
| Contributed to this repository? | `<not recorded>` |
| TypeScript familiarity | `<not recorded>` |
| C# familiarity | `<not recorded>` |
| Eligibility decision and reason | `<not assessed>` |

## Environment

| Field | Value |
|---|---|
| OS / architecture | `<not recorded>` |
| Browser / viewport | `<not recorded>` |
| Docker / Compose | `<not recorded>` |
| Rust | `<not recorded>` |
| Node.js | `<not recorded>` |
| .NET SDK | `<not recorded>` |
| Studio URL | `<not recorded; no token or query credential>` |
| Dependency/image caches prepared | `<not checked>` |
| Unexpected download required | `<not checked>` |

## SC-008-001 — failed-run diagnosis

**Session status:** `NOT RUN`

**Interaction threshold:** `N = PENDING OWNER DECISION`. Until the owner defines `N` in the feature
specification, record the count but set the criterion verdict to `NOT EVALUABLE — N UNSET`.

### Preflight

- [ ] Failed or compensated run prepared without participant involvement.
- [ ] Run has more than one meaningful attempt.
- [ ] Journal independently checked through `sagat workflow inspect <run-id>`.
- [ ] Trace and attempt span projected; no lag/missing-trace state.
- [ ] Operator account signed in; browser starts on the default Runs list.
- [ ] Failure setup and expected answer hidden from participant.
- [ ] No token, payload or secret visible or captured.

If any box is unchecked, record `NOT RUN — PRECONDITION FAILED` and stop.

### Prepared evidence (observer only)

| Field | Expected / prepared | Participant answer |
|---|---|---|
| Run ID | `<not prepared>` | `<not answered>` |
| Terminal run status | `<not prepared>` | `<not answered>` |
| Failed step ID | `<not prepared>` | `<not answered>` |
| Failed attempt number | `<not prepared>` | `<not answered>` |
| Stable failure code | `<not prepared>` | `<not answered>` |
| Journal cause / relevant sequence(s) | `<not prepared>` | `<not answered>` |
| Trace fingerprint only | `<not prepared>` | `<not answered>` |
| Run/step/attempt correlation explanation | `<not prepared>` | `<not answered>` |
| Recommended next action | `<not prepared>` | `<not answered>` |

### Timing and interactions

| Field | Value |
|---|---|
| Start timestamp | `<not started>` |
| End timestamp | `<not ended>` |
| Elapsed wall time | `<not measured>` |
| Raw interaction count | `<not counted>` |
| Assistance given | `<none/not run>` |

| # | Time from start | Participant action | Resulting view/state | Counted? Why? |
|---:|---:|---|---|---|
| — | — | `NOT RUN` | — | — |

### Observer checklist

- [ ] Used the supplied run.
- [ ] Identified the exact failed step and attempt.
- [ ] Named the stable failure code and journal cause.
- [ ] Correlated journal and trace to the same run, step and attempt.
- [ ] Treated any projection warning correctly.
- [ ] Made no materially incorrect diagnosis.
- [ ] Exposed no token, secret or payload.

| Outcome | Value |
|---|---|
| Diagnosis | `NOT RUN` |
| Interaction comparison | `NOT EVALUABLE — N UNSET` |
| SC-008-001 verdict | `NOT RUN` |
| Observer notes | `<none>` |

## SC-012-001 and NFR-012-PERF — newcomer onboarding

**Session status:** `NOT RUN`

### Preflight

- [ ] Participant is new to Sagat and this repository.
- [ ] Clean checkout; no generated workflow or `.env`; no Sagat Compose project running.
- [ ] Docker/Compose, pinned Rust, Node.js 20+ and .NET SDK 10+ work.
- [ ] Required images and package dependencies are already downloaded/cached.
- [ ] Ports 18080, 18090 and 19000 are available.
- [ ] `sagat` is not preinstalled for this checkout.
- [ ] Platform is not started or seeded and no token has been created.
- [ ] Capture is disabled or proven to redact credentials.

If any box is unchecked, record `NOT RUN — PRECONDITION FAILED`; do not start the timer.

### Timer and allowed sources

| Field | Value |
|---|---|
| Start timestamp / monotonic reading | `<not started>` |
| End timestamp / monotonic reading | `<not ended>` |
| Elapsed wall time | `<not measured>` |
| Pauses | `none — continuous timer` |
| Documentation pages opened | `<none>` |
| Outside source used | `<none/not run>` |
| Assistance given | `<none/not run>` |

### Completion evidence

| Check | TypeScript | C# |
|---|---|---|
| Example run started | `<not run>` | `<not run>` |
| Run ID | `<not recorded>` | `<not recorded>` |
| Final terminal status | `<not recorded>` | `<not recorded>` |
| Distinct from other language's run ID | `<not checked>` | `<not checked>` |
| Evidence inspected without copying payload | `<not checked>` | `<not checked>` |

Additional required check:

- `sagat doctor` ended with `nothing to fix`: `<not run>`

### Friction and failure record

| Time | Document/step | What happened | Participant's recovery from documentation | Classification |
|---:|---|---|---|---|
| — | — | `NOT RUN` | — | — |

Use classifications `documentation`, `product`, `participant`, `precondition` or `infrastructure`.
Quote only non-sensitive UI/document text; never paste command output containing a token.

### Outcomes

| Outcome | Value |
|---|---|
| Eligible newcomer completed both languages unaided from allowed docs | `NOT RUN` |
| Both distinct runs ended `SUCCEEDED` | `NOT RUN` |
| SC-012-001 human portion | `NOT RUN` |
| Automated invalid-workflow supporting evidence | `NOT RERUN IN THIS HUMAN SESSION` |
| NFR-012-PERF `< 15:00` target | `NOT RUN` |
| Observer notes | `<none>` |

## Data handling and review

- [ ] Record contains run IDs only for product-resource evidence.
- [ ] Full trace IDs were replaced by fingerprints.
- [ ] No token, cookie, secret, payload or raw environment dump is present.
- [ ] Screenshots/recordings are absent or reviewed and redacted.
- [ ] Assistance, outside sources, aborts and failed attempts are recorded.
- [ ] A reviewer compared this record with the protocol and product commit.
- [ ] No specification, task or traceability status was changed solely by filling this record.

**Reviewer decision:** `NOT REVIEWED`

**Reason:** `<pending completed session and review>`
