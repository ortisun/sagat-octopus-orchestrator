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
./run.sh            # npm start -- [NAME]
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
  attempt of the step, so a repeat does not do it twice.
- `package.json` points at the SDK in this checkout (`"@sagat-octopus-orchestrator/sdk":
  "file:../../../sdks/typescript"`), which `prepare.sh` builds first; replace the line with the version of
  the package, and drop the SDK build from `prepare.sh`, once the SDK is published.
