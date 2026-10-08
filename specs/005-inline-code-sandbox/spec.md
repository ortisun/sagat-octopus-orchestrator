# Feature Specification: Script registry, code editor and inline execution

**Feature Branch:** `005-inline-code-sandbox`  
**Created:** 2026-10-07  
**Status:** Draft — Ready for review  
**Input:** Durable workflow platform, Sagas, SDKs, inline scripts, messaging, webhooks, observability and Studio.

## Vision and value

Allow writing and publishing code directly on the platform without manual application build/deploy.

**Priority:** M3. **Dependencies:** 001, 002, 009.

## User Scenarios & Testing (mandatory)

### User Story 1 — Edit and test a script in the browser (P1)

**As** a user of this feature, **I want** to edit and test a script in the browser, **so that** it allows writing and publishing code directly on the platform without manual application build/deploy.

**Why P1:** independent, verifiable value requirement; supports the M3 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US01-AC01**

- **Given** an authorized author; **When** they write TS or Python code in Monaco and run a sandbox test; **Then** they receive filtered logs, result and inferred schema.

### User Story 2 — Publish an immutable artifact (P1)

**As** a user of this feature, **I want** to publish an immutable artifact, **so that** it allows writing and publishing code directly on the platform without manual application build/deploy.

**Why P1:** independent, verifiable value requirement; supports the M3 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US02-AC01**

- **Given** an approved script with a lockfile; **When** the author publishes a version; **Then** the artifact digest is signed/immutable and new Runs pin the version.

### User Story 3 — Run inline C# code (P1)

**As** a user of this feature, **I want** to run inline C# code, **so that** it allows writing and publishing code directly on the platform without manual application build/deploy.

**Why P1:** independent, verifiable value requirement; supports the M3 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US03-AC01**

- **Given** a C# script with allowed NuGet references; **When** the author publishes; **Then** Roslyn compiles in a build worker and an isolated .NET host executes without a manual app deploy.

### User Story 4 — Block a malicious script (P1)

**As** a user of this feature, **I want** to block a malicious script, **so that** it allows writing and publishing code directly on the platform without manual application build/deploy.

**Why P1:** independent, verifiable value requirement; supports the M3 delivery.

**Independent test:** demonstrate the acceptance scenario with API/Studio/SDK, proving persisted transitions and events.

**Acceptance scenario US04-AC01**

- **Given** a script tries to access cloud metadata, the host, or exceed CPU; **When** the worker executes; **Then** egress and resources are denied, the failure result is audited.

## Functional Requirements

- **FR-005-001**: The system MUST allow immutable ScriptDraft, ScriptVersion, BuildJob and ArtifactDigest per tenant/namespace.
- **FR-005-002**: The system MUST implement isolated per-language builders and pinning of toolchain/dep lockfile, SBOM and supply chain policy.
- **FR-005-003**: The system MUST initially support TS/JS and Python; extend to C#/.NET, Go and Java with dedicated build runners.
- **FR-005-004**: The system MUST execute code in the worker sandbox with cgroups/namespaces/seccomp/containers and a network policy.
- **FR-005-005**: The system MUST provide TestRun without publishing it to production, with its own quota and a prohibition on prod secrets.
- **FR-005-006**: The system MUST generate an input schema / typed parameter form and validate output against JSON Schema.
- **FR-005-007**: The system MUST store redacted stdout, stderr, exit status, deps, metrics and trace with configurable retention.
- **FR-005-008**: The system MUST manage build caches by source hash+toolchain+deps+policy, invalidating when the policy changes.

## Edge Cases

- dependency changed remotely.
- build reproduced with a different lock.
- compilation timeout.
- script tries to exfiltrate a secret.
- large cold start.
- publishing during execution.
- syntax error and unknown dependency.

## Mandatory cross-cutting requirements

- **NFR-005-SEC:** tenant isolation, RBAC, least privilege, secret never in logs or in the journal.
- **NFR-005-DUR:** after commit, loss of a worker/broker cannot lose a transition; use lease/fencing where applicable.
- **NFR-005-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-005-COMP:** versioned protocol and schemas; do not break runs in progress.
- **NFR-005-PERF:** Cold/warm start metrics per language; caches only if policy/digest are identical; builds separate from execution.

## Success Criteria (measurable)

- **SC-005-001:** TS/Python/C# script goes from the editor to execution; escape tests and CPU/memory/network limits verified.
- **SC-005-002:** 100% of scenarios US01..US04 pass in a reproducible automated suite.
- **SC-005-003:** no violation of the Constitution's gates and no silent loss of persisted state.
- **SC-005-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.

## Key Entities

Check `docs/architecture/overview.md`, `docs/architecture/execution-semantics.md` and `specs/001-platform-foundation/data-model.md` for the persisted entity; feature abstractions are described in the local `data-model.md`.

## Assumptions

- Workflow IRs and versions are immutable after publication.
- External integrations are at-least-once, subject to idempotency or reconciliation.
- V1 single-region HA; multi-region active-active requirements are out of scope.

## Out of Scope

- Reimplementing Temporal/Restate; implementing the entire connector market; automatic migration of arbitrary code; exactly-once physical execution of external side effects.

## Open Design Questions

- Define capacity/quota values per tier, after benchmark and security testing.
- Confirm the payload classification/retention scheme per customer and regulator.
- Adjust SDK ergonomics based on interviews and proof-of-concept.

## Traceability

Requirements `FR-005-###` and criteria `SC-005-###` must be mapped to tasks and tests in `tasks.md` before the start of each implementation.
