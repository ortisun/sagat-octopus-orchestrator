# Quickstart: a workflow running in minutes

From nothing to a two-step workflow that runs, that you can watch in the Studio, and that you can serve with an SDK
in your language. Nothing here needs an identity provider of your own, a broker or a database to be set up by hand.

> **NOT for production.** The local stack has development credentials, single instances, no TLS and an identity
> provider that signs a token for anyone who asks. Read [the runbook](../runbooks/developer-experience.md) before
> reusing any of it.

## 1. What you need

| For | You need |
|---|---|
| The platform | Docker with Compose |
| The `sagat` command | Rust (the version in `rust-toolchain.toml`); there is no prebuilt binary yet |
| An SDK | the toolchain of your language: Node 20+, Go 1.25+, Python 3.10+, .NET 10 (the SDK also targets 8) or a JDK 17+ |

```sh
cargo install --path apps/cli --locked      # puts `sagat` in ~/.cargo/bin
```

## 2. Start the stack

```sh
cp infra/docker/.env.example infra/docker/.env
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env up -d --build
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env --profile seed run --rm seed
```

The first command builds the images (a few minutes the first time). It starts PostgreSQL, NATS, the control plane
(the REST API and the Studio), the execution engine (the worker API) and the development identity provider. The
seed makes a tenant `dev`, its administrator `dev-admin`, and a namespace `payments`; it is safe to run again.

Everything listens on `127.0.0.1` only:

| What | Address |
|---|---|
| REST API | `http://127.0.0.1:18080/v1` |
| Studio | `http://127.0.0.1:18080/studio/` |
| Worker API (gRPC) | `http://127.0.0.1:19000` |
| Development identity provider | `http://127.0.0.1:18090` |

## 3. Tell `sagat` where things are, and who you are

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)      # a token of dev-admin of tenant dev, valid for an hour
sagat doctor
```

`sagat doctor` checks, one thing at a time, the address, the token, that the platform answers and is ready, that it
knows you and what you may do, that the versions fit, that the namespace is there, that the worker endpoint accepts
connections, and that the Studio is served. It says what to do about each thing that is wrong, and ends with
`nothing to fix` when there is none. A token ends after an hour: run `sagat dev token` again.

Onboarding telemetry is off by default. To keep a local, append-only NDJSON report with one
structured completion event and one duration metric point per command, opt in explicitly:

```sh
export SAGAT_ONBOARDING_TELEMETRY_FILE="$PWD/sagat-onboarding.ndjson"
```

To send the command span, correlated completion log and duration metric to an OpenTelemetry
Collector instead, set the standard common `OTEL_EXPORTER_OTLP_ENDPOINT`. A signal-specific
`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` sends only the span. Plain HTTP is accepted only for a
collector on loopback; remote collectors must use HTTPS. The CLI sends no telemetry unless one of
these settings or the local report is configured. Command and outcome labels come from fixed sets;
tokens, inputs, payloads, paths and URLs are never recorded.

## 4. Your first workflow, before you have written a worker

```sh
sagat new hello-world                                   # a two-step workflow, and the answers to give it
sagat workflow validate hello-world/workflow.json       # the platform's own checks, run here
sagat workflow publish hello-world/workflow.json
```

In a second terminal, serve its two activities from a file of canned answers and leave it running:

```sh
cd /path/to/sagat-octopus-orchestrator       # the same checkout root used in the first terminal
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)         # each terminal needs its own environment
sagat worker dev --mocks hello-world/mocks.yaml --workflow hello-world/workflow.json --bind
```

Back in the first, run it and wait for it:

```sh
sagat workflow run hello-world --input '{"name":"Ada"}' --wait
```

You get the status, the journal (every event, in order) and the attempts. `sagat workflow inspect <run>` shows them
again whenever you like.

`worker dev` is a real worker: it registers, takes leases and reports through the same protocol an SDK speaks, and
the engine that runs the workflow is the platform's own, so a retry, a compensation or a reconciliation behaves as it
will in production. What is canned is the answer: [`mocks.yaml`](../../templates/hello-world/mocks.yaml) says, for
each activity, what it returns or how it fails (`fail`, `sequence`, `lose` for a worker that dies with the task,
`delayMs`). `--bind` lets your identity serve the activities of the workflow; it says so, because it replaces what
the identity served in the namespace.

## 5. See it in the Studio

Open `http://127.0.0.1:18080/studio/`, choose to sign in with a token, and paste the output of `sagat dev token`
(the token is held in the page's memory only). The run is under **Runs**: the graph, the table and the timeline are
the same journal `sagat workflow inspect` printed.

## 6. Serve it with an SDK

Replace `worker dev` with a program of yours. Each example is one program that is the worker and the client, and
they print the same lines and make the same run:

| Language | Page | Example |
|---|---|---|
| TypeScript | [typescript.md](typescript.md) | [`examples/hello-world/typescript`](../../examples/hello-world/typescript) |
| Go | [go.md](go.md) | [`examples/hello-world/go`](../../examples/hello-world/go) |
| Python | [python.md](python.md) | [`examples/hello-world/python`](../../examples/hello-world/python) |
| C# | [dotnet.md](dotnet.md) | [`examples/hello-world/dotnet`](../../examples/hello-world/dotnet) |
| Java | [java.md](java.md) | [`examples/hello-world/java`](../../examples/hello-world/java) |

Before running one, let your identity serve the activities (the example's README has the whole sequence):

```sh
sagat worker bind --workflow hello-world/workflow.json
```

## 7. Rehearse a failure

[saga.md](saga.md) is a payment saga: reserve a limit, charge a card, tell the customer. Make the card be declined
and watch the platform compensate; make the provider take the money and lose the answer, and watch it ask the
provider instead of charging twice. First with canned answers, then with a worker of your own.

## 8. When something goes wrong

- A document the platform will not take: [errors.md](errors.md) has each code, what it means and what to do.
- A machine that is not ready: `sagat doctor`.
- `sagat` exit codes: `0` done, `1` failed, `2` usage, `3` waiting for approval, `4` refused as breaking, `5` differences,
  `6` it did not finish in time, or waits for a person (`workflow run --wait`, `script test`, `script publish`).

## 9. Stop and clean up

```sh
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env down        # keeps the data
docker compose -f infra/docker/compose.yaml --env-file infra/docker/.env down -v     # forgets it, and the provider's key
```

## What this does not cover

- **A person's first day has not been timed.** The goal is under fifteen minutes with the dependencies installed
  (`NFR-012-PERF`); this was measured with a script (see the report in `tests/performance/reports/developer-experience.md`),
  not with people (`SC-012-001` is open).
- There is no prebuilt `sagat`, no published SDK package and no hosted documentation yet: the SDKs are used from this
  checkout, and the examples say where to change a line once they are published.
- The Java and C# examples download their dependencies on first use.
