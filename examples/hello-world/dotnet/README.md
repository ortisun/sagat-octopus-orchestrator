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
./run.sh            # executes the Release build; dotnet run [-- NAME] builds and runs in one step
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
  for every attempt of the step; the provider must deduplicate that key to prevent duplicate effects.
- `HelloWorld.csproj` points at the SDK in this checkout (`ProjectReference`); once the SDK is published,
  replace that line with `<PackageReference Include="SagatOctopusOrchestrator.Sdk" Version="0.2.0" />`.

## Interface

- Top-level command execution uses local `RunAsync` and internal input/output records. [examples/hello-world/dotnet/Program.cs:RunAsync]

## Invariants

- The four connection settings are mandatory; missing settings exit 2. [examples/hello-world/dotnet/Program.cs:Setting]
- Registers compose/deliver activities and reads the prior compose output; the name defaults to Ada. [examples/hello-world/dotnet/Program.cs:RunAsync]
- The command waits at most 60 seconds and succeeds only with `RunStatuses.Succeeded`. [examples/hello-world/dotnet/Program.cs:RunAsync]
- The executable targets `net10.0` and references the checkout SDK with nullable types and invariant globalization enabled. [examples/hello-world/dotnet/HelloWorld.csproj:PropertyGroup]

## Side Effects

- Writes Release output with telemetry/first-run behavior disabled. [examples/hello-world/dotnet/prepare.sh:dotnet build]
- Launch requires the built DLL, then replaces the shell with `dotnet`; a missing DLL exits 1. [examples/hello-world/dotnet/run.sh:exec]
- Starts/stops a worker, starts/waits for a workflow, handles Ctrl-C, and prints diagnostics. [examples/hello-world/dotnet/Program.cs:RunAsync]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
