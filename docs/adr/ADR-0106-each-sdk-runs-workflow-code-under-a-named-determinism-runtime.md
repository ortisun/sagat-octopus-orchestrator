# ADR-0106: Each SDK runs workflow code under a named determinism runtime, with the same deterministic primitives in six languages

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), after the determinism study of 2026-11-24, under the owner's delegation (OD-2026-11-20-6, OD-2026-11-24-4); the study was required by OD-2026-11-24-3
**Refines / supersedes:** none; adds to ADR-0007 and ADR-0015 (one conformance suite for the SDKs)
**Enforced by:** the conformance scenarios `code-deterministic-primitives`, `code-nondeterminism` and `code-activation-deadline` in all six SDKs; each SDK's runtime tests of design 044 Tasks 044-10 and 044-15 to 044-19 (for example `registration_refuses_with_hash_randomization`, `TestSagatvetRefusesGoStatementsSelectAndMapRange`, `ConfigureAwaitFalseBeforeAnSdkCallFailsTheWorkflowTask`, `theArtifactRefusesToStartBelowJava21`, `a_tokio_sleep_in_a_workflow_fails_the_workflow_task`)

## Context

Each language has its own sources of nondeterminism: Python's string hashes change per process (four orders under four
seeds in the spike), Go's map iteration and `select` are randomized (eight orders in fifty ranges; 59 of 100), Rust's
`HashMap` iterates differently per map (49 orders in fifty maps), C#'s `ConfigureAwait(false)` and `Task.Run` leave a
custom scheduler, Java has no public custom scheduler for virtual threads, and a Node `vm` context is a determinism
boundary but not a security one. The determinism spikes are recorded in Decision 044-041 of
`specs/044-workflow-as-code/research.md`.

## Decision

1. Each SDK MUST run workflow code under this runtime: TypeScript, a `node:vm` context per run with
   `microtaskMode: 'afterEvaluate'`, code bundled by the SDK and imports of `node:` built-ins refused at bundling, the
   nondeterministic globals replaced or removed; Python, a deterministic `asyncio` event loop, an import finder and a
   PEP 578 audit hook active during activations; Go, an `iter.Pull` coroutine dispatcher with the `sagatvet` analyzer;
   C#, a single-threaded `TaskScheduler` checked on every SDK call, with a Roslyn analyzer; Java, virtual threads under
   one dispatcher permit, checked on every SDK call, in an artifact for release 21; Rust, a single-threaded executor on a
   thread without a Tokio runtime, with a clippy configuration.
2. An SDK MUST refuse to register workflow types when its runtime's precondition fails: Python with string hash
   randomization on (`sys.flags.hash_randomization != 0`), Java below 21.
3. Time MUST be the database time of the workflow task's `workflow_task.started`, constant during an activation; random
   numbers MUST come from xoshiro256** seeded by the run's 32-byte seed (four little-endian 64-bit words; an all-zero seed
   replaced by SplitMix64 from 0), `random()` as `(next() >> 11) × 2^-53`, bounded integers without bias by rejection,
   UUIDs version 4 from the generator; the six SDKs MUST produce the same sequence for the same seed.
4. An activation that does not yield within 2 s (configurable from 100 ms to the workflow task timeout) MUST fail the
   workflow task with `ACTIVATION_DEADLINE`.
5. An SDK MUST record in a completion the behaviour flags it used for the first time in the run, and MUST use a flag on
   replay only if the history recorded it.

## Considered options

1. **A runtime per language, one set of primitives** — each closes its language's actual hazards, measured.
2. **WebAssembly for every language** — lost: WASM is not a mandatory runtime (constitution), and Go, Java and C#
   workflow code would lose its ordinary tooling.
3. **Documentation and tests only, no runtime** — lost: Python sets and Go maps break replay silently.
4. **Java on platform threads (Temporal's model) on release 17** — lost: 8,869 ns per handoff against 4,460 ns, and a
   reserved stack per coroutine against about 1,845 bytes; two implementations would double the test matrix.

## Consequences

- Good: the strongest available isolation in each language, a common golden for the primitives.
- Bad: Go, C# and Java rely on analyzers or per-call checks for what the runtime cannot forbid (Gap 044-002, Gap
  044-007); Python workers need `PYTHONHASHSEED=0`; the TypeScript SDK gains esbuild as a build-time dependency (Gap
  044-012); Java code workflows need Java 21.
- The `vm` context is not a sandbox (Gap 044-008): workflow code is the tenant's, in the tenant's worker.

## Evidence

Design 044, Technical Plan 1; Decision 044-041.
