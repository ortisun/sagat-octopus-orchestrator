# hello-world in C#

The two-step workflow of `templates/hello-world` (`greeting.compose`, then `greeting.deliver`) served
and started with the [.NET SDK](../../../sdks/dotnet/README.md): one program that is the worker and the
client.

## Run it

You need the local stack running (see [the quickstart](../../../docs/quickstarts/README.md)), the
`sagat` CLI, and the .NET SDK 10 or newer.

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)      # from the local stack's development identity provider

sagat new hello-world                      # the workflow (skip it if you have it)
sagat workflow publish hello-world/workflow.json
sagat worker bind --workflow hello-world/workflow.json     # let your identity serve its two activities

./prepare.sh        # dotnet build -c Release (once, and after a change)
./run.sh            # runs the build; dotnet run [-- NAME] builds and runs in one step
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

- The activities are the two `RegisterActivity` calls. What a handler receives is the input of the run
  (its first argument, here `Who`) and, in `context.Steps`, the outputs of the steps before it
  (`context.Steps["compose"]`); what it returns is its output, written as JSON in camel case.
- A real effect (a payment, an e-mail) sends `context.IdempotencyKey` to its provider: it is the same
  for every attempt of the step, so a repeat does not do it twice.
- `HelloWorld.csproj` points at the SDK in this checkout (`ProjectReference`); once the SDK is published,
  replace that line with `<PackageReference Include="SagatOctopusOrchestrator.Sdk" Version="0.2.0" />`.
