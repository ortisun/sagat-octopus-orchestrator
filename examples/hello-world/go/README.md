# hello-world in Go

The two-step workflow of `templates/hello-world` (`greeting.compose`, then `greeting.deliver`) served
and started with the [Go SDK](../../../sdks/go/README.md): one program that is the worker and the
client.

## Run it

You need the local stack running (see [the quickstart](../../../docs/quickstarts/README.md)), the
`sagat` CLI, and Go 1.25 or newer.

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)      # from the local stack's development identity provider

sagat new hello-world                      # the workflow (skip it if you have it)
sagat workflow publish hello-world/workflow.json
sagat worker bind --workflow hello-world/workflow.json     # let your identity serve its two activities

./prepare.sh        # build bin/hello-world
./run.sh            # bin/hello-world [NAME]
```

It prints, in every language:

```text
serving greeting.compose and greeting.deliver at protocol 1.2
run 01a12b51-158b-7147-b9b3-847882a5c939 started
delivering Hello, Ada!
run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

and `sagat workflow inspect <run>` shows the journal: the same one whatever the language.

## What to change

- The activities are the two `RegisterActivity` calls. What a step receives is the input of the run
  (here `Person`) and, in `worker.TaskFrom(ctx).Steps`, the outputs of the steps before it by step id;
  what it returns is its output.
- A real effect (a payment, an e-mail) sends `worker.TaskFrom(ctx).IdempotencyKey` to its provider: it is
  the same for every attempt of the step; the provider must deduplicate that key to prevent duplicate effects.
- `go.mod` points at the SDK in this checkout (`replace`); remove that line once the SDK has a tag.

## Interface

- The `main` package runs the local `run` driver and its activity handlers. [examples/hello-world/go/main.go:main]

## Invariants

- The four connection settings are mandatory; missing settings exit 2. [examples/hello-world/go/main.go:setting]
- Registers greeting compose/deliver; a malformed compose output yields permanent `NO_GREETING`. [examples/hello-world/go/main.go:run]
- The command waits at most 60 seconds and treats a terminal status other than `SUCCEEDED` as an error. [examples/hello-world/go/main.go:run]
- The Go 1.25 module replaces its SDK dependency with the checkout SDK. [examples/hello-world/go/go.mod:replace]

## Side Effects

- Builds `bin/hello-world`. [examples/hello-world/go/prepare.sh:go build]
- Launch replaces the shell with the compiled executable. [examples/hello-world/go/run.sh:exec]
- Starts/stops the worker, starts/waits for a workflow, handles OS interrupt, and prints diagnostics. [examples/hello-world/go/main.go:run]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
