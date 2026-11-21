# hello-world in TypeScript

The two-step workflow of `templates/hello-world` (`greeting.compose`, then `greeting.deliver`) served
and started with the [TypeScript SDK](../../../sdks/typescript/README.md): one program that is the
worker and the client.

## Run it

You need the local stack running (see [the quickstart](../../../docs/quickstarts/README.md)), the
`sagat` CLI, and Node.js 20 or newer.

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)      # from the local stack's development identity provider

sagat new hello-world                      # the workflow (skip it if you have it)
sagat workflow publish hello-world/workflow.json
sagat worker bind --workflow hello-world/workflow.json     # let your identity serve its two activities

./prepare.sh        # once: npm ci for the SDK and for the example, then tsc
./run.sh            # node dist/main.js [NAME]
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

- The activities are the two `registerActivity` calls in `src/main.ts`. A handler receives the input of the
  run (its first argument) and the outputs of the steps before it (`ctx.steps.<id>`); what it returns is its
  output.
- A real effect (a payment, an e-mail) sends `ctx.idempotencyKey` to its provider: it is the same for every
  attempt of the step; the provider must deduplicate that key to prevent duplicate effects.
- `package.json` points at the SDK in this checkout (`"@sagat-octopus-orchestrator/sdk":
  "file:../../../sdks/typescript"`), which `prepare.sh` builds first; replace the line with the version of
  the package, and drop the SDK build from `prepare.sh`, once the SDK is published.

## Interface

- `prepare.sh` builds the checkout SDK and example; `run.sh` launches compiled `dist/main.js`. [examples/hello-world/typescript/prepare.sh:npm_install]
- Package scripts expose `build` and `start`. [examples/hello-world/typescript/package.json:scripts]
- [src](src/README.md) — TypeScript executable serving and starting the canonical greeting workflow.

## Invariants

- Preparation is fail-fast and disables dependency lifecycle scripts, audit, and funding output. [examples/hello-world/typescript/prepare.sh:set -euo pipefail]
- The package requires Node 20 or newer and references the checkout SDK. [examples/hello-world/typescript/package.json:engines]
- Compilation uses strict TypeScript with NodeNext modules and ES2022 output. [examples/hello-world/typescript/tsconfig.json:compilerOptions]

## Side Effects

- Installs locked SDK/example dependencies and writes compiled output to `dist`. [examples/hello-world/typescript/prepare.sh:npm_install]
- Replaces the shell with Node and forwards arguments. [examples/hello-world/typescript/run.sh:exec]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
