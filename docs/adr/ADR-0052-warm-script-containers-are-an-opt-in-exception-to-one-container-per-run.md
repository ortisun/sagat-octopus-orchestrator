# ADR-0052: Warm script containers are an opt-in, per-artifact exception to one container per run

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26)
**Refines / supersedes:** supersedes ADR-0013 **in part** (coordinator decision C22): it relaxes ADR-0013's rule
"a container per build and per run", and its rejection of a long-lived container, for opted-in artifacts only; for
every artifact without a policy that rule remains in force. Preserves ADR-0006, ADR-0021, ADR-0024 and every other
boundary of ADR-0013 (signing, digests, supervisor channel, no network, runner without database or signing key).
**Enforced by:** planned tests of feature 019 — the warm residue suite (`/tmp` file, `/dev/shm` write, SysV shared
memory, semaphore and message queue, POSIX message queue, global, child and detached process, signal or `ptrace` of
the resident process, environment, scratch fill, secret-digest eviction) in `integration_inline_code_sandbox`, the eviction tests on secret rotation/revocation, the
test-run exclusion test, and the quota tests proving the default of zero. Until they exist: review only.

## Context

ADR-0013 starts a container for every script run and removes it after: nothing a run leaves (a file, a process, a
poisoned module) can reach the next one. It rejected "one long-lived container per tenant" for exactly that
residue, at a cost it estimated at about a second. The feature-005 report measured, on an Apple M3 Max with
Docker/runc (`tests/performance/reports/inline-code-sandbox.md`), a whole cold run (create, start, run, remove) of
443 ms p50 for TypeScript and 664 ms for Python, against a start-to-first-instruction of 104 ms and 327 ms. Windmill
offers dedicated workers that keep a process alive between jobs of one script
(<https://www.windmill.dev/docs/core_concepts/dedicated_workers>) and does not document isolation between runs.
Feature 019 is asked to offer warm script workers and to state what they give up.

Facts that bound the design:

- Residue inside one container can carry one run's data (personal data of customer A) to a later run of the same
  code for customer B, and a secret value handed to one run to a later run whose step did not declare it.
- The container's network, mounts, capabilities, seccomp profile, user and cgroup do not change with reuse.
- The runner holds no database or signing key; it can only learn of secret rotation or revocation through the
  platform. A run that never asks for a secret produces no refusal, so a refusal is not a usable trigger.
- The current profile (`crates/infra-sandbox/src/docker.rs`) gives every container a writable `/tmp`, Docker's default
  writable `/dev/shm`, a private IPC namespace that lives as long as the container (System V shared memory,
  semaphores and message queues; POSIX message queues under `/dev/mqueue`), `init: true` and user `65534` for every
  process. In a reused container all of these outlive a process and are invisible to a process count, and a resident
  process of the scripts' own uid can be signalled or traced by a script. These channels can be enumerated, wiped and
  verified; only state the runner cannot enumerate (page cache contents, kernel or runtime vulnerabilities, G98 of
  feature 005) remains a stated residual risk.

## Decision

1. Warm containers MUST be opt-in per (tenant, namespace, artifact digest, declared secret set) by a holder of
   `script:warm:manage`, with an explicit acknowledgement of a versioned residue statement, audited. Warm container
   quotas MUST default to zero and MUST be enforced exactly: a warm container MAY serve a run only while its runner
   holds a live warm-slot lease that the control plane granted, over the runner REST surface, after counting live
   slots per namespace and tenant (the runner still never touches the database).
2. A warm container MUST use the same hardened profile as a cold one and MUST serve one invocation at a time, each
   in a **fresh interpreter process** with its own lease, input channel, limits and deadline. In-process reuse of
   an interpreter or module state between runs MUST NOT be offered by this decision (O1042).
3. **The warm profile and the reset between invocations are fixed and checked.**
   - Each invocation MUST be started as a fresh process by an engine `exec` into the container, issued by the
     runner, which already holds the engine; its input arrives on that exec's standard input and its result leaves
     on its standard output, as for a cold run.
   - The resident idle process MUST run under a uid different from the scripts' (scripts keep `65534`), so a script
     cannot signal it; the warm seccomp profile MUST additionally deny `ptrace`, `process_vm_readv` and
     `process_vm_writev`; keyring calls stay denied as in the engine's default profile.
   - `/dev/shm` MUST be mounted read-only with size 0 in warm containers.
   - After each invocation the runner MUST kill every process of the scripts' uid, remove everything under `/tmp`,
     and then verify: `/tmp` is empty; `/proc/sysvipc/shm`, `/proc/sysvipc/sem` and `/proc/sysvipc/msg` list no
     object; `/dev/mqueue` is empty; the process list equals the baseline (the same resident process, executable and
     start time). Any failure MUST evict the container; the run that left the residue is not failed for it.
4. A warm container MUST never serve another tenant, namespace, artifact digest or declared secret set, and MUST be
   evicted on a limit breach, its use/idle/lifetime bound, a policy change, a trust-key change, or any rotation,
   revocation or loss of usability of a secret value it was handed. The trigger MUST be explicit: every warm-slot
   renewal and every warm-eligible lease carry the version digest of the declared secret set, and the runner MUST
   evict a container whose digest differs before starting a process.
5. Output, log and checkpoint scanning of every run in a warm container MUST cover every secret value the container
   has been handed. The runner MUST report the warm slot id with each execution, and the control plane MUST check the
   slot is live and matches tenant, namespace, artifact digest and secret set, and record cold or warm per attempt.
   Test runs MUST NOT use warm containers.
6. Lease, retry, timeout, cancellation and result semantics MUST be identical to a cold run; with no warm container
   available the runner MUST run cold.

## Considered options

1. **Container reuse with a fresh process per run, opt-in and acknowledged (chosen).** Saves container creation and
   removal, keeps process-level isolation of memory and modules, states the remaining filesystem/kernel-level risk
   and makes a person accept it. The saving must be measured; it is not claimed.
2. **Process and module reuse (Windmill-style dedicated workers).** Largest latency gain, but module globals,
   caches and timers survive between runs of different customers; rejected for this feature, kept as O1042.
3. **No warm containers.** Keeps ADR-0013 intact; rejected because the brief asks for the capability and a
   bounded, opt-in variant can be specified and tested.
4. **Pre-created but single-use containers.** Removes the creation cost from the critical path without any reuse;
   viable and safer, but it still pays removal per run and needs the same pool machinery; it remains a possible
   refinement if measurement shows reuse buys little.

## Consequences

- Opted-in artifacts may start faster; by how much is a measurement of feature 019 on a stated machine.
- A namespace that opts in accepts that runs of that artifact may observe what earlier runs left at levels the
  runner cannot enumerate (kernel and runtime state); the audit trail shows who accepted it and when.
- Idle warm containers consume runner capacity; quotas bound it per namespace and tenant.
- Revocation hygiene depends on the secret-version digest carried by slot renewals and leases; until the eviction
  tests pass the capability is not advertised. A second seccomp profile and a resident process under another uid are
  new maintenance (G1059 of feature 019).
- ADR-0013 receives a "superseded in part by ADR-0052" pointer when this ADR is accepted (task T019-001), not before.
- Rollback: set the policy disabled or the quota to zero; the runner drains warm containers and returns to cold
  runs with no change to workflows or artifacts.

## Evidence

None yet. Baseline measurements: `tests/performance/reports/inline-code-sandbox.md`. Design:
[feature 019 plan](../../specs/019-worker-deployments-dispatch/plan.md), D1065–D1066 and G1046 in its
[research](../../specs/019-worker-deployments-dispatch/research.md).
