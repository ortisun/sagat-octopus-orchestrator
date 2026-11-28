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

## Workflow as code

**Not offered yet.** This section shows the API that [design 044](../designs/044-workflow-as-code.md) specifies; the Java
SDK offers it once its runtime, Task 044-18 of that design, lands. None of it runs today. The calls are those of
[`sdks/SPEC.md`](../../sdks/SPEC.md) section 12 (the new artifact `sagat-workflows`, the same names as TypeScript,
durations in the language's idiom); where the design leaves a name or a shape to the runtime's task (the context type,
registering the function on the host, the replayer's entry point), the code below is a sketch that Task 044-18 settles.
An operator also records the requirements `code-workflows-v1` and, because this workflow declares a signal,
`signal-inbox-v1` ([instance-registry runbook](../runbooks/instance-registry.md)).

Instead of a graph, the workflow is a method. It reserves the stock, charges the card, sleeps one day and sends a
reminder unless a `paid` signal arrived meanwhile (journey 1 of the design):

```java
static OrderResult order(WorkflowContext ctx, OrderInput input) {
    AtomicBoolean paid = new AtomicBoolean(false);
    ctx.onSignal("paid", payload -> paid.set(true));
    ctx.activity("inventory.reserve", input, Reservation.class);
    ctx.activity("payments.charge", input, Charge.class);
    ctx.sleep(Duration.ofDays(1));
    if (!paid.get()) {
        ctx.activity("customers.remind", input, Reminder.class);
    }
    return new OrderResult(input.orderId(), paid.get());
}
```

Each `ctx` call is a command the platform records and executes; the activities run in an ordinary worker as above. The
method is run again from the start against the run's journal whenever a worker does not hold it in memory, so it must be
deterministic. The SDK runs each workflow on virtual threads under a single permit, and every SDK call checks that it is
made from the thread that holds the permit (another thread fails the workflow task with `NONDETERMINISM_RISK`, which
puts the run under review at once, Technical Plan 6.3, not after counted failures). Nothing
checks the rest of your code before it runs, so keep to the SDK: no `System.currentTimeMillis`, `Random`,
`UUID.randomUUID`, threads, executors or `CompletableFuture`'s pool, and no iteration over a `HashSet` of objects that
hash by identity (enums included); use `ctx.now()`, `ctx.random()` and `ctx.uuid()`. The workflow artifact needs Java 21
(the core SDK stays on Java 17) and a default time zone of UTC and default locale `Locale.ROOT` for the workflow worker.

An activity failure the method does not catch fails the workflow task, not the run: the task is retried after 1, 2, 4
and 8 s, and the fifth consecutive failure puts the run under review. To fail the run on purpose, throw the SDK's
`WorkflowFailure` with a code (Requirement 044-009).

**One jar for workflow code, another for activities.** The code digest of a Java workflow hashes every jar and class
directory of the worker's class path, so a change of an activity on that class path makes a new digest, and runs open
on the old one move to it only by a verified move. Keep the workflow methods in their own worker jar
(`order-workflows.jar`) and the activities in another (`order-activities.jar`).

**Write the manifest and publish the version.** The workflow worker prints its manifest:

```sh
java -Duser.timezone=UTC -Duser.language= -Duser.country= -jar order-workflows.jar --sagat-manifest > manifest.json
```

```java
String manifest = Files.readString(Path.of("manifest.json"));
CodeEnvelope envelope = CodeEnvelope.activities("inventory.reserve", "payments.charge", "customers.remind");
CodeDeclarations declarations = CodeDeclarations.signals(new SignalDeclaration("paid", true, 10,
        """
        {"type": "object", "properties": {}, "additionalProperties": false}"""));
api.definitions().publishCode(manifest, envelope, declarations);
```

`publishCode` assembles the code document from the manifest, the envelope and the declarations, saves it as the
workflow's draft and publishes it, making it active (Decision 044-238). The envelope lists what the method's commands
may use; a command outside it puts the run under review. The declarations hold the rest of the code document of
Technical Plan 3.1, here the `paid` signal, buffered, at most 10 pending (`sagat workflow publish-code` takes them with
`--declarations <file>`).

**Serve it.** The identity of the workflow worker needs a binding that lists the workflow type `order` (`workflowTypes`
on the worker binding; setting it needs `definition:publish` as well as `worker:bind`). In `order-workflows.jar`, started
with the same `-Duser.timezone=UTC -Duser.language= -Duser.country=`:

```java
WorkerHost workflows = new WorkerHost(WorkerConfig.builder()
        .grpcUrl(grpcUrl).namespace(namespace).token(token).workerName("order-workflows").build());
workflows.registerWorkflow("order", OrderFlow::order); // the registration's name is fixed by Task 044-18
workflows.start();
```

Deploy a new workflow jar beside the old one: runs stay on their current code version's digest (the start version's
until a move), so keep the old worker running until the version list shows no run left on it, or move them.

**Test it with the replayer.** Export the history of a run (`api.runs().exportHistory(runId)`, or
`sagat workflow history export --workflow order --last 64 --dir histories/` once the command ships; both need
`history:export`), keep it with your tests, and replay it against the current code in JUnit (run the tests with the same
time zone and locale as the workers, through surefire's `argLine`):

```java
@Test
void theNewCodeReplaysTheRecordedOrderRuns() throws IOException {
    try (BufferedReader history = Files.newBufferedReader(Path.of("histories/order-1.jsonl"))) {
        ReplayVerdict verdict = Replayer.replay(history, "order", OrderFlow::order); // named by Task 044-18
        assertTrue(verdict.matched());
    }
}
```

A mismatch names the first command that differs; a history of an export format other than 1 is refused, and a replay
that reaches a value the export redacted stops with `INCONCLUSIVE_REDACTED` instead of a false mismatch.

## Where to go next

- [The SDK README](../../sdks/java/README.md): the client, the worker host, the builders of the workflow document, the errors.
- [`sdks/SPEC.md`](../../sdks/SPEC.md): what the six SDKs do the same way, and the conformance suite that holds them to it.
- [saga.md](saga.md): a payment saga, with compensation and reconciliation.
