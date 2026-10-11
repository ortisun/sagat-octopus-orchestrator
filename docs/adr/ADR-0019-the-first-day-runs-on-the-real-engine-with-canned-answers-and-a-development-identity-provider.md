# ADR-0019 — The first day runs on the real engine with canned answers, a development identity provider is a separate thing that refuses to run anywhere else, and the documentation is checked by the tool it describes

Status: **Accepted** | Date: 2026-10-11 | Feature: 012 | Builds on ADR-0004 (external effects are at least once, so the key is the provider's), ADR-0007 (the SDKs implement one worker protocol), ADR-0015 (SDKs meet the server by minor version and are held to one conformance suite), ADR-0016 (the Studio is a same-origin app over the REST API) and ADR-0018 (the command line is a client of the API)

## Context

Feature 012 asks that a developer can go from nothing to a two-step workflow that runs and can be seen, in any
of five SDKs, with a saga they can make fail, a command line that tells them what is wrong, and documentation
that is generated from the contracts. Each of those has a tempting answer that fails in a way that shows only
later:

1. **A local emulator of the engine** ("so that nothing has to be running"). It has its own idea of a retry, a
   compensation, a reconciliation, a timer and a lease. A workflow that passed on it can fail on the platform for
   reasons nobody can see, which is worse than having no way to try it.
2. **Mocks inside each SDK.** Five implementations of what a mock is, and none of them can say what the platform
   would do with a task that never comes back.
3. **A development identity provider that is part of the server** (a flag, `--insecure-dev-auth`). The flag is one
   misconfiguration away from a production server that accepts a token from anyone. The repository had no
   provider at all (gap G10), so the local stack could not be used without one of the developer's own.
4. **Documentation that is written, and reviewed when someone remembers.** Commands and flags drift from the tool,
   excerpts from the examples, links from the files, a reference from the contract.
5. **Examples that look right.** The first Go example did not work: it read the step document where the SDK
   hands a handler the run's input, and made `Hello, !`. Nothing that only checked that a run succeeded could see it.

## Decision

**1. There is no second engine. `sagat worker dev` is a real worker that answers from a file.** It registers a
session, takes leases and reports through the worker protocol an SDK speaks, so the retry, the compensation, the
reconciliation, the lease and the fencing that happen around it are the platform's own. What is canned is the
answer, and the file can say that a worker dies with its task (`lose`): the lease then has to expire and the
platform reconciles. A compensation not in the file succeeds, a reconciler not in the file says `unknown`, an
activity not in the file fails for good as `NO_MOCK`, and the tool says which names it is serving and which have
no answer. `worker dev --bind` and `worker bind` put the identity of the token in the namespace's worker bindings for
those names, **replacing** what the identity was bound to, and say so.

**2. The command line is a client of the API and nothing else.** It validates documents with the same codec the
server uses (`sagat-infra-ir`), so a document that validates is one the platform accepts, and every finding has a
code, a help line and a link into `docs/quickstarts/errors.md`; two tests keep the table and the page in step
in both directions. It reads the token from `--token-file` or `SAGAT_TOKEN`, never an argument; it sends it only to
an https address or an http address on loopback, uses no proxy for http and follows no redirect; it prints nothing
that somebody else wrote without turning control and bidirectional characters into `?`; a parse error never quotes
the file. `GET /v1/version` (authenticated, no permission) lets `doctor` compare the build, the API and the worker
protocol range of the platform with the tool's.

**3. The development identity provider is a binary of its own, in an image stage of its own, and it refuses to
run anywhere but where it was meant to.** It signs an ES256 token for any subject of any tenant it is asked for,
which is the whole point and the whole danger. So: `SAGAT_ENV=dev` exactly, or it exits; a loopback address unless
`SAGAT_DEV_IDP_EXPOSED=true` (the container sets it, and compose publishes the port on the host's loopback only);
a `Host` allowlist (DNS rebinding); `application/json` only and at most 4 KiB (what a page of another origin can
send without asking first is neither); subjects the platform accepts and not its reserved `system:` actors; a life
of at most the hour the platform accepts; no CORS headers; a key in a file only its owner reads, which it refuses
to use if others can. It is not in the control-plane or engine image and not started by anything but the local
compose file. A token is still only a signature: the platform lets in the members of a tenant, so a token for
someone the seed did not make is `401`. `sagat dev token` asks it and prints the token alone (or writes it to a
file with mode 600).

**4. The examples are one program that is the worker and the client, configured by the names the tool
already reads** (`SAGAT_SERVER`, `SAGAT_GRPC`, `SAGAT_NAMESPACE`, `SAGAT_TOKEN`), and they print the same lines in
every language. They use the SDK from the checkout until it is published, and say where a line changes then. One
end-to-end test runs them all against a real platform and holds them to the same story: the exact output of each
step (as the digest the journal keeps of it), the line that proves the data went from the run's input through
the first step into the second, the same journal as each other and as `worker dev`'s, and a trace on every event.

**5. The documentation is checked by the tool it describes.** The reference pages (`docs/reference/api.md`,
`workflow-document.md`, `cli.md`) are generated from the OpenAPI document, the workflow schema and the tool's own
help text, and a contract test fails when a committed page is not what a regeneration gives. The same test
parses every `sagat ...` line in the shell blocks with the tool's own argument parser, requires a code block
marked `excerpt=<path>` to be in that file as shown, requires the scripts a README runs to exist, and follows every
relative link. A generator was not built as a separate tool: a test that regenerates and compares is the check,
and `SAGAT_DOCS_BLESS=1` is the regeneration.

**6. The local stack serves what a first day needs and nothing a second day would take for granted.** Compose
starts the provider; `.env.example` points the API and the engine at it; the seed leaves a workspace and a
namespace for the tenant it makes; the control-plane image carries the Studio's files but serves them only when
`SAGAT_STUDIO_DIR` says where they are, which compose says and nothing else does.

## Consequences

- A developer needs the stack, the tool and a toolchain, and nothing else. A change to the retry or the
  compensation of the engine is a change to what `worker dev` shows, with no second place to update.
- The provider is a tool that must never be mistaken for a feature of the platform. The risk is a person who
  copies the compose file to a shared host: `.env.example` says what to change, the runbook says why, and the
  provider's own refusals (`SAGAT_ENV`, the loopback, the host) are what stand between that and a server that
  signs for anyone. It does not stop an operator who sets all three on purpose.
- The examples cost five toolchains in CI (the `sdks` job already has them).
- The generated pages are a map of the contract, not a copy of it; the OpenAPI file stays the reference.
- Mock files cannot say "answer by what the input was" or keep state beyond a sequence (G221). A real worker, or
  an SDK example, is the way to a provider with memory, and the payment saga example is that.

## Alternatives not taken

- An in-process engine for `worker dev` (1); mocks in the SDKs (2); an environment switch in the server (3);
  generating the pages with a separate binary (the check would then have to run the binary, and a developer
  would have one more thing to build); signing documentation excerpts rather than comparing them (the file is
  the source of truth).
