# ADR-0079: The lake runner asks the sandbox runner for an attempt, never for a container

**Status:** Proposed (2026-11-07). Accepting it closes the API half of gaps G035-022 and G035-055 of feature 035. It does **not**
close the Kubernetes half (whether a node-local socket can be shared between two containers or two pods: T035-029), and rules 2
and 3 are conditional on that answer (see "Open").
**Decided by:** Agent proposal under the owner's delegation for feature 035 ("Tome as decisões por mim", 2026-11-02), recommended in
research D035-110 (d), drafted and then **rewritten after an independent security review and an architecture review of 2026-11-07**
(the first draft had a Blocker and seven Majors; "Evidence" lists them and what became of each). It adds a server to the process that
holds the container-engine socket and is hard to reverse once the lake runner and the deployments depend on it, so a person accepts it.
**Refines / supersedes:** Refines [ADR-0071](ADR-0071-lake-writes-and-destinations-are-managed-effects-recovered-at-the-destination.md)
(the `lake` container profile; the sandbox runner "of a lake deployment serves only `lake` containers", which rule 1 makes a checked
mode) and [ADR-0013](ADR-0013-scripts-run-in-containers-the-control-plane-never-touches.md) (the process that starts containers is not
the process that holds the data). Supersedes nothing.
**Enforced by:** Planned; **review-only until each exists** (the list says which are not testable and why):
- `scripts/check-architecture.py`, on the **resolved** graph of `cargo metadata` (the script reads direct dependencies today):
  `sagat-database-runner` and `sagat-infra-lake` reach neither `bollard` nor `sagat-infra-sandbox` (rule 2); and a test over
  `infra/docker/*.yaml` and the Helm templates that the lake runner's service mounts no engine socket (rule 2);
- `apps/sandbox-runner` tests against an in-memory engine double: lake mode refuses to start with script or build configuration, and
  script mode has no lake listener (rule 1); an unknown request member, an image, mount, capability, command or limit is refused,
  and a slot outside the range, in use, or a deadline above the ceiling is refused (rule 5); a peer of another user, uid 0 and uid 65534
  are refused, and an unset peer user refuses to start (rule 3); the body, header, connection and concurrency limits, with a closed
  body for every rejection including 404, 405 and 413 (rule 4); the ceiling is held across concurrent starts (rule 6); the deadline
  sweep, `kill` and `wait` after a simulated restart, and the drain (rules 7 to 9);
- one test that runs the real client of `infra-lake` against the real server router over a real Unix socket (rule 10);
- `integration_iceberg_writes` isolation cases (T035-015) run through this API on Compose, **including a symlink swap of the attempt
  directory and the minimum engine version** (rule 5), and on Kubernetes under T035-029;
- **Review-only, and why:** rule 11 (no engine text in a response is a canary test, but "none in the operator's log" is deliberately
  not true, see the rule); the audit trail of rule 12 lives in the lake runner and is checked there.

## Context

The lake runner (`apps/database-runner`, lake profile) holds the credentials of a destination for the life of one attempt, writes the
attempt directory (request line, relay socket, token) into the work volume, and then needs a container: the pinned Python host of
`runners/lake`, with no network but loopback, one mount, and a user of its own per slot (`crates/infra-sandbox/src/lake.rs`,
`container_config`).

Facts, checked against the code on 2026-11-07:

- `LakeEngine` (`crates/infra-sandbox/src/lake/engine.rs`) can create and start a container, run one to its end, sweep by age since
  creation, and (through `LakeContainer::stop`) kill and remove it. It has **no** kill, inspect, wait or listing by attempt label;
  `remove` is forced and discards the container's state, including the out-of-memory flag the supervisor needs
  (`ICEBERG_LIMIT_EXCEEDED`, `crates/infra-lake/src/supervisor.rs`). The decision below depends on those additions (rule 7).
- The engine socket is host-root: a process that can create a container of its own choosing can mount the host, run privileged and
  read every other tenant's attempt directory. Only the sandbox runner holds it in production; only the integration tests link
  `LakeEngine` directly.
- D035-048 co-locates the lake runner and the sandbox runner on a node. The work volume is created by the deployment, mounted in both,
  and the container mounts its own attempt directory as a sub-path. ADR-0071, `plan.md` and `tasks.md` (T035-028) say the sandbox
  runner of a lake deployment serves only lake containers.
- The port the driver is written against exists (`crates/infra-lake/src/attempt.rs`): `LakeSandbox::{start, exited, kill}`. The relay is
  built **before** the container starts and `run_attempt` refuses a relay without a fixed peer user (`RelayConfig::with_peer_uid`,
  `attempt.rs:320`; `relay.rs:332`, `peer_allowed`): so the user id of the container must be known to the lake runner before `start`.
- The sandbox runner today is a `reqwest` client of the control plane plus an admin TCP listener (`apps/sandbox-runner/src/main.rs`);
  `sagat_api_http::serve` accepts only a `TcpListener`. Its sweeper runs every 300 s and removes by age since creation. The Compose
  service is "a development convenience, not a deployment" and the Helm chart has no sandbox-runner template.
- The plan fixes: an attempt deadline up to 60 min, `memory.max` 2 GiB for append and 6 GiB for upsert (up to 16 GiB), 2 to 8 lake
  containers per node (`plan.md`, limits table).

The question: how does the process that holds the credentials get a container without being able to choose what the container is?

## Decision

1. **Lake mode of the sandbox runner is exclusive.** A sandbox-runner process started in lake mode serves only lake containers: it has
   no control-plane runner token, runs no build or script step, and refuses to start if script or build configuration is present. A
   process in script mode has no lake listener. (Both modes are one binary and one image; they are never one process.)
2. **The lake runner holds no container-engine access.** `sagat-database-runner` and `sagat-infra-lake` MUST NOT depend, directly or
   transitively, on `bollard` or `sagat-infra-sandbox`, and a deployment MUST NOT mount the engine socket into the lake runner.
   (Integration tests may link `LakeEngine`; a test is not a deployment.)
3. **Transport.** A Unix socket in a directory that the sandbox runner owns (mode 0750, the lake runner's group may search it, not write
   it); the socket is bound under a restrictive umask and then set to 0660 in that group. On every accept the sandbox runner MUST read
   `SO_PEERCRED` and refuse a peer whose user is not the configured lake-runner user. That user is **required**: with none configured
   the listener does not start; 0 and 65534 (the overflow id of an unmapped user) are refused as a configuration. There is no TCP
   listener for this API. The listener is a small Unix accept loop built for this (`sagat_api_http::serve` is TCP-only).
4. **The listener's limits are explicit.** Request body at most 1 KiB (an explicit limit; the framework's default is 2 MiB), headers at
   most 4 KiB (a `traceparent` fits), a header-read timeout of 5 s, an idle timeout of 30 s, a cap on open connections and on
   in-flight requests of a small multiple of the ceiling, and at most the ceiling of `wait` calls at once. **Every** response, including
   the router's 404 and 405, an extractor rejection and a 413, is a closed body with no engine text.
5. **The API is three calls, HTTP/1.1 with JSON bodies, versioned in the path (`/v1`):**
   - `POST /v1/attempts/{attempt_id}` with `{"slot": n, "class": "small" | "large", "deadline_ms": n}` starts the attempt's container.
     It answers `{"result": "started"}`, `{"result": "refused", "reason": "ceiling" | "slot" | "class" | "deadline" | "directory"}` or
     `{"result": "unavailable"}`.
   - `GET /v1/attempts/{attempt_id}/wait` holds for at most 25 s and answers `{"state": "running" | "ended"}`; the client asks again.
   - `DELETE /v1/attempts/{attempt_id}` ends the container and answers `{"ended": bool, "memory_killed": bool}`.

   Requests are parsed with `deny_unknown_fields`, `deadline_ms` is an integer of at least 1, and `class` is a closed enum.
   **A request cannot name an image, a mount, a capability, a command, a limit, or any user outside the lake range.** The sandbox runner
   builds the `LakeSpec` from its own configuration: the image by digest, the work volume, the lake group, the owner label, and the
   **budget of the `class`** (memory, processes, CPU, work size, `/tmp`; the node configures `small` and `large`, and the lake runner
   maps append to `small` and upsert to `large` by its own table). The **slot is the lake runner's choice** within the node's
   configured range, because the relay needs the user before `start`; the user id is `uid_for_slot(slot)`, a pure function defined once
   in a crate both may depend on and pinned equal on both sides by a test. A slot is a number in a range: it selects one unused user of
   the lake range and nothing else. Over the range, or already used by a live container, the answer is `refused`.
6. **Admission holds across the creation.** The ceiling (containers and the sum of the reserved budgets against a node memory budget
   that is configured, and which must fit the node: checked at start-up) is enforced under a reservation that is taken **before** the
   engine is asked to create and released when the container is gone, so two concurrent starts cannot both pass. It never exceeds
   `MAX_SLOTS`. The lake runner also limits its own concurrency to the ceiling before it resolves a connection, so a refusal is the
   exception and not the normal case of a busy node.
7. **State is the engine's, not the process's.** Every container carries labels for its attempt, slot, class and **absolute deadline**.
   At start-up, and before the listener accepts, the sandbox runner rebuilds its reservations and slot table from them. `wait`, `kill`
   and an idempotent `start` resolve **by the attempt label**, never by an in-memory table alone: a second `start` for an attempt that
   has a container (running, or exited and retained) answers `started` and makes no second container. This needs `LakeEngine` to gain
   list-by-label, inspect, wait and kill-by-label that reads the out-of-memory flag **before** it removes the container. A container
   that exited is retained until `kill` or until the sweep removes it five minutes after its deadline, so `memory_killed` can still be
   read; `kill` for an attempt with no container answers `ended: true` only when the engine answered and showed none, and
   `ended: false` when it did not answer. `wait` for an attempt with no container answers `ended`; when the engine does not answer it
   answers `unavailable` and the client asks again.
8. **Every container has a hard deadline that does not depend on the process.** The deadline in `start` is refused, not clamped, if it
   exceeds the node's attempt ceiling (`refused`, reason `deadline`: a silent clamp would end a container after its request was sent
   and be reported as a retryable `LAKE_HOST_PROTOCOL` instead of `OUTCOME_UNKNOWN`). The sandbox runner ends the container at its label
   deadline, with an in-process timer and with a sweep of the labels **every 30 s and at start-up**, which is the guarantee across a
   restart. A lake runner that dies leaves nothing running past its deadline. `start` runs in a task of its own that a closed
   connection does not cancel, so a driver that drops its `start` future and then calls `kill` finds the container the start made (or
   waits for the start to settle, and then ends it).
9. **No tombstone for a `kill` that comes before its `start`.** A late `start` is not made harmless by a record in memory; it is harmless
   because (a) the lake runner stops the relay before it kills (`run_attempt`), so a container that starts late reaches no credential;
   (b) the lake runner removes the attempt directory after the kill, so a late `start` finds none and is refused (rule 10); and
   (c) the container has the label deadline of rule 8. This also leaves no unbounded state for a hostile `kill` to grow.
10. **A start is refused for a directory it cannot vouch for.** The attempt id is 1 to 60 lower-case hexadecimal digits and hyphens,
    starting with a hexadecimal digit (the stricter of the profile's and the staging code's rule), and generated by the lake runner as an
    opaque random value, never derived from a run or step id. Before it starts anything, the sandbox runner `lstat`s
    `<work volume>/<attempt_id>` without following links: it MUST be a directory and not a link, owned by the lake-runner user. **This
    check is advisory** (a swap after the check is possible, and every directory in the volume is the lake runner's): what confines the
    container is the engine's resolution of the sub-path inside the volume, which is a precondition tested by a symlink-swap case of
    the isolation suite with a minimum engine version. The sandbox runner mounts the work volume **read-only**; it only needs `lstat`.
    The lake runner already owns, and holds the credentials of, every attempt directory in its own volume, so what the check bounds is
    a container seeing another attempt's directory of the same lake runner, never a path outside the volume.
11. **Failure says nothing about the engine, and the operator's log is not the response.** A failure of the engine, the volume or the
    image is `unavailable`, with the engine's own words in the sandbox runner's log for the operator only (they may carry host paths and
    image references; that log is the operator's and is not a response). The lake runner never receives them, and no response, metric
    or span carries a secret or a tenant's data (the sandbox runner does not know the tenant).
12. **Telemetry, with no high-cardinality label.** Gauges for lake containers running and the ceiling (saturation), counters for starts,
    refusals by the closed reason, deadline kills and sweeps; the attempt id is a span attribute and never a metric label; the call
    carries a `traceparent`. The lake runner emits the same refusal counter and keeps **audit**, because it knows the tenant.
13. **Drain on shutdown.** On SIGTERM the sandbox runner stops accepting `start` (`unavailable`), serves `wait` and `kill` for at most
    30 s, and exits **without killing running containers**: in lake mode it holds no engine handle with a removal guard, only names and
    labels. The next process rebuilds from the labels (rule 7) and the label sweep ends what is past its deadline. The lake runner, on
    its own shutdown, ends its attempts through `run_attempt`.
14. **The wire types have one home and one test.** The request, response and client live in `crates/infra-lake` (a module with no
    engine dependency); `apps/sandbox-runner` depends on `infra-lake` for them. A shared JSON corpus is read by both sides, and one
    test runs the real client against the real server over a real socket. The client treats any response it does not recognise (an
    unknown variant, an undecodable body, an unknown reason) as `unavailable` and **never** as `started` or `ended`; `wait` retries
    on a transport error and never resolves it. A new request member, or a new response variant, is a `/v2` path served next to `/v1`
    for one release; the sandbox runner is upgraded first.
15. **The port changes with it.** `StartRequest` gains `slot` and `class`; `SandboxError::Refused` and `Unavailable` stay distinct
    in the driver's metrics and the failure detail, and both end the attempt as `LAKE_SANDBOX_UNAVAILABLE` (retryable). A refusal counts
    as an attempt like any retryable error; rule 6 makes it rare.

## Considered options

1. **A node-local Unix-socket API of three calls on a lake-mode sandbox runner (chosen).** The credential-holding process cannot choose
   what a container is, the engine socket stays where it is, and the added surface sits in a process that runs nothing else, so it is
   also the smallest separate broker this decision needs (a third binary would hold the same engine socket and add a deployment unit).
2. **The lake runner links `LakeEngine` itself.** Rejected for production: it hands the host-root engine socket to the process that
   handles destination credentials and customer data (D035-048). It stays acceptable in the integration tests.
3. **A job brokered by the control plane and claimed by the sandbox runner like a build.** Rejected as the primary route: a table, a
   lease type and polling latency on every attempt, and the control plane then sees the attempt id and class for every lake write.
   It is the **fallback** if a node-local socket cannot be shared on Kubernetes, which T035-029 decides; that would need its own ADR.
4. **The sandbox runner allocates the slot.** Rejected: the relay needs the user before `start` (`run_attempt` binds it first), so a
   late-bound peer would be needed in the relay, with credentials held back until the user is known. Letting the lake runner choose a
   slot in a range, which the sandbox validates, keeps the relay as it is and gives up nothing: a slot selects an unused user and nothing
   more.
5. **Tombstones for a kill that precedes its start.** Rejected (rule 9): unbounded state in the process that holds the engine socket,
   lost on restart, and unnecessary given the relay stop, the directory removal and the label deadline.
6. **gRPC; TCP with mutual TLS.** Rejected: three unary calls do not earn code generation; the two processes share a node (D035-048),
   and a network listener turns a local trust decision into one that depends on certificates and a firewall.
7. **A two-call API (start, kill) with no `wait`.** Considered: the host-start timeout covers a container that died before connecting.
   Kept `wait` because the supervisor already distinguishes a container that is gone (image missing, killed on import, a crash in the
   first second) from a silent host, and the port has `exited`; the cost is a polling read of the engine bounded by the ceiling.

## Consequences

- **Good:** a compromised lake runner can start pinned containers on directories of its own, in the lake range of users, from two
  configured budgets, up to a ceiling; it can kill them. It cannot pick an image, a mount, a capability, a command, a limit or a user
  outside the range. The engine socket never enters the process with the credentials, and the process that holds it runs nothing else.
- **Bad, stated:** the ceiling is shared by every tenant of the node, so a compromised lake runner can fill it and starve the other
  tenants' lake attempts there (denial of service). Per-tenant fairness is the lake runner's admission (the namespace and tenant
  quotas of T035-055); a tenant that must not share a ceiling has its own lake deployment (ADR-0071). The peer check separates users,
  not tenants.
- **Cost:** a Unix listener and its limits, an engine API of list, inspect, wait and kill-by-label, a work volume mounted read-only, a
  slot range, two budgets and a shared socket directory. Compose gains a lake-mode service; **the Kubernetes shape is unwritten**
  (there is no sandbox-runner template) and T035-029 writes it.
- **Compatibility:** a new surface; nothing existing changes. The sandbox runner has its own switch: lake mode is on only when its
  configuration names the socket directory and the lake-runner user. **Rollback:** stop the lake-mode process (no listener remains)
  and set `lake.enabled` off at the control plane; running attempts end at their label deadline. The control plane's `lake.enabled` does
  not stop the sandbox runner by itself.
- **Harder:** the budgets of the sandbox runner and the limits a workflow may ask for (`plan.md`) must stay in step; a memory kill of
  an upsert on a node whose `large` budget is too small is `ICEBERG_LIMIT_EXCEEDED`, which is not retryable, so the node's `large` budget
  is a deployment decision to document in the runbook. The tmpfs work volume lives in node memory: orphaned attempt directories of a
  restarted lake runner are the lake runner's to sweep at start-up, and the volume's use is its gauge (the driver slice).

## Open

- **Kubernetes (T035-029).** `SO_PEERCRED` compares user ids, and two containers or pods often share one non-root id, in which case the
  peer check separates nothing; the shared node-local socket needs two containers of one pod or a node-local volume. Rules 3 and 5 are
  written for the Compose shape and are conditional on T035-029; if no shape works, option 3 replaces this ADR's transport.
- Defaults for the ceiling, the two budgets, the slot range and the sweep intervals (the plan gives 2 to 8 containers, 2 and 6 GiB).

## Evidence

- `LakeSandbox`, `StartRequest`, `ContainerExit` (`crates/infra-lake/src/attempt.rs`) and the supervisor table (D035-105 to D035-111) are
  written against this API; `container_config` and `LakeEngine` (`crates/infra-sandbox/src/lake.rs`, `lake/engine.rs`) are the profile.
- Research: D035-048 (co-location), D035-086 (group, not user, separates), D035-110 (d) (the recommendation), D035-116 (this decision),
  G035-022, G035-055, G035-056.
- **Independent review of the first draft (security-reviewer and architecture-reviewer, given the ADR and the files, not my conclusion),
  every claim checked against the code before it was taken.** Taken: the relay needs the user before `start` (Blocker: rule 5, slot);
  lifecycle state lived in memory (rules 7 to 9; the sweeper ran every 300 s and by age since creation); the ADR diverged from ADR-0071
  and the plan on a lake-only sandbox runner (rule 1); facts said "already true" that were not (engine kill, inspect and listing;
  `lake.enabled` as the sandbox runner's switch; a listener; an axum server that takes a Unix socket; a Helm template); one memory
  budget against the plan's two (rule 5, `class`); a silent deadline clamp (rule 8); listener limits unenforced (rule 4); the
  `lstat` check named as a guarantee (rule 10); unbounded tombstones and an `ended: true` answered from memory (rules 7 and 9);
  shutdown and drain, wire-type ownership and version skew (rules 13, 14); an attempt-id rule that disagreed with the staging code
  (rule 10); the dangling D035-116 (now recorded). **Not taken:** a per-peer request rate limit (one authorised peer; the connection and
  concurrency caps bound the resources); a separate broker binary (rule 1 and option 1 give the same isolation without a third unit).
- **Not yet built or measured:** the API, the engine additions, the ceiling and deadline behaviour, the symlink-swap case and a minimum
  engine version, the Compose service and the two Kubernetes shapes. The checks under "Enforced by" do not exist yet.
