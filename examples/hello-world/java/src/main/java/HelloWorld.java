// The hello-world workflow served and started with the Java SDK: a worker that answers the two
// activities of the workflow (templates/hello-world), a run of it, and the end of the run.
//
//   SAGAT_SERVER=http://127.0.0.1:18080/v1 SAGAT_GRPC=http://127.0.0.1:19000 \
//   SAGAT_NAMESPACE=payments SAGAT_TOKEN=... ./run.sh [NAME]

import dev.sagatoctopusorchestrator.sdk.ClientConfig;
import dev.sagatoctopusorchestrator.sdk.Run;
import dev.sagatoctopusorchestrator.sdk.RunStatus;
import dev.sagatoctopusorchestrator.sdk.SagatClient;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import dev.sagatoctopusorchestrator.sdk.WaitOptions;
import dev.sagatoctopusorchestrator.sdk.worker.WorkerConfig;
import dev.sagatoctopusorchestrator.sdk.worker.WorkerHost;
import java.time.Duration;
import java.util.Map;

public final class HelloWorld {
    // What a step receives is the input of the run, bound by the SDK to the type you register (fields it does
    // not name are ignored); the outputs of the steps that ran before it are in the context: ctx.steps().
    record Input(String name) {}

    record Greeting(String greeting) {}

    record Delivery(boolean delivered, String greeting) {}

    public static void main(String[] args) {
        int exit;
        try {
            exit = run(args);
        } catch (SagatException e) {
            // The code, the trace and what to do about it; never the payload or the token.
            System.err.println(e.code() + " (trace " + e.traceId() + "): " + e.remediation());
            exit = 1;
        }
        // The host's threads are not daemons: exit explicitly, once everything is stopped.
        System.exit(exit);
    }

    private static int run(String[] args) {
        String apiUrl = setting("SAGAT_SERVER");
        String grpcUrl = setting("SAGAT_GRPC");
        String namespace = setting("SAGAT_NAMESPACE");
        String token = setting("SAGAT_TOKEN");
        String name = args.length > 0 ? args[0] : "Ada";

        // The worker: it takes the tasks of the two activities and answers them.
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
            System.out.println("serving greeting.compose and greeting.deliver at protocol " + host.protocolVersion());

            // The client: start the run and wait for it.
            Run started = api.start("hello-world", Map.of("name", name));
            System.out.println("run " + started.runId() + " started");
            Run done = api.waitForCompletion(started.runId(), WaitOptions.create().timeout(Duration.ofSeconds(60)));
            System.out.println("run " + done.runId() + " " + done.status());
            if (done.status() != RunStatus.SUCCEEDED) {
                System.err.println("the run ended " + done.status());
                return 1;
            }
            return 0;
        } finally {
            host.stop(Duration.ofSeconds(10));
        }
    }

    private static String setting(String name) {
        String value = System.getenv(name);
        if (value == null || value.isEmpty()) {
            System.err.println(name + " is not set (see the README)");
            System.exit(2);
        }
        return value;
    }
}
