# ADR-0015 — SDKs meet the server by minor version of the worker protocol, and are held to one conformance suite

Status: **Accepted** | Date: 2026-10-10 | Feature: 006 | Refines ADR-0007 (does not change its decision)

## Context

ADR-0007 decided that the SDKs of five languages share one protocol (gRPC/Protobuf for workers, OpenAPI for
clients) so that no language forks the semantics, and that this needs a permanent compatibility suite. Feature
006 has to say two things ADR-0007 left open.

**How an SDK and a server of different ages meet.** Servers and SDKs are upgraded on different days, by
different teams. The server used to accept exactly `protocol_version = "1"` and refuse everything else, so an
SDK could only be upgraded in step with its server (US04: "a compatible previous minor version; the worker
connects to the updated server; protocol negotiation supports progressive upgrade"). Protocol additions since
1.0 are additive fields (1.1: the step identity of `Heartbeat`; 1.2: `task_kind`, `target_node`,
`ResolveSecrets`).

**What "the five SDKs behave the same" is checked against.** Five SDKs have five unit-test suites, written by
the same authors from the same document; they can all be wrong the same way, or each right about a different
reading. A test that only one SDK runs says nothing about the other four. The TypeScript SDK was written first
and is a tempting reference, but a reference implementation is an opinion, not a contract.

## Decision

**1. Negotiation by minor version, in `RegisterWorker`.** A worker offers the newest minor version of protocol 1
it speaks (`"1"` or `"1.<m>"`; `"1"` is `1.0`). The server answers `accepted_protocol_version`: the worker's own
if it is not newer than the server's, the server's otherwise. Another major version, or anything that is not a
version, is refused with `INVALID_ARGUMENT` and the stable reason `UNSUPPORTED_PROTOCOL` (the SDK raises the
canonical error of that name and does not retry). The SDK must use only what the accepted version has: against
`1` it sends `Heartbeat` without run, step and attempt and does not call `ResolveSecrets`; against `1.1` it does
not rely on `task_kind`. **The server must keep serving the forms of every minor it accepts**; a change that
makes it refuse the form of an older minor is a breaking change and needs a new major.

**2. One conformance suite is the contract** (`tests/contract/sdk-conformance/`). Scenarios say what an SDK
does, not how it is written; each SDK ships a small runner that plays them through its **public API only**;
the harness boots a **real platform** (PostgreSQL, the execution engine, the REST API) **per scenario and per
language** and requires, for every scenario, that

- the builders of the five SDKs write byte-identical workflow documents (canonical JSON), equal to a golden file;
- the normalized observations (ids, instants and trace ids replaced; the SDK's own message words dropped but
  checked to carry no token or marked payload), the worker log, the number of calls each injected fault
  answered, and **the state of every run as the platform holds it** (read over REST, without any SDK) are equal
  across the five and equal to a golden file;
- faults are injected at each SDK's transport hook (a lost answer, a call that never leaves, an HTTP or gRPC
  status), counted per fault, so a fault that was never reached, or an SDK that repeated a call it must not,
  shows in the golden;
- the runners run under a time zone that is not UTC and every instant of a Run must still be UTC with a `Z`.

Golden files are reviewed by a person against what each scenario claims; blessing is a separate, deliberate
act (`SAGAT_SDK_BLESS=1`), never part of a normal run. The golden files are *not* "what TypeScript printed".

## Consequences

- An SDK can be upgraded before or after its server within protocol 1; a server can be upgraded without
  breaking workers of an older minor. The price is a rule the server has to keep (point 1), which the suite
  holds: the `protocol-negotiation` scenario runs a worker at `1` through a task that outlives its lease.
  **That scenario found a defect on the first honest run:** the server refused every heartbeat that lacked an
  attempt (it required `attempt >= 1` for all requests), so a worker of 1.0 could not keep a lease and its task
  was leased again until the run failed with `LEASE_EXPIRED`. No test had sent a heartbeat of 1.0. Fixed in the
  server (`Attempt::parse_heartbeat`; the step's own attempt is taken when the worker says none; the lease token
  and the generation still fence the lease).
- Adding an SDK is writing a runner and passing the suite; the suite does not change. Adding a behaviour is
  adding a scenario and a golden, then the five runners.
- The suite is slow (a platform per scenario and language, scenarios that space their actions in time because a
  worker is leased tasks in the order they became ready) and needs Docker. It is the gate for SDK changes, not
  for server-only changes; the server's own gRPC contract tests (`tests/contract/006-sdk-multilanguage/grpc`)
  stay fast and run everywhere.
- A scenario that can be racy is written so that it is not (`awaitJournal`/`awaitWorkerLog` instead of sleeps,
  a worker started after the run, `control` actions to move time). A golden that flaked would train people to
  re-bless; every scenario has been run repeatedly before its golden was accepted.

## Alternatives considered

1. **Capability flags per feature instead of a minor number.** Finer, but five SDKs would each have to map
   flags to behaviour; a number is one comparison and the proto already says "added in 1.x" at each field.
2. **A version range in a header on every call.** Repeats on every call what one registration can say; the
   server would have to remember nothing but would parse it everywhere.
3. **Per-SDK test suites only, the TypeScript SDK as the reference.** Rejected: see Context. The first thing the
   shared suite did was to find a server defect that the reference SDK's own tests could not show.
4. **A golden recorded from the first SDK that passes.** Rejected for the same reason; the goldens are reviewed
   against the scenario's claim (two of the first ones recorded behaviour that was wrong and were caught on
   review: a wait that never woke, a worker that was handed the retry it should not have taken).

## Review criteria

Open a new ADR for a new major version of the protocol, for dropping the rule that the server keeps the forms
of every accepted minor, or for letting a golden file be produced without review. Do not change the
normalization silently: what it hides (instants, ids, the SDK's own words) is the contract of what may differ
between languages.
