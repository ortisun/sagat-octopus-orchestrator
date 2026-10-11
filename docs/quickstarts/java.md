# Java quickstart

The hello-world workflow ([`templates/hello-world`](../../templates/hello-world/workflow.json): `greeting.compose`, then
`greeting.deliver`) served and started with the Java SDK, in [one file](../../examples/hello-world/java/src/main/java/HelloWorld.java). You need a JDK 17 or newer, the
local stack and `sagat`; the [main quickstart](README.md) gets you there.

## Run it

```sh
export SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 SAGAT_NAMESPACE=payments
export SAGAT_TOKEN=$(sagat dev token)

sagat new hello-world
sagat workflow publish hello-world/workflow.json
sagat worker bind --workflow hello-world/workflow.json
```

then, in [`examples/hello-world/java`](../../examples/hello-world/java), follow its [README](../../examples/hello-world/java/README.md). It prints

```text
serving greeting.compose and greeting.deliver at protocol 1.2
run 01a12b51-158b-7147-b9b3-847882a5c939 started
delivering Hello, Ada!
run 01a12b51-158b-7147-b9b3-847882a5c939 SUCCEEDED
```

and `sagat workflow inspect <run>` shows the journal. The same program in each of the five languages makes the same run,
the same journal and the same outputs, byte for byte, which the end-to-end test of the examples checks.

The SDK is not in a Maven repository yet: `prepare.sh` runs `./mvnw install` of `sdks/java` (the library only, tests skipped) into your local
repository and then builds the example with the same wrapper, so no system Maven is needed. Once the artifact is published, drop that step.

## The worker

An activity receives the **input of the run** and, in the context, the **outputs of the steps that ran before it**:
`ctx.steps().get("compose")` is the output of the step with that id, as a `JsonNode`. It returns its output. It fails on purpose with a retryable or a permanent failure; anything else it
raises is reported as `ACTIVITY_ERROR`, retryable, with only the type of the error, never its text.

```java excerpt=examples/hello-world/java/src/main/java/HelloWorld.java
WorkerHost host = new WorkerHost(WorkerConfig.builder()
        .grpcUrl(grpcUrl)
        .namespace(namespace)
        .token(token)
        .workerName("hello-world-java")
        .build());
host.registerActivity("greeting.compose", Input.class, (input, ctx) -> new Greeting("Hello, " + input.name() + "!"));
host.registerActivity("greeting.deliver", (input, ctx) -> {
    // The output of the compose step, which ran before this one.
    String greeting = ctx.steps().get("compose").path("greeting").asText();
    // A real delivery is an external effect: send ctx.idempotencyKey() to the provider, which is
    // the same for every attempt of this step.
    System.out.println("delivering " + greeting);
    return new Delivery(true, greeting);
});
// Ctrl-C stops the host too, so that the task in flight is not abandoned.
Runtime.getRuntime().addShutdownHook(new Thread(() -> host.stop(Duration.ofSeconds(10))));

try (SagatClient api = SagatClient.create(ClientConfig.builder().apiUrl(apiUrl).namespace(namespace).token(token).build())) {
    host.start();
```

The platform delivers at least once, so an activity that makes an effect in the world (a payment, an e-mail) sends
`ctx.idempotencyKey()` to its provider: it is the same for every attempt of the step, so a repeat does not do it twice. The
platform cannot make the provider exactly-once; the key is how the provider can.

## The client

```java excerpt=examples/hello-world/java/src/main/java/HelloWorld.java
Run started = api.start("hello-world", Map.of("name", name));
System.out.println("run " + started.runId() + " started");
Run done = api.waitForCompletion(started.runId(), WaitOptions.create().timeout(Duration.ofSeconds(60)));
System.out.println("run " + done.runId() + " " + done.status());
```

Starting a run takes the name of a workflow of the namespace (or its id). Without an idempotency key the SDK makes one per
call and reuses it for every retry of that call, so a lost answer never starts a second run. The tenant is never
configured: it is the token's.

## Testing

- **A workflow, before your worker exists:** `sagat worker dev --mocks mocks.yaml --workflow workflow.json --bind` serves its
  activities from a file of canned answers and failures, on the real engine. Make `greeting.compose` fail once
  (`sequence: [{fail: {code: X, retryable: true}}, {output: ...}]`) and see the retry; make it `lose` the task and see the lease
  expire.
- **Your activities, against the platform:** the example is the harness. It is what the end-to-end test of this
  repository runs, with the platform it starts, and what you can run against your own stack.
- **Your activities, alone:** A handler is a lambda of `(input, ctx)`: keep the effect behind an interface and unit test the logic by calling the code the lambda calls.
The SDK ships no fake context yet.

## Where to go next

- [The SDK README](../../sdks/java/README.md): the client, the worker host, the builders of the workflow document, the errors.
- [`sdks/SPEC.md`](../../sdks/SPEC.md): what the five SDKs do the same way, and the conformance suite that holds them to it.
- [saga.md](saga.md): a payment saga, with compensation and reconciliation.
