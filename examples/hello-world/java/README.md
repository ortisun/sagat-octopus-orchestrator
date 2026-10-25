# hello-world in Java

The two-step workflow of `templates/hello-world` (`greeting.compose`, then `greeting.deliver`) served
and started with the [Java SDK](../../../sdks/java/README.md): one program that is the worker and the
client.

## Run it

You need the local stack running (see [the quickstart](../../../docs/quickstarts/README.md)), the
`sagat` CLI, and a JDK 17 or newer.

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)      # from the local stack's development identity provider

sagat new hello-world                      # the workflow (skip it if you have it)
sagat workflow publish hello-world/workflow.json
sagat worker bind --workflow hello-world/workflow.json     # let your identity serve its two activities

./prepare.sh        # installs the SDK from this checkout into ~/.m2, then builds the example
./run.sh            # java -cp ... HelloWorld [NAME]
```

The SDK is not published to a repository yet, so `prepare.sh` runs `./mvnw install` of
`sdks/java` (the library only, tests skipped) into your local Maven repository, and then builds the
example with the same wrapper (there is no need for a system `mvn`). The first run downloads the
dependencies. `prepare.sh` is safe to run again.

It prints, in every language:

```text
serving greeting.compose and greeting.deliver at protocol 1.2
run 01a12b51-158b-7147-b9b3-847882a5c939 started
delivering Hello, Ada!
run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

and `sagat workflow inspect <run>` shows the journal: the same one whatever the language.

The SDK also logs on `System.Logger` (the JDK's default: standard error), for example
`worker registered worker=hello-world-java protocol=1.2`; it never logs a payload or the token.

## What to change

- The activities are the two `registerActivity` calls. What a step receives is the input of the run
  (here `Input`, bound by the SDK; the fields it does not name are ignored) and, in `ctx.steps()`, the outputs
  of the steps before it by step id; what it returns is its output.
- A real effect (a payment, an e-mail) sends `ctx.idempotencyKey()` to its provider: it is the same for
  every attempt of the step, so a repeat does not do it twice.
- The dependency in `pom.xml` (`dev.sagatoctopusorchestrator:sagat-octopus-orchestrator-sdk:0.2.0`) resolves
  to the copy `prepare.sh` installed; remove the install step from `prepare.sh` once the SDK is published.
