# hello-world in Python

The two-step workflow of `templates/hello-world` (`greeting.compose`, then `greeting.deliver`) served
and started with the [Python SDK](../../../sdks/python/README.md): one program that is the worker and the
client.

## Run it

You need the local stack running (see [the quickstart](../../../docs/quickstarts/README.md)), the
`sagat` CLI, and Python 3.10 or newer.

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)      # from the local stack's development identity provider

sagat new hello-world                      # the workflow (skip it if you have it)
sagat workflow publish hello-world/workflow.json
sagat worker bind --workflow hello-world/workflow.json     # let your identity serve its two activities

./prepare.sh        # a virtualenv in .venv with the SDK (once; safe to repeat)
./run.sh            # .venv/bin/python main.py [NAME]
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

- The activities are the two `register_activity` calls. What a step receives is the input of the run (the
  first argument) and the outputs of the steps before it (`ctx.steps["<id>"]`); what it returns is its output.
- A real effect (a payment, an e-mail) sends `ctx.idempotency_key` to its provider: it is the same for every
  attempt of the step; the provider must deduplicate that key to prevent duplicate effects.
- `prepare.sh` installs the SDK from this checkout (`pip install ../../../sdks/python`), because it is not on
  PyPI yet. Once it is, replace that line with `pip install sagat-octopus-orchestrator`.

## Interface

- `main` and `run` drive the local command; `compose` and `deliver` implement its activities. [examples/hello-world/python/main.py:run]

## Invariants

- The four Sagat connection settings are required; missing settings exit 2. [examples/hello-world/python/main.py:setting]
- Registers compose/deliver activities, uses the prior compose output, and defaults the name to Ada. [examples/hello-world/python/main.py:run]
- The wait is bounded to 60 seconds and success requires `SUCCEEDED`. [examples/hello-world/python/main.py:run]
- Preparation installs hash-pinned dependencies, then the checkout SDK with `--no-deps`. [examples/hello-world/python/prepare.sh:--require-hashes]

## Side Effects

- Creates and populates `.venv` and verifies the SDK import. [examples/hello-world/python/prepare.sh:pip install]
- Starts/stops a worker, starts/waits for the workflow, and prints activity/run/error diagnostics. [examples/hello-world/python/main.py:run]
- Replaces the shell with the virtualenv Python process. [examples/hello-world/python/run.sh:exec]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
