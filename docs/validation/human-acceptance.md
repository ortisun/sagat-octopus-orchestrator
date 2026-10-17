# Human acceptance protocol

This protocol collects the human evidence still required by
[SC-008-001](../../specs/008-observability-operations/spec.md) and
[SC-012-001](../../specs/012-developer-experience/spec.md). Automated tests and machine benchmarks
remain supporting evidence; they do not replace these sessions. Copy
[`human-acceptance-results.template.md`](human-acceptance-results.template.md) for every participant
and preserve every completed, aborted and invalid session.

No human session has been run merely because this protocol exists. Do not update either criterion to
done until completed records have been reviewed. The owner set the threshold of SC-008-001 to
`N = 10` on 2026-10-16: a session passes with **at most 9** counted interactions. Record the raw
interaction count as well as the verdict.

## Rules shared by both evaluations

- Use a participant identifier made for the study. Do not record a name, email address or account
  subject.
- Record the repository commit, browser and tool versions, screen size and the exact documents used.
  Do not mix results from different commits into one session record.
- Never paste a bearer token, cookie, secret, payload, complete environment dump or unredacted terminal
  history into the record. A run ID is evidence, not a credential, and may be recorded. Record only a
  fingerprint of a trace ID, such as the first eight characters or its SHA-256 digest.
- The observer may read the participant handout verbatim and resolve an immediate safety issue. Any
  hint about where to click, which command to run, or how to interpret an error is assistance and must
  be recorded. An assisted session remains useful research but is not unaided acceptance evidence.
- Do not repair the environment while a measured session is running. Abort, mark the session `NOT RUN
  — PRECONDITION FAILED` or `INVALID — INFRASTRUCTURE`, repair it, reset the environment and begin a new
  record.
- Record observed facts and the participant's words. Do not infer a successful diagnosis from the page
  they happened to open, and do not turn a partial run into a pass.

## Evaluation A: diagnose a failed run (SC-008-001)

### Participant profile

Recruit an operator who has not seen the prepared run or its injected failure. They may know workflow
operations generally. Record whether they have used this Studio before; do not describe them as a new
operator unless that is true.

### Facilitator setup and preflight

The facilitator prepares this environment before the participant arrives:

1. Start the local platform and observability services using the runnable prerequisites and Compose
   command in the [feature 008 validation quickstart](../../specs/008-observability-operations/quickstart.md#prerequisites).
   The [observability runbook](../runbooks/observability.md) explains the timeline, trace projection and
   operational data being evaluated.
2. Create a terminal failed or compensated run with more than one meaningful attempt. The runnable
   [payment saga failure quickstart](../quickstarts/saga.md) is the reference setup. Keep the chosen
   mock and its expected diagnosis from the participant.
3. Save the run ID. Confirm independently that `sagat workflow inspect <run-id>` contains the complete
   journal, the failed step and attempt, and the stable failure code. Confirm that the Studio timeline
   has a trace ID and attempt span for that same run and is not marked `BEHIND`, `CATCHING_UP`,
   `NO_TRACE` or `UNTRACKED`.
4. Sign the participant into the Studio with an Operator account before measurement. Close unrelated
   tabs and place the browser on the **Runs** list with default filters. The prepared run must be
   visible in that tenant. Give the participant the run ID on paper or in the task handout; do not open
   its detail page for them.
5. Verify that no token, injected failure description, mock file or expected answer is visible. If any
   preflight item fails, do not start interaction counting.

Use one prepared run per participant, or reset the same immutable run view. Record the setup commands
and expected diagnosis in the observer-only result, never in the participant handout.

### Participant-only task instructions

Give the participant only the text in this block:

> A workflow run needs investigation. Its run ID is **`<RUN_ID>`**. Starting from the Runs page,
> determine:
>
> 1. which step and attempt failed;
> 2. the failure code and the cause shown by the journal;
> 3. whether the trace entry you inspect belongs to that same run, step and attempt; and
> 4. the next action you would recommend, without executing that action.
>
> Tell the observer when you have finished and give those four parts of your diagnosis. You may use
> the product UI and its links. Do not ask the observer for navigation or interpretation help. Do not
> reveal or copy payloads, credentials or tokens.

The task begins after the participant finishes reading this text and says they are ready. It ends when
they say they are finished and give their diagnosis. The observer must not show the setup section or
the expected answer.

### Interaction counting

Count participant-initiated actions from the start signal until the final diagnosis:

- Count one interaction for each click, tap or keyboard activation that navigates, changes a selected
  run or tab, applies a filter, expands or collapses details, opens a trace, changes page, or uses
  browser Back/Forward.
- Count a submitted search or filter as one interaction regardless of the number of characters typed.
  Selecting a suggestion and then submitting it are two interactions if both cause distinct UI
  changes.
- Count a command the participant runs in a terminal as one interaction if they leave the UI to use
  it. Count each subsequent command separately.
- Do not count scrolling, pointer movement, hovering, focusing a field, selecting text, copying an ID,
  ordinary typing before one submission, reading, or stating the diagnosis.
- Do not count the observer opening the start page, signing in, or giving the run ID during preflight.
  Record accidental reloads and ineffective clicks in the event log; count them when they meet the
  rules above.

If an ambiguous action occurs, record it and the counting decision. Do not change the rule after seeing
the total. The recorded total is compared with `N = 10`; the specification says **fewer than** `N`, so 10
interactions do not satisfy it.

### Observer checklist and successful diagnosis

The observer records, without prompting, whether the participant:

- selected the supplied run ID and no other run;
- identified the exact failed step ID and attempt number;
- named the stable failure code and described the journal's cause without inventing payload content;
- inspected journal and trace evidence and explicitly correlated the same run, step and attempt;
- recognized any trace projection warning rather than treating an incomplete trace as journal truth;
- recommended an action consistent with the visible state, without executing a mutation; and
- exposed no token, secret or payload while explaining the result.

A **successful failure diagnosis** requires all of the second through fourth items, use of the supplied
run, and no materially incorrect claim. A recommendation is recorded for usability research but does
not rescue an incorrect diagnosis. Keep the diagnosis outcome separate from the SC threshold:

- `diagnosis: successful|unsuccessful`
- `interactions: <raw count>`
- `SC-008-001 threshold: N = 10 (pass: at most 9 interactions)`

## Evaluation B: first TypeScript and C# runs (SC-012-001)

### Participant profile

Each participant must be a newcomer to Sagat: no previous Sagat quickstart, CLI or SDK use and no
contribution to this repository. They need ordinary terminal familiarity and enough TypeScript and C#
experience to recognize their toolchains. The same participant completes both languages; two people
each completing one language do not demonstrate the criterion as written.

### Facilitator setup and preflight

NFR-012-PERF sets a target of **less than 15 minutes with dependencies installed**. Complete this
preflight before starting the timer:

- provide a clean checkout at the commit under evaluation, with no `infra/docker/.env`, generated
  `hello-world` directory or running Sagat Compose project;
- verify Docker with Compose, the Rust toolchain pinned by the repository, Node.js 20 or newer and the
  .NET SDK 10 or newer;
- ensure required container images and language/package dependencies are already downloaded or in the
  local cache, matching the scope of the existing
  [machine-time report](../../tests/performance/reports/developer-experience.md); do not count a network
  outage or first-time dependency download as the participant's onboarding result;
- make ports 18080, 18090 and 19000 available and verify the container engine can start containers;
- do not install `sagat`, create `.env`, start the platform, seed it, create a token, publish the
  workflow, or prepare either example for the participant; those are product onboarding steps; and
- disable screen/terminal capture unless its redaction has been tested. Keep any observer notes away
  from the participant's terminal.

If a prerequisite is missing, a port is occupied, a registry/package source is required unexpectedly,
or the checkout already contains completed onboarding state, mark `NOT RUN — PRECONDITION FAILED`.
Reset before trying again.

### Participant-only task instructions

Give the participant only this text and the repository root:

> You are evaluating the documentation as a new team member. Starting at
> [`docs/quickstarts/README.md`](../quickstarts/README.md), use only that documentation and pages or
> example READMEs it links to.
>
> 1. Start the local platform and make `sagat doctor` finish with `nothing to fix`.
> 2. Follow the [TypeScript quickstart](../quickstarts/typescript.md) and run its hello-world example
>    successfully.
> 3. Follow the [C# quickstart](../quickstarts/dotnet.md) and run its hello-world example successfully.
> 4. Give the observer the two run IDs and show that each ended `SUCCEEDED`.
>
> You may search within the documentation and follow its links. Do not use source-code search, issue
> trackers, chat, web search or advice from the observer. If the documentation does not let you
> continue, state where you are blocked instead of guessing or asking for a hint. Never read a token
> aloud or paste it into the result record.

Start the timer when the participant says they are ready and first opens the main quickstart. Use one
continuous wall-clock timer: reading, typing, `cargo install`, Compose startup, local builds and both
example runs all count. Do not pause it for ordinary product work. Stop it when the participant shows
two distinct run IDs, one emitted by the TypeScript example and one by the C# example, and both terminal
results say `SUCCEEDED`.

### Observer checklist and outcomes

The observer watches without steering and records:

- start and end timestamps from one monotonic timer;
- every documentation page opened, including linked example READMEs;
- whether `sagat doctor` reached `nothing to fix`;
- the TypeScript and C# run IDs and terminal statuses;
- errors, backtracking, documentation dead ends and any assistance;
- whether the participant used a source not allowed by the handout; and
- any token or sensitive value exposure, described by category only and removed from artifacts.

SC-012-001's human portion is successful only when one eligible participant completes both distinct
runs unaided from the allowed documentation and both end `SUCCEEDED`. Record the automated invalid
workflow suite separately as existing supporting evidence; do not ask the participant to rerun it.
Evaluate the time independently:

- elapsed time `< 15:00`: NFR-012-PERF target met for this session;
- elapsed time `>= 15:00`: target not met for this session; or
- precondition/infrastructure abort: not run, with no performance verdict.

The target is an evaluation result for the recorded environment, not a universal performance promise.
An assisted completion or use of an unlisted source is `INVALID FOR DOCS-ONLY ACCEPTANCE`, even if both
runs succeed.

## Reviewing and retaining results

Review the completed record against this protocol and the commit under test. Preserve failed and
invalid sessions alongside successful ones. A sanitized evidence record may contain run IDs, statuses,
interaction totals, elapsed times and trace fingerprints. Before publishing it, remove participant
identity, tokens, cookies, environment dumps, payloads, secrets, screenshots containing them and raw
terminal recordings. Only after this review may the owner update the relevant specification,
traceability row and criterion status.
