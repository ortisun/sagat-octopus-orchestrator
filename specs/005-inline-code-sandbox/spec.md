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

### User Story 5 — Run inline Go code (P1)

**As** an author, **I want** to compile and run an inline Go function, **so that** I can use a native
binary without creating and deploying a separate service.

**Why P1:** Go is a promised dedicated runner and native artifacts require explicit architecture and
startup capability gates to avoid scheduling incompatible code.

**Independent test:** compile a fixed-module Go source in a real isolated builder, validate and sign
its native artifact, then execute it through the real sealed-memory loader and mediated host calls.

**Acceptance scenario US05-AC01**

- **Given** a runner configured and natively proven for the artifact's signed Linux target; **When**
  the author publishes and runs a Go script; **Then** the runner executes the static binary from a
  sealed memory file under the unchanged sandbox profile and returns its JSON result.
- **Given** an incompatible target, a failed startup probe, a non-empty lockfile or a malformed gzip
  or ELF; **When** build or execution is requested; **Then** the runner fails closed and does not
  advertise or execute Go on that runner.

### User Story 6 — Run inline Java code (P1)

**As** an author, **I want** to compile and run an inline Java 17 class, **so that** I can use Java
without creating and deploying a separate service.

**Why P1:** Java is a promised dedicated runner and needs a narrow class-file and JSON boundary rather
than an author-controlled project or build tool.

**Independent test:** compile `Script.java` with the trusted `javac` host in a real isolated builder,
validate and sign its classes, then execute it with the real parent-first Java runtime and mediated
host calls.

**Acceptance scenario US06-AC01**

- **Given** a Java class implementing `InlineScript`; **When** the author publishes and runs it;
  **Then** the Java 17 verifier loads only `Script.class` and its nested classes and exchanges maps,
  lists and JSON primitives through the trusted API.
- **Given** a JAR, unrelated class, preview/wrong-version class, non-empty lockfile or author build
  options; **When** build or execution is requested; **Then** the runner refuses it.

## Functional Requirements

- **FR-005-001**: The system MUST allow immutable ScriptDraft, ScriptVersion, BuildJob and ArtifactDigest per tenant/namespace.
- **FR-005-002**: The system MUST implement isolated per-language builders and pinning of toolchain/dep lockfile, SBOM and supply chain policy.
- **FR-005-003**: The system MUST support TS/JS, Python, C#/.NET, Go and Java with dedicated build runners and additive artifact formats that preserve stored earlier versions.
- **FR-005-004**: The system MUST execute code in the worker sandbox with cgroups/namespaces/seccomp/containers and a network policy.
- **FR-005-005**: The system MUST provide TestRun without publishing it to production, with its own quota and a prohibition on prod secrets.
- **FR-005-006**: The system MUST generate an input schema / typed parameter form and validate output against JSON Schema.
- **FR-005-007**: The system MUST store redacted stdout, stderr, exit status, deps, metrics and trace with configurable retention.
- **FR-005-008**: The system MUST manage build caches by source hash+toolchain+deps+policy, invalidating when the policy changes.
- **FR-005-009**: A Go artifact MUST bind one natively proven Linux target; a runner MUST reject a target mismatch and advertise Go only after the production-path startup probe passes.
- **FR-005-010**: Go execution MUST preserve the default seccomp profile and `noexec` scratch mount by executing the signed bytes from a four-seal memory file, with input on sealed fd 3 and host replies on standard input.
- **FR-005-011**: Java builds MUST use fixed `javac --release 17 -proc:none` arguments and execution MUST load only validated `Script.class`/`Script$*.class` bytes through the trusted parent-first Java 17 host.
- **FR-005-012**: Go and Java MUST reject non-empty lockfiles and dependency/package requests; their compiler, ABI, target, encoding/compression or JSON library, and exact image identities MUST participate in build provenance and SBOM evidence.

## Edge Cases

- dependency changed remotely.
- build reproduced with a different lock.
- compilation timeout.
- script tries to exfiltrate a secret.
- large cold start.
- publishing during execution.
- syntax error and unknown dependency.
- architecture mismatch and a startup capability probe that fails.
- malformed, truncated, concatenated or over-expanding gzip.
- ELF interpreter, executable stack, writable-executable segment or out-of-bounds headers.
- Java preview/wrong-version bytecode, class-name mismatch, unrelated classes and JARs.

## Mandatory cross-cutting requirements

- **NFR-005-SEC:** tenant isolation, RBAC, least privilege, secret never in logs or in the journal.
- **NFR-005-DUR:** after commit, loss of a worker/broker cannot lose a transition; use lease/fencing where applicable.
- **NFR-005-OBS:** structured logs, correlated traces, essential metrics and auditability per Run/Step.
- **NFR-005-COMP:** versioned protocol and schemas; do not break runs in progress.
- **NFR-005-PERF:** Cold/warm start metrics per language; caches only if policy/digest are identical; builds separate from execution.

## Success Criteria (measurable)

- **SC-005-001:** TS/Python/C#/Go/Java script goes from the editor to execution; escape tests and CPU/memory/network limits verified on every enabled native target.
- **SC-005-002:** 100% of scenarios US01..US04 pass in a reproducible automated suite.
- **SC-005-003:** no violation of the Constitution's gates and no silent loss of persisted state.
- **SC-005-004:** critical errors expose `code`, `trace_id` and remediation without leaking payload/secrets.
- **SC-005-005:** 100% of malformed v3 gzip/ELF/class fixtures fail before execution while the complete v1/v2 regression suite remains unchanged.
- **SC-005-006:** Go is advertised only for the runner's configured native target after its startup compile-and-execute probe; native ARM64 and AMD64 evidence are recorded independently.

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
- Replace language-only worker claims with target-aware claims before allowing mixed-architecture Go pools.

## Traceability

Requirements `FR-005-###` and criteria `SC-005-###` must be mapped to tasks and tests in `tasks.md` before the start of each implementation. Go and Java decisions and rollout gates are recorded in ADR-0024.
