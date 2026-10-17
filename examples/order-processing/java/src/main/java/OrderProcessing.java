import dev.sagatoctopusorchestrator.sdk.ClientConfig;
import dev.sagatoctopusorchestrator.sdk.Run;
import dev.sagatoctopusorchestrator.sdk.RunStatus;
import dev.sagatoctopusorchestrator.sdk.SagatClient;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import dev.sagatoctopusorchestrator.sdk.WaitOptions;
import dev.sagatoctopusorchestrator.sdk.worker.ActivityFailure;
import dev.sagatoctopusorchestrator.sdk.worker.WorkerConfig;
import dev.sagatoctopusorchestrator.sdk.worker.WorkerHost;
import java.time.Duration;

/** Serves and starts the routing, fulfillment, and reminder order examples. */
public final class OrderProcessing {
    record OrderInput(String orderId, int amount, boolean failNotify) {}

    record Validation(boolean accepted) {}

    record Reservation(boolean reserved, String orderId) {}

    record Quote(int shippingCents) {}

    record Notification(boolean notified, String orderId) {}

    private record Scenario(String name, String workflow) {}

    private OrderProcessing() {}

    public static void main(String[] args) {
        int exit;
        try {
            exit = run();
        } catch (SagatException e) {
            System.err.println(e.code() + " (trace " + e.traceId() + "): " + e.remediation());
            exit = 1;
        }
        System.exit(exit);
    }

    private static int run() {
        String apiUrl = requiredSetting("SAGAT_SERVER");
        String grpcUrl = requiredSetting("SAGAT_GRPC");
        String namespace = requiredSetting("SAGAT_NAMESPACE");
        String token = requiredSetting("SAGAT_TOKEN");
        Scenario scenario = selectedScenario();
        OrderInput input = new OrderInput(
                "demo-order-001",
                "1".equals(System.getenv("SAGAT_REJECT_ORDER")) ? -1 : 100,
                "1".equals(System.getenv("SAGAT_FAIL_NOTIFY")));

        WorkerHost host = new WorkerHost(WorkerConfig.builder()
                .grpcUrl(grpcUrl)
                .namespace(namespace)
                .token(token)
                .workerName("order-processing-java")
                .build());
        host.registerActivity("order.validate", OrderInput.class, (order, ctx) -> {
            System.out.println("activity order.validate");
            return new Validation(order.amount() > 0);
        });
        host.registerActivity("order.reserve", OrderInput.class, (order, ctx) -> {
            System.out.println("activity order.reserve");
            return new Reservation(true, order.orderId());
        });
        host.registerActivity("order.quote", OrderInput.class, (order, ctx) -> {
            System.out.println("activity order.quote");
            return new Quote(500);
        });
        host.registerActivity("order.notify", OrderInput.class, (order, ctx) -> {
            System.out.println("activity order.notify");
            // Predecessors differ by scenario. If a real notification needs one, use
            // ctx.steps().get(...) and handle null instead of assuming the output exists.
            if (order.failNotify()) {
                throw ActivityFailure.permanent(
                        "NOTIFICATION_REJECTED", "the simulated notification was rejected");
            }
            // A real provider call must use ctx.idempotencyKey() for deduplication and reconcile an
            // unknown outcome before retrying. This example has no external effect.
            return new Notification(true, order.orderId());
        });
        Runtime.getRuntime().addShutdownHook(new Thread(() -> host.stop(Duration.ofSeconds(10))));

        try (SagatClient api = SagatClient.create(ClientConfig.builder()
                .apiUrl(apiUrl)
                .namespace(namespace)
                .token(token)
                .build())) {
            host.start();
            Run started = api.start(scenario.workflow(), input);
            Run done = api.waitForCompletion(
                    started.runId(), WaitOptions.create().timeout(Duration.ofSeconds(60)));
            System.out.println("scenario " + scenario.name() + " run " + done.runId() + " " + done.status());
            return done.status() == RunStatus.SUCCEEDED ? 0 : 1;
        } finally {
            host.stop(Duration.ofSeconds(10));
        }
    }

    private static Scenario selectedScenario() {
        String name = System.getenv("SAGAT_SCENARIO");
        if (name == null || name.isEmpty()) {
            name = "routing";
        }
        return switch (name) {
            case "routing" -> new Scenario(name, "order-routing");
            case "fulfillment" -> new Scenario(name, "order-fulfillment");
            case "reminder" -> new Scenario(name, "order-reminder");
            default -> {
                System.err.println("SAGAT_SCENARIO must be routing, fulfillment, or reminder");
                System.exit(2);
                throw new AssertionError("unreachable");
            }
        };
    }

    private static String requiredSetting(String name) {
        String value = System.getenv(name);
        if (value == null || value.isEmpty()) {
            System.err.println(name + " is not set (see the README)");
            System.exit(2);
        }
        return value;
    }
}
