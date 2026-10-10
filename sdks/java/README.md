# sagat-octopus-orchestrator-sdk — Java

The Java SDK of the Sagat Octopus Orchestrator: a **client** for runs and events, a **worker host** for
activities, compensations and reconciliations, and **builders** of the workflow document. One artifact.

The SDK keeps no journal of its own. The platform owns every state, every retry decision and every timer; the
SDK speaks its REST API and its worker protocol (gRPC) and reports faithfully. It behaves like the TypeScript,
Go, Python and C# SDKs because all five are held to the same conformance suite
(`tests/contract/sdk-conformance/`); the behaviour is written down in [`../SPEC.md`](../SPEC.md).

## Install

Coordinates: `dev.sagatoctopusorchestrator:sagat-octopus-orchestrator-sdk:0.1.0`, Java 17 or newer.

```xml
<dependency>
  <groupId>dev.sagatoctopusorchestrator</groupId>
  <artifactId>sagat-octopus-orchestrator-sdk</artifactId>
  <version>0.1.0</version>
</dependency>
```

The artifact is **not published to a repository yet** (no signing or release pipeline exists). Build and install
it into your local repository with `./mvnw install` from this directory. The library depends on Jackson
(`jackson-databind`), gRPC (`grpc-api`, `grpc-stub`, `grpc-protobuf`, `grpc-netty-shaded` at run time) and
`protobuf-java`; it logs through `System.Logger` and has no OpenTelemetry or logging-framework dependency.

## Start a run

```java
ClientConfig config = ClientConfig.builder()
        .apiUrl("https://api.example/v1")
        .namespace("payments")
        .tokenProvider(() -> System.getenv("SAGAT_TOKEN")) // called before every request
        .build();
try (SagatClient client = SagatClient.create(config)) {
    Run run = client.start("order-flow", Map.of("orderId", "o-1", "amount", "19.99"),
            StartOptions.create().idempotencyKey("order-o-1").correlationId("o-1"));
    Run finished = client.waitForCompletion(run.runId(), WaitOptions.create().timeout(Duration.ofMinutes(2)));
    client.listAll(ListOptions.create().status(RunStatus.RUNNING)).limit(10).forEach(r -> System.out.println(r.runId()));
    client.startAsync("order-flow", Map.of("orderId", "o-2"), null).thenAccept(r -> System.out.println(r.runId()));
} catch (SagatException e) {
    System.err.println(e.code() + " " + e.remediation() + " trace=" + e.traceId());
}
```

`start` takes a workflow name or id. Every call has a blocking form and an `...Async` form returning a
`CompletableFuture` (the asynchronous form runs the blocking one on an executor, so cancelling the future
interrupts a call that is waiting; the executor is the client's own, or yours through
`ClientConfig.Builder.asyncExecutor`). `listAll` is a lazy `Stream` that reads a page when the previous one is
consumed, in the order the server gave (newest first).

The SDK propagates the W3C `traceparent` (the one you pass, else the ambient context you expose through
`ClientConfig.Builder.traceContext(...)`, for example from OpenTelemetry), the correlation id and the deadline, and
retries **only when that is safe**: a `GET`, or a `POST` that carries an idempotency key (8 to 160 visible ASCII
characters) or an event id (the SDK generates one per call and reuses it for every retry of that call), and only
after `TRANSPORT`, `UNAVAILABLE`, `RATE_LIMITED` or `INTERNAL` errors (`SagatException.isRetriable()`); never a `4xx`
other than `429`, whatever the body claims. A `501`, or a `404` that does not carry the platform's error envelope, is
`NOT_FOUND` with the reason `UNSUPPORTED_BY_SERVER` (a newer SDK talking to an older server).

Every failure is a `SagatException` with `code()`, `httpStatus()` / `grpcStatus()`, `retryable()`, `traceId()`,
`reason()`, `retryAfterMs()` and a `remediation()` sentence from the shared catalog. It never repeats a payload, a
token or a secret. Answers are typed values that also keep the server's JSON (`raw()`), so a field added by a newer
server is reachable.

JSON values are `JsonNode`s and never go through `double`: an integer stays an integer (`600000`, `2^63-1`), a
decimal keeps its digits and `"19.99"` (a string) stays a string. Instants are `java.time.Instant`, always UTC; the
SDK never formats or parses one in the zone of the process.

## Serve activities

```java
WorkerConfig config = WorkerConfig.builder()
        .grpcUrl("https://worker.example:443")
        .namespace("payments")
        .tokenProvider(() -> System.getenv("SAGAT_WORKER_TOKEN"))
        .workerName("payments-worker-1")
        .maxInflight(8)
        .build();
WorkerHost host = new WorkerHost(config);
host.registerActivity("payments.charge", Order.class, (order, ctx) -> {
    // ctx.idempotencyKey() is stable across attempts: send it to the provider of the external effect.
    if (ctx.cancellation().isCancelled()) {
        throw ActivityFailure.permanent("CANCELLED", "the run was cancelled");
    }
    return Map.of("chargeId", "ch-" + ctx.idempotencyKey());
});
host.registerCompensation("payments.refund", (input, ctx) -> Map.of("refunded", ctx.targetNode()));
host.registerReconciler("payments.lookup", (input, ctx) -> Verdict.notApplied());
host.start(); // returns once registered
Runtime.getRuntime().addShutdownHook(new Thread(() -> host.stop(Duration.ofSeconds(30))));
```

The host registers the activities it can run, negotiates the protocol version (`host.protocolVersion()`), polls
without ever busy-polling (an empty answer is always followed by a wait), heartbeats every lease, cancels an
activity through `ctx.cancellation()` (reason `run-cancelled`, `lease-lost` or `shutdown`) and reports each result
with a key that is the same on every retry. The handler's input is bound to the type you register
(`Order.class`; a `JsonNode` when you give none; unknown fields are ignored, a value that does not fit is reported
as `INPUT_INVALID`, not retried). A handler fails on purpose with `ActivityFailure.retryable(code, message)`,
`ActivityFailure.permanent(...)` or `ActivityFailure.outcomeUnknown(...)`; any other `Throwable` is reported as
`ACTIVITY_ERROR` with only its type name, never its text. Return `null` for JSON `null`, or
`ActivityHandler.NO_RESULT` for no value at all.

The host is the work to wait for: its polling and task threads are not daemons, so a process that started a host
stays alive until `stop(Duration)` / `close()`. The host never interrupts an activity's thread; the activity
decides what to do with its cancellation signal (`await(Duration)` is the cancellable sleep). What the end of a
cancelled activity means depends on why it was cancelled: `run-cancelled` and a cancellation error is reported as
`CANCELLED` (not retried), `lease-lost` reports nothing, and `shutdown` with a cancellation error reports nothing
either, so the lease lapses and the step is leased again (a deploy does not fail steps for good). The heartbeat runs
until the report is made, not only until the handler returns; its interval is a third of what is left on the lease (its
expiry minus the clock of the host when the task is taken), between 0.5 s and 10 s, counted on the monotonic clock. A
host whose clock is not synchronized sets `WorkerConfig.Builder.heartbeatInterval(Duration)`. `ctx.secrets()`
resolves the step's secrets once per attempt, on first use, and overwrites them when the handler returns.

Platform guarantees stay at-least-once for external effects: the SDK gives you the idempotency key and the
reconciliation hook, it cannot make a provider exactly-once.

## Write a workflow

```java
ObjectNode document = WorkflowBuilder.named("order-flow")
        .description("charges, then ships")
        .step("validate", "order.validate", StepOptions.create().timeoutMs(5_000))
        .saga("settlement", SagaOptions.create().cancelPolicy(SagaOptions.CancelPolicy.COMPENSATE_COMPLETED),
                Flow.create()
                        .step("charge", "payments.charge", StepOptions.create()
                                .retry(RetryOptions.create().maxAttempts(3).initialDelayMs(100).strategy(RetryOptions.Strategy.EXPONENTIAL))
                                .compensateWith("payments.refund")
                                .reconcileWith("payments.lookup"))
                        .step("ship", "shipping.ship"))
        .waitForEvent("delivered", WaitForEventOptions.of("shipment.delivered", "input.orderId", 86_400_000).onTimeout("done"))
        .end("done")
        .build();
```

The builders (`step`, `sleep`/`sleepMs`, `waitForEvent`, `invokeChild`, `condition`, `parallel`, `saga`, `end`) have no
side effects and refuse what the server would refuse and they can see (a duplicate id, a dangling reference, a
saga without a body), as `VALIDATION_FAILED`. `build()` returns the document; `toJson()` its canonical JSON (RFC 8785: no whitespace, keys sorted by UTF-16 code
unit, numbers as ECMAScript prints them, so `1.0` is `1`), byte for byte what the other four SDKs write for the same
program. Canonical text refuses an integer beyond 2^53 - 1 (send it as a string); the text sent on the wire does not
round anything.

The snippets above are compiled by the build (`sdk/src/test/java/.../ReadmeExamples.java`).

## Transport hooks and TLS

* REST goes through `HttpTransport` (default `JdkHttpTransport`, `java.net.http.HttpClient`, HTTP/1.1). Wrap it for a
  proxy or metrics layer: `ClientConfig.Builder.transport(...)`. The conformance runner wraps it to inject faults.
* `ClientConfig.Builder.timeout` (30 s) is the deadline of one REST call and covers the whole exchange, body included
  (the JDK's own request timeout stops at the response headers, so the default transport enforces the rest itself);
  `WorkerConfig.Builder.timeout` is the deadline of one unary RPC.
* **A redirect is never followed**: the request carries a bearer token, the namespace and an idempotency key, and a
  `Location` can point anywhere. A `3xx` is raised as `PROTOCOL` ("the server redirected the call"; the `Location` is not
  repeated) and is not repeated. The default transport is built with `followRedirects(NEVER)`;
  `JdkHttpTransport.using(HttpClient)` takes a client you built and refuses one that follows redirects; a transport you
  implement yourself must not follow them either (the SDK cannot check that).
* gRPC accepts `io.grpc.ClientInterceptor`s: `WorkerConfig.Builder.interceptor(...)`.
* `TlsOptions` configures TLS for both channels (see below). Plain `http://` outside loopback is accepted and logs
  one warning when the client or the host is created (it names only the host); `https://` logs none.
* Logs go to `System.Logger` (`dev.sagatoctopusorchestrator.sdk`, `...sdk.worker`) and never contain a payload, a
  token, a lease token or a secret. `WorkerConfig.Builder.onEvent(...)` receives `WorkerEvent`s (tasks taken,
  completed with their duration, failed, stale, heartbeats, polls, reconnects) for metrics.

## TLS and mTLS

Options (`TlsOptions`, given to `ClientConfig.Builder.tls(...)` and `WorkerConfig.Builder.tls(...)`):

* `TlsOptions.systemRoots()`, or no `tls(...)` at all: the JVM's default trust roots.
* `TlsOptions.trusting(pem)` / `trusting(path)`: exactly the certificates in that PEM are trusted (a private CA, or the
  server's own certificate).
* `.withClientCertificate(certificatePem, pkcs8KeyPem)` (or the two files): the client certificate chain and its
  **PKCS#8** key (`BEGIN PRIVATE KEY`; RSA, EC or Ed25519) for mTLS. A PKCS#1 `BEGIN RSA PRIVATE KEY` is refused with a
  message that says how to convert it (`openssl pkcs8 -topk8 -nocrypt`).
* `tls.sslContext()` feeds `java.net.http.HttpClient` (REST); `tls.channelCredentials()` feeds the gRPC channel.
* Not offered: encrypted keys, PKCS#12/JKS stores, protocol or cipher selection (the JVM's defaults apply),
  revocation checking, pinning, a server name other than the URL's host (`tls.serverName`). There is **no option that
  turns verification off**: no trust-all `TrustManager`, no disabled host-name verification (a test reads the
  sources, scripts and poms of the SDK and its runner and fails on any of them).

Tested, on both channels, against in-process TLS servers (HTTPS for REST, a Netty TLS gRPC server for the worker) with
throw-away EC identities made by `keytool` at test time for `127.0.0.1`:

| Case | REST | gRPC |
|---|---|---|
| the configured CA makes a TLS server work (no client certificate is sent) | tested | tested |
| mTLS: the server sees the client's own certificate | tested | tested |
| a server that requires a client certificate refuses a client without one (`TRANSPORT`) | tested | tested |
| a server certificate that is not trusted fails as `TRANSPORT`: another CA configured, no TLS options, `systemRoots()` | tested | tested |
| a trusted certificate that names another host than the one reached is refused (host-name verification) | tested | tested |
| `https://` logs no plain-text warning, and no log line holds the token | tested | tested |
| a `3xx` is never followed and is `PROTOCOL` (a second server sees no request) | tested | not applicable |
| malformed PEM, PKCS#1 key: `VALIDATION_FAILED`, the key is not echoed | tested | tested |

Not tested: TLS 1.2-only peers, client certificate chains longer than one certificate, RSA and Ed25519 client keys
(only EC P-256 was generated), certificate expiry, a corporate proxy.

## Versions

**SemVer.** `0.x` is pre-release: a minor may change the public API and says so in the changelog. From `1.0` the
public API follows SemVer. The SDK's version is independent of the platform's. The **worker protocol** has its own
version, negotiated at registration: this SDK offers `1.2`; a server that speaks an older minor serves it at that
minor, a server of another major refuses with `UNSUPPORTED_PROTOCOL` (ADR-0015). Package metadata: group
`dev.sagatoctopusorchestrator`, artifact `sagat-octopus-orchestrator-sdk`, version `0.1.0`, licence `UNLICENSED` (the
repository has not declared one; an owner decision recorded in `specs/006-sdk-multilanguage/research.md`),
repository `https://github.com/ortisun/sagat-octopus-orchestrator`, JPMS automatic module name
`dev.sagatoctopusorchestrator.sdk`.

**Runtime matrix** — what was actually run, and what was not:

| Runtime | Status |
|---|---|
| JDK 23.0.2 (Oracle OpenJDK, macOS arm64) | **tested**: `./mvnw verify` (125 unit tests) and the conformance suite (13 scenarios) |
| JDK 26.0.2.1 (Homebrew OpenJDK, macOS arm64) | **tested**: the 125 unit tests only (not the conformance suite). The JVM prints warnings from `protobuf-java` 3.25 (`sun.misc.Unsafe`) and Netty (`System::loadLibrary`) on 24+ |
| Java 17 and 21 | **declared (`--release 17`), not run here**. The library is compiled with `--release 17`, so it uses only Java 17 APIs, and every dependency's class files are Java 8 level (checked on the class files of the run-time classpath); no JDK 17 or 21 runtime was available |
| Linux, Windows | not run. The `protoc` artifact is chosen by `os-maven-plugin` for the build machine |
| Android, GraalVM native image | not supported / not tried |

The Maven Wrapper in this directory is the Unix script only (`mvnw`, `distributionType=only-script`); there is no
`mvnw.cmd`. It pins Apache Maven 3.9.16 with its SHA-256.

Pinned dependencies (each at least two weeks old when chosen): gRPC 1.83.1, protobuf-java and `protoc` 3.25.9 (the
version `grpc-protobuf` 1.83.1 depends on), Jackson 2.22.2, JUnit Jupiter 5.14.4, `org.xolstice` protobuf-maven-plugin
0.6.1 with os-maven-plugin 1.7.1, and the Maven plugins listed in `pom.xml`.

## Develop

```text
./mvnw -q verify                      # compiles (the stubs are generated from sdk/src/main/proto/worker.proto), then runs the 125 unit tests
scripts/sync-shared.sh                # copies worker.proto and error-catalog.json from the repository; SharedContractsTest fails when behind
conformance/prepare.sh                # builds the runner (target/classpath.txt); conformance/run.sh build|run ... is what the harness calls
cargo test -p sagat-tests --test sdk_conformance   # the conformance suite (needs Docker), from the repository root
```

Layout: `sdk/` is the library (`dev.sagatoctopusorchestrator.sdk` for the client, errors and JSON,
`...sdk.worker` for the host, `...sdk.workflow` for the builders); `conformance/` is the runner, built on the public
API of the library only. The unit tests run offline against an in-process HTTP server and an in-process gRPC
`WorkerService`, with the test JVM in the zone `Asia/Kolkata` so that any use of local time shows. The TLS tests
generate throw-away identities with the JDK's `keytool`; no private key is kept in the repository.
