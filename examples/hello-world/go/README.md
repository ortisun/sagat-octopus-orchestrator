# hello-world in Go

The two-step workflow of `templates/hello-world` (`greeting.compose`, then `greeting.deliver`) served
and started with the [Go SDK](../../../sdks/go/README.md): one program that is the worker and the
client.

## Run it

You need a platform (`docker compose up`, see [the quickstart](../../../docs/quickstarts/README.md)), the
`sagat` CLI, and Go 1.25 or newer.

```sh
sagat new hello-world                      # the workflow (skip it if you have it)
sagat workflow publish hello-world/workflow.json
sagat worker bind --workflow hello-world/workflow.json     # let your identity serve its two activities

export SAGAT_API_URL=http://127.0.0.1:8080/v1 SAGAT_GRPC_URL=http://127.0.0.1:19000
export SAGAT_NAMESPACE=payments SAGAT_TOKEN=<the token you gave sagat>
./run.sh            # go run . [NAME]
```

It prints, in every language:

```text
serving greeting.compose and greeting.deliver at protocol 1.2
run 01a12b51-158b-7147-b9b3-847882a5c939 started
run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

and `sagat workflow inspect <run>` shows the journal: the same one whatever the language.

## What to change

- The activities are the two `RegisterActivity` calls. What a step receives is the input of the run
  (`input`) and the outputs of the steps before it (`steps.<id>.output`); what it returns is its output.
- A real effect (a payment, an e-mail) sends `worker.TaskFrom(ctx).IdempotencyKey` to its provider: it is
  the same for every attempt of the step, so a repeat does not do it twice.
- `go.mod` points at the SDK in this checkout (`replace`); remove that line once the SDK has a tag.
