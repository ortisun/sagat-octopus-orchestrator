# ADR-0106: Each SDK runs workflow code under a named determinism runtime, with the same deterministic primitives in six languages

**Status:** Proposed
**Decided by:** agent (Claude Code, design 044), after the determinism study of 2026-11-24, under the owner's delegation (OD-2026-11-20-6, OD-2026-11-24-4); the study was required by OD-2026-11-24-3; no person has reviewed it
**Refines / supersedes:** none; adds to ADR-0007 and ADR-0015 (one conformance suite for the SDKs)
**Required before acceptance:** the replay measurements of design 044 Task 044-25
**Enforced by:** the conformance scenarios `code-deterministic-primitives`, `code-nondeterminism` and `code-activation-deadline` in all six SDKs; each runtime's tests of design 044 Tasks 044-10 and 044-15 to 044-20 (for example `the context global is not a host object`, `registration_refuses_an_unset_or_random_hash_seed`, `a_module_level_import_of_uuid_is_refused_in_a_process_that_loaded_it`, `TestSagatvetRefusesGoStatementsSelectAndMapRange`, `ConfigureAwaitFalseBeforeAnSdkCallFailsTheWorkflowTask`, `theArtifactsClassFilesAreJava21`, `a_tokio_sleep_in_a_workflow_fails_the_workflow_task`, `four abandoned activations withdraw the workflow types`)

## Context

Each language has its own sources of nondeterminism: Python's string hashes change per process (four orders under four
seeds in the spike), Go's map iteration and `select` are randomized (eight orders in fifty ranges; 59 of 100), Rust's
`HashMap` iterates differently per map (49 orders in fifty maps), C#'s `ConfigureAwait(false)` and `Task.Run` leave a
custom scheduler, Java has no public custom scheduler for virtual threads, and a Node `vm` context with an ordinary
contextified global leaks the host's objects (only `vm.constants.DONT_CONTEXTIFY` gives an in-context global). The
runtime version, the dependencies, the time zone and the locale also change behaviour. The spikes are recorded in
Decisions 044-041 and 044-053 of `specs/044-workflow-as-code/research.md`.

## Decision

1. Each SDK MUST run workflow code under this runtime: TypeScript, a `node:vm` context per run created with
   `DONT_CONTEXTIFY`, no code generation from strings, no host object placed in it (string calls in, a JSON string out),
   code bundled by the SDK with imports of built-ins refused at bundling and the nondeterministic globals replaced or
   removed; Python, a deterministic `asyncio` event loop, workflow modules executed in a private module namespace whose
   `__import__` is the SDK's (shared vetted modules, restricted views of `os`, `time`, `datetime`, `random`, `uuid`,
   `locale` and `pathlib`, refused modules, everything else executed per digest), and a PEP 578 audit hook that acts only
   while the activation's context variable is set; Go, an `iter.Pull` coroutine dispatcher with the
   `sagatvet` analyzer; C#, a single-threaded `TaskScheduler` checked on every SDK call, with a Roslyn analyzer; Java,
   virtual threads under one dispatcher permit, checked on every SDK call, in an artifact for Java 21; Rust, a
   single-threaded executor on threads without a Tokio runtime, with a clippy configuration.
2. Each runtime MUST give workflow code UTC and the root locale. An SDK MUST refuse to register workflow types when its
   precondition fails: TypeScript below Node 22.8 or a process time zone other than UTC; Python below 3.11, a process
   time zone other than UTC, or a `PYTHONHASHSEED` that is unset, `random` or `0`; C# with a local time zone other than
   UTC; Java below 21, or with a default time zone other than UTC or a default locale other than `Locale.ROOT`.
3. The code digest MUST include the runtime's name and full version, with its locale-data version where it has one, and
   the dependency closure (for Python, extension binaries included), and for Python the `hash()` of a fixed string under
   the process's seed. Each SDK MUST declare whether it offers workflow as code, and the conformance suite MUST report
   the scenarios of an SDK that does not as not offered, never as passed.
4. Time MUST be the time of the activation's `workflow_task.started` (the later of the previous start's time and the
   database clock), constant during the activation run; random
   numbers MUST come from xoshiro256** seeded by the run's 32-byte seed (four little-endian 64-bit words; an all-zero seed
   replaced by SplitMix64 from 0), `random()` as `(next() >> 11) × 2^-53`, bounded integers without bias by rejection,
   UUIDs version 4 from the generator; the six SDKs MUST produce the same sequence for the same seed. A reset MUST keep
   its source's seed.
5. An activation run that does not yield within 2 s (configurable from 100 ms to the workflow task timeout) MUST fail the
   workflow task with `ACTIVATION_DEADLINE`; a worker with 4 abandoned activation runs MUST withdraw its workflow types
   and report itself unhealthy.
6. The protocol MUST reserve a field for behaviour flags of the SDKs; none is recorded in 0.3.0.

## Considered options

1. **A runtime per language, one set of primitives** — each closes its language's measured hazards.
2. **WebAssembly for every language** — lost: WebAssembly is not a mandatory runtime (constitution), and Go, Java and C#
   workflow code would lose its ordinary tooling.
3. **Documentation and tests only, no runtime** — lost: Python sets and Go maps break replay silently.
4. **Java on platform threads (Temporal's model) on release 17** — lost: 8,869 ns per handoff against 4,460 ns, and a
   reserved stack per coroutine against about 1,845 bytes; two implementations would double the test matrix.
5. **`PYTHONHASHSEED=0` for every Python worker** — lost: a known seed lets an outside sender build colliding keys; the
   operator's own fixed value keeps determinism without that exposure.

## Consequences

- Good: the strongest available isolation for determinism in each language; a common golden for the primitives; a
  digest that changes when the runtime or a dependency changes.
- Bad: Go, C# and Java rely on analyzers or per-call checks for what the runtime cannot forbid (Gap 044-002, Gap
  044-007); TypeScript workflow code needs Node 22.8, Python workflow code 3.11 and an operator-chosen seed, Java workflow
  code Java 21; the TypeScript SDK gains esbuild as a build-time dependency (Gap 044-012); an abandoned activation run
  cannot be killed in five of the six languages.
- The `vm` context is not a sandbox (Gap 044-008): workflow code is the tenant's, in the tenant's worker.
- Compatibility with runs in flight: none exist before this decision; an SDK upgrade changes the code digest, so it never
  applies to a running run without a move. Operational cost: new language baselines for the workflow packages (Node
  22.8, Python 3.11, Java 21) in the SDKs' continuous integration, an esbuild review, and the operator's Python hash
  seed kept as a deployment secret.

## Evidence

Design 044, Technical Plan 1 and 3.2; Decisions 044-041, 044-051, 044-052, 044-053 and 044-063.
