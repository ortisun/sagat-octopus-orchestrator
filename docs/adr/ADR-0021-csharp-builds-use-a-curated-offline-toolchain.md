# ADR-0021 — C# builds use a curated offline toolchain and signed binary artifacts

Status: **Accepted; implementation verified** | Date: 2026-10-11 | Feature: 005

Supersedes only ADR-0013's refusal of C# and its blanket refusal of dependencies for C#.
The isolation, signing, tenant, lease and host-call boundaries of ADR-0013 remain in force.

## Context

US03 and T005-018–022/029 require Roslyn compilation and execution with allowed NuGet references.
The existing artifact format carries text files, and the existing dependency policy refuses every
package. Merely enabling the language or compiling against framework references would not meet
that acceptance scenario. Restoring an author's project would also execute a much broader build
surface than compiling C# source.

## Decision

1. A dedicated build container compiles source with Roslyn into a managed assembly. It never runs
   an author's project, MSBuild target, source generator, analyzer or native package asset. An
   execution uses a separate runtime container and the compiled artifact, without compiling again.
2. Both containers retain the existing network-disabled, non-root, read-only, capability-free,
   seccomp and quota controls. No database, signing key or container-engine socket enters either
   container. Assembly loading is a mechanism, never the security boundary.
3. Dependencies come from a closed catalog installed when the trusted runner image is built.
   Every package has an exact identity, version, NuGet lockfile `contentHash`, and independently
   verified SHA-512 of the downloaded archive. The lockfile hash and archive hash are recorded
   separately; they are not assumed equal for a signed package. The image toolchain identity
   participates in the build key. The lockfile must name exactly an approved
   dependency closure. Unknown, changed, native or executable build assets are refused. There is
   no package restore or registry access during an author's build or execution.
4. Binary artifacts use additive bundle version 2, language `csharp`, `encoding: "base64"`,
   entry `script.dll`, and the existing `files` object with base64 values. Only managed DLLs are
   carried. The validator rejects invalid types, encodings, paths, colliding names, absent entries
   and size/count excess before signing. Dependency assemblies travel inside the signed bundle.
   Version 1 TypeScript/Python artifacts retain their existing wire representation.
5. The build protocol carries the lockfile. Its bounded response frame accommodates an artifact
   up to the existing artifact ceiling. Runtime result and log ceilings do not increase. The C#
   runtime alone permits a transport frame of at most `6 * MAX_RESULT_BYTES + 16 KiB` (1,490,944
   bytes), because .NET escapes some Unicode even with its relaxed encoder. Above the ordinary
   262,144-byte frame limit, only a valid terminal result is accepted: oversized host calls,
   unknown messages and malformed JSON are refused. The parsed result still must fit 245,760
   bytes. This admits escaped string/key content without multiplying the host-call budget by six;
   it does not promise to accept every numerically equivalent JSON representation. Build resource
   limits remain bounded independently of any measurement-driven adjustment.
6. Existing leases, fencing, immutable versions, execution records, typed failures, host calls,
   redaction and low-cardinality telemetry serve C# too. No parallel execution-state machine is added.

## Consequences and alternatives

The platform must maintain and rebuild a small curated language image when the SDK, package catalog
or policy changes. That operational cost buys a reproducible offline build surface. Arbitrary NuGet
restore is deliberately not supported. Future catalog additions require dependency review and an
image identity change; a package name on its own is not authorization.

Text-only artifacts cannot represent compiled assemblies. A new bundle version makes that boundary
explicit while keeping existing runs compatible. Raising every protocol limit would broaden the
runtime attack surface. Build output gets an artifact-sized ceiling; C# result envelopes get a
separate bounded allowance with a terminal-result-only guard. No other runtime frame budget grows.

## Verification

Evidence must cover an allowed NuGet reference from publish through a real container execution,
rejection of changed hashes and unsupported packages/assets, malformed binary bundles, signature
and cache identity checks, compilation errors/timeouts, execution limits, denied network/secret
access, redaction, duplicate completion, runner loss/retry and tenant boundaries. Record cold/warm
build and execution measurements, resource settings and failure observations in feature 005's
performance report; do not infer isolation from a local host-only compiler test.

The real-container suite passed 30/30 cases, registry integration 41/41, script E2E 28/28 and fault
tests 12/12. The final framing guard passed 19 sandbox unit tests and its real-container Unicode
case. The exact Studio starter compiled and executed in an isolated container. Independent security
review found no remaining blockers after PE, catalog, SBOM and framing corrections.

The [C# performance report](../../tests/performance/reports/inline-code-sandbox-csharp.md) records
20 real builds and executions in release mode, with image identities, resource policy and local
load. All produced the expected result and left no containers behind. Feature 005's quickstart and
tasks link the reproducible commands. These measurements establish a local baseline, not a
production SLO; full repository CI remains a merge gate.

## Sources

- [Microsoft: AssemblyLoadContext provides no security features](https://learn.microsoft.com/en-us/dotnet/api/system.runtime.loader.assemblyloadcontext).
- [NuGet: package content resource and SHA-512 hashes](https://learn.microsoft.com/en-us/nuget/api/package-base-address-resource).
- ADR-0006, ADR-0013 and `specs/005-inline-code-sandbox/spec.md`.
