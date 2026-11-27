# ADR-0052: Warm script containers are an opt-in, per-artifact exception to one container per run

**Status:** Accepted (2026-11-24, delegate (OD-2026-11-24-4), after the revision of 2026-11-24)
**Decided by:** agent proposal (2026-10-26); revised and accepted by an agent in the owner's place on 2026-11-24, delegate
(OD-2026-11-24-4). Reasoning: the revision makes option 4 of the draft (a container created ahead and used once) the
default warm mode, because it removes container creation from a run's critical path with no residue between runs, which
is the latency the user story asks for at none of the isolation cost; reuse of one container by several runs stays as a
second, acknowledged mode for the throughput that only it can give. Windmill's dedicated workers keep a process alive
between jobs with no stated isolation; both modes here keep a fresh process per run and state what they give up.
**Refines / supersedes:** the `reused` mode (decision 3) supersedes ADR-0013 **in part** (coordinator decision C22): it
relaxes ADR-0013's rule "a container per build and per run", and its rejection of a long-lived container, for artifacts
whose policy chooses that mode only. The `prepared` mode keeps that rule (each container serves one run) and only moves
the container's creation before the run. For every artifact without a policy ADR-0013 is unchanged. Preserves ADR-0006,
ADR-0021, ADR-0024 and every other boundary of ADR-0013 (signing, digests, supervisor channel, no network, runner without
database or signing key). Composes with ADR-0082: the control plane's warm-slot routes are the fleet requirement
`warm-scripts-v1`.
**Enforced by:** planned tests of feature 019 — the warm residue suite of the `reused` mode (`/tmp` file, `/dev/shm`
write, SysV shared memory, semaphore and message queue, POSIX message queue, global, child and detached process, signal
or `ptrace` of the resident process, environment, scratch fill, secret-digest eviction) and the single-use test of the
`prepared` mode (a container that served a run is never handed another, and it is removed) in
`integration_inline_code_sandbox`; the eviction tests on secret rotation and revocation; the test-run exclusion test;
the quota tests proving the default of zero; the slot tests with two runners at the tenant ceiling. Until they exist:
review only.

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
- The 443 ms of a whole TypeScript run splits into the part before the script's first instruction (creation and start)
  and the part after its end (removal). A container created and started before the run moves the first part off the
  critical path and lets the second happen after the result is reported, without any reuse. What reuse adds on top is
  the saving of one creation and one removal per run in runner capacity (throughput), not in a run's latency.

## Decision

1. Warm containers MUST be opt-in per (tenant, namespace, artifact digest, declared secret set) by a holder of
   `script:warm:manage`, in one of two modes, `prepared` (the default) or `reused`. Warm container quotas MUST default
   to zero and MUST be enforced exactly in both modes: a warm container MAY exist and serve a run only while its runner
   holds a live warm-slot lease that the control plane granted, over the runner REST surface, after counting live
   slots per namespace and tenant (the runner still never touches the database). A container serves only runs whose
   resource limits equal the limits it was created with; any other run of the artifact runs cold.
2. **Prepared mode: one container per run, created ahead.** A prepared container MUST be created and started with the
   hardened profile of ADR-0013, the artifact mounted and nothing else: no secret, input, lease or run identity. It MUST
   serve exactly one invocation, started as a fresh process by an engine `exec` issued by the runner, with its input on
   that exec's standard input and its result on its standard output, and MUST be removed after that invocation ends,
   after the result is reported. It MUST never be handed a second run, so no residue acknowledgement is asked and no
   reset or residue check runs. A prepared container that waits longer than its idle bound, or whose policy, trust keys or
   slot lease change, is removed unused.
3. **Reused mode: one container, a fresh process per run, acknowledged.** A reused container MUST be enabled only with an
   explicit acknowledgement of a versioned residue statement, audited, and MUST serve one invocation at a time, each in a
   **fresh interpreter process** with its own lease, input channel, limits and deadline. In-process reuse of an
   interpreter or module state between runs MUST NOT be offered (owner question O1042, decided no). The warm profile and
   the reset between invocations are fixed and checked:
   - Each invocation MUST be started as a fresh process by an engine `exec` into the container, issued by the
     runner, which already holds the engine; its input arrives on that exec's standard input and its result leaves
     on its standard output, as for a cold run.
   - The resident idle process MUST run under a uid different from the scripts' (scripts keep `65534`), so a script
     cannot signal it; the warm seccomp profile MUST additionally deny `ptrace`, `process_vm_readv` and
     `process_vm_writev`; keyring calls stay denied as in the engine's default profile.
   - `/dev/shm` MUST be mounted read-only with size 0 in reused containers.
   - After each invocation the runner MUST kill every process of the scripts' uid, remove everything under `/tmp`,
     and then verify: `/tmp` is empty; `/proc/sysvipc/shm`, `/proc/sysvipc/sem` and `/proc/sysvipc/msg` list no
     object; `/dev/mqueue` is empty; the process list equals the baseline (the same resident process, executable and
     start time). Any failure MUST evict the container; the run that left the residue is not failed for it.
4. A reused container MUST never serve another tenant, namespace, artifact digest or declared secret set, and MUST be
   evicted on a limit breach, its use, idle or lifetime bound, a policy change, a trust-key change, or any rotation,
   revocation or loss of usability of a secret value it was handed. The trigger MUST be explicit: every warm-slot
   renewal and every warm-eligible lease carry the version digest of the declared secret set, and the runner MUST
   evict a container whose digest differs before starting a process.
5. Output, log and checkpoint scanning of every run in a reused container MUST cover every secret value the container
   has been handed. In both modes the runner MUST report the warm slot id with each execution, and the control plane MUST
   check the slot is live and matches tenant, namespace, artifact digest and secret set, and record per attempt whether
   it ran `cold`, `prepared` or `reused`. Test runs MUST NOT use warm containers.
6. Lease, retry, timeout, cancellation and result semantics MUST be identical to a cold run in both modes; with no warm
   container available the runner MUST run cold.
7. The control plane's warm-slot routes are the fleet requirement `warm-scripts-v1` of the instance registry
   (ADR-0082); until it is recorded a warm policy cannot be enabled.

## Considered options

1. **Two modes, `prepared` by default and `reused` acknowledged (chosen, revised 2026-11-24).** The default keeps
   ADR-0013's isolation exactly and takes creation off the critical path; reuse is there for throughput, with its
   residue stated and accepted by a person. Cost: two pool behaviours in the runner, sharing slot leases, quotas, slot
   checks and the exec path.
2. **Container reuse with a fresh process per run as the only mode (the draft's choice).** Saves creation and removal
   per run, but asks every adopter to accept residue to get a latency gain the prepared mode gives without it.
3. **Process and module reuse (Windmill-style dedicated workers).** Largest latency gain, but module globals,
   caches and timers survive between runs of different customers; rejected (owner question O1042).
4. **No warm containers.** Keeps ADR-0013 intact; rejected because the brief asks for the capability and a
   bounded, opt-in variant can be specified and tested.
5. **Prepared containers as the only mode.** Safest, but it still pays one creation and one removal of runner capacity
   per run; a namespace with a high rate of short scripts would need the reused mode to keep up, so it stays available.

## Consequences

- Opted-in artifacts start faster; by how much, in each mode, is a measurement of feature 019 on a stated machine.
- A namespace that chooses the `reused` mode accepts that runs of that artifact may observe what earlier runs left at
  levels the runner cannot enumerate (kernel and runtime state); the audit trail shows who accepted it and when. The
  `prepared` mode asks for no such acceptance.
- Idle warm containers consume runner capacity; quotas bound it per namespace and tenant.
- Revocation hygiene of the reused mode depends on the secret-version digest carried by slot renewals and leases;
  until the eviction tests pass that mode is not offered. A second seccomp profile and a resident process under another
  uid are new maintenance (G1059 of feature 019).
- ADR-0013 receives a "superseded in part by ADR-0052 (reused mode)" pointer with this acceptance (task T019-001).
- Rollback: set the policy disabled or the quota to zero; the runner drains warm containers and returns to cold
  runs with no change to workflows or artifacts.

## Evidence

None yet. Baseline measurements: `tests/performance/reports/inline-code-sandbox.md`. Design:
[feature 019 plan](../../specs/019-worker-deployments-dispatch/plan.md), D1065–D1066, D019-009 and G1046 in its
[research](../../specs/019-worker-deployments-dispatch/research.md).
