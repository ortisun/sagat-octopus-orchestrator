# ADR-0013 — Scripts run in a container of their own, started by a runner the control plane never lets near its database

Status: **Accepted** | Date: 2026-10-09 | Feature: 005 | Supersedes nothing; refines ADR-0006

The C# language/dependency restrictions below are superseded by
[ADR-0021](ADR-0021-csharp-builds-use-a-curated-offline-toolchain.md), which introduces a curated
offline Roslyn toolchain and versioned binary bundles. All isolation boundaries still apply.

## Context

ADR-0006 decided that scripts are untrusted execution: compiled in build workers, run in isolated
containers or pools, under quota and an egress policy. Feature 005 has to make that real: an author
writes TypeScript or Python in the platform, tries it, publishes it, a workflow pins it, and a hostile
script — one that reaches for the cloud metadata service, the host, a secret it was not given, or
all the CPU and memory it can get — is held to its limits and audited.

Three forces shape the design:

1. **Nothing in a language is a sandbox.** `vm`, `ast`, `AssemblyLoadContext` and the like are not
   security boundaries (`docs/architecture/security-and-sandbox.md`). The boundary has to be the
   kernel's: namespaces, cgroups, capabilities, seccomp.
2. **The party that holds the container engine is root on its machine.** Whoever can talk to the
   engine's socket can start a privileged container that mounts the host. The control plane (which
   holds the database, the signing key and every tenant's data) must not be that party.
3. **A version has to be exactly what was built.** Workflows pin an artifact; a database row written
   by anything other than the platform must not become code that runs.

## Decision

**A container per build and per run, started by a separate runner process.**

* The sandbox is a container (`infra-sandbox`, over the engine's API) created for one build or one
  run and removed after it: no network interface (`none`), a read-only root filesystem, a small
  writable `/tmp` mounted `noexec,nosuid,nodev` and size-limited, every capability dropped,
  `no-new-privileges`, the engine's default seccomp profile, user `65534`, a private PID and IPC
  namespace, memory with swap off, a CPU quota and a CPU-time limit, a limit on processes, on file
  size and on open files, no log driver, no mounts. The strength of that is the strength of the
  runtime under it (`runc` here); gVisor or Kata are a matter of the engine's configuration, not of
  the code, and are what a shared deployment should use (G98).
* **Code in, one line out.** The container's root is read-only, so nothing can be uploaded into it:
  the source (or the artifact's files) and the input travel as the first message on the container's
  standard input and the shim writes them to the scratch area. The result is one JSON line on
  standard output; standard error is the log, redacted and bounded. Everything the script wants from
  the outside — an HTTP request, a secret — it asks of its supervisor over the same channel, and the
  supervisor may refuse. The script has no network of its own.
* **The runner is a separate program (`apps/sandbox-runner`).** It holds the container engine and
  one tenant's `runner` token; it has no database, no signing key and no secrets of its own. The
  control plane never starts a container. They meet over REST (builds, test runs, artifacts, the
  egress policy, execution records) and over the platform's own worker protocol (script steps), so
  leases, heartbeats, retries and timeouts of a script step are the engine's, not a second
  mechanism. A new role, `runner`, holds exactly `worker:execute`, `script:build` and
  `script:execute`; an ordinary worker holds neither of the last two, so a worker that serves
  activities never reads the source of a script.
* **An artifact is a signed, content-addressed bundle.** The build makes a canonical JSON bundle:
  the code, the policy it was built under (limits and network) and the schemas of its input and
  result. Its digest is its identity; the control plane signs `tenant ‖ digest ‖ language` with an
  Ed25519 key it alone holds (`SAGAT_SCRIPT_SIGNING_KEY`) and the runner verifies the digest of the
  bytes and the signature before it runs anything. A row planted in the database, or edited in
  place, does not verify (and a trigger refuses to update an artifact or a finished version at all).
  The control plane signs only a bundle that says what the version says (its language, the policy it
  was clamped to, its schemas, plain file names and an entry point among them), and the key it files
  the build under is the one it handed out with the claim, not one worked out from the report.
  Because the policy and the schemas are part of the bytes, a digest decides what a script runs
  under, and the same code under other limits is another artifact.
* **A build is reusable only if nothing that matters changed.** The build key is the SHA-256 of the
  length-prefixed language, source, lockfile, toolchain (the digest of the image and the version of
  the runner's shim), policy and schemas, scoped to the tenant. A change of any of them builds again;
  the same key is never built twice, and the runner is not even handed a build that exists.
* **A workflow pins a digest, not a name.** A `script` node becomes the activity
  `script.sha256-<hex>`. Publishing a workflow checks that every pinned digest is a ready artifact of
  the tenant (`UNKNOWN_SCRIPT` otherwise). Publishing a newer version of the script changes nothing
  about a workflow that names the old digest.
* **The runner serves script steps by advertising what is ready.** A worker session advertises exact
  activity names (at most 256), and script activities are named by digest, so the runner asks the
  control plane which script activities have a step ready and registers a session for those. No change
  to the worker protocol (G103 records the cost). The engine keeps one session per identity and name,
  so the runner keeps **one session per namespace** and registers it again, with the activities it
  already advertised and the new ones, when something new is ready.
* **Test runs are the same machinery without production's rights.** A test run builds the draft and
  runs it with the author's input; it has its own quota (shared with builds: work the sandbox does for
  authors), no secrets of the namespace — ever — and the network only if both the draft's policy and
  the namespace's egress policy allow it.
* **A secret is a name in a step, a value in a lease.** A step declares the secrets its script may
  ask for; the runner takes them with `ResolveSecrets` (so the lease is the capability, the access
  list of the secret applies and the access is audited) and hands out only those by name. Every value
  the run was handed is searched for in the log and the result before either is kept.
* **What a script was refused is data.** The supervisor records `fetch <scheme>://<host>` and
  `secret <name>` (never a path, a query or a value), the runner reports them with the execution, and
  the control plane audits each distinct one once — a script in a loop cannot grow the trail.

## Why this and not the alternatives

| Alternative | Why not |
|---|---|
| Run the script in the control plane's or the engine's process (V8 isolates, WASM, `vm`) | No language boundary is a security boundary, and AGENTS.md rule 7 forbids it. |
| The control plane starts the containers | It would hold the engine's socket next to the database and the signing key. One escape and one compromised process would hold both. |
| One long-lived container per tenant | A script could leave something behind for the next one (a file, a process, a poisoned module). A container per run costs about a second and leaves nothing. |
| Upload the code with the engine's archive API | Refused for a read-only root, and a writable root is a disk the script can fill (the engine has no per-container quota without XFS project quotas). Standard input costs nothing. |
| Name script activities `script.run` and carry the digest in the input | Needs the engine to put a node's digest in every step document, and makes the activity name stop saying what the step runs. |
| Pattern advertisement (`script.*`) in the worker protocol | The claim query is a lateral index probe per advertised activity; a prefix match would defeat it on the hottest path of the engine, for the benefit of one feature. |
| Sign nothing; trust the database | A row written by anything with database access would run as code. The signature moves the root of trust to a key the database never holds. |

## Consequences

* The runner is a high-value host: whoever controls it controls its engine (G98). It should run on a
  machine that holds nothing else, with a rootless or sandboxed engine where possible, and its token is
  worth exactly one tenant's scripts.
* One runner serves one tenant (its token's); a shared pool is a decision not taken (G99).
* Dependencies cannot be installed in v1: a lockfile that pins a package is refused with the reason,
  not ignored (G97). That keeps the supply chain closed instead of pretending to manage it.
* Artifacts live in PostgreSQL (at most 2 MiB each). An object store is the answer if that is not
  enough (G109). What an author can leave behind is bounded by counts (500 versions per script, 100
  finished test runs kept, 100 different refusals audited), not yet by bytes (G117).
* A supervisor never waits on a script or on the engine without a deadline: writes to the script are
  bounded by the run's wall clock, the channel by a number of lines, every engine call by 20 s and a
  whole run by its wall clock plus 60 s (G119). A script that stops reading, or floods, ends its own
  run.
* C#, Go and Java are named by the workflow schema and refused with `LANGUAGE_NOT_BUILT` (G96).

## Review criteria

Open a new ADR with benchmark data, failure tests, operational cost and compatibility with existing
runs if: the sandbox moves off the container engine; the runner gains a database or the signing key;
a workflow can name a script by anything but a digest; or a script gains a way to the network that is
not the supervisor.
