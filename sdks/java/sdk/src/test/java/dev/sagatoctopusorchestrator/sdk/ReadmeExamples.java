package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import dev.sagatoctopusorchestrator.sdk.worker.ActivityFailure;
import dev.sagatoctopusorchestrator.sdk.worker.Verdict;
import dev.sagatoctopusorchestrator.sdk.worker.WorkerConfig;
import dev.sagatoctopusorchestrator.sdk.worker.WorkerHost;
import dev.sagatoctopusorchestrator.sdk.workflow.Flow;
import dev.sagatoctopusorchestrator.sdk.workflow.RetryOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.SagaOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.StepOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.WaitForEventOptions;
import dev.sagatoctopusorchestrator.sdk.workflow.WorkflowBuilder;
import java.time.Duration;
import java.util.Map;

/**
 * The examples of README.md, kept here so that the build compiles them: when an API changes and a snippet does not
 * follow, the build fails. Nothing here is run.
 */
@SuppressWarnings("unused")
final class ReadmeExamples {
    private ReadmeExamples() {}

    static void startARun() {
        ClientConfig config = ClientConfig.builder()
                .apiUrl("https://api.example/v1")
                .namespace("payments")
                .tokenProvider(() -> System.getenv("SAGAT_TOKEN")) // called before every request
                .build();
        try (SagatClient client = SagatClient.create(config)) {
            Run run = client.start("order-flow", Map.of("orderId", "o-1", "amount", "19.99"),
                    StartOptions.create().idempotencyKey("order-o-1").correlationId("o-1"));
            Run finished = client.waitForCompletion(run.runId(), WaitOptions.create().timeout(Duration.ofMinutes(2)));
            System.out.println(finished.status());
            client.listAll(ListOptions.create().status(RunStatus.RUNNING)).limit(10).forEach(r -> System.out.println(r.runId()));
            client.startAsync("order-flow", Map.of("orderId", "o-2"), null).thenAccept(r -> System.out.println(r.runId()));
        } catch (SagatException e) {
            System.err.println(e.code() + " " + e.remediation() + " trace=" + e.traceId());
        }
    }

    static void registerAnActivity() {
        record Order(String orderId, String amount) {}

        WorkerConfig config = WorkerConfig.builder()
                .grpcUrl("https://worker.example:443")
                .namespace("payments")
                .tokenProvider(() -> System.getenv("SAGAT_WORKER_TOKEN"))
                .workerName("payments-worker-1")
                .maxInflight(8)
                .build();
        WorkerHost host = new WorkerHost(config);
        host.registerActivity("payments.charge", Order.class, (order, ctx) -> {
            ctx.logger().info("charging " + order.orderId() + " attempt " + ctx.attempt());
            if (ctx.cancellation().isCancelled()) {
                throw ActivityFailure.permanent("CANCELLED", "the run was cancelled");
            }
            // ctx.idempotencyKey() identifies the effect at the provider: send it, so a retry never charges twice.
            return Map.of("chargeId", "ch-" + ctx.idempotencyKey());
        });
        host.registerCompensation("payments.refund", (input, ctx) -> Map.of("refunded", ctx.targetNode()));
        host.registerReconciler("payments.lookup", (input, ctx) -> Verdict.notApplied());
        host.start(); // returns once registered; the host keeps the process alive until stop()
        Runtime.getRuntime().addShutdownHook(new Thread(() -> host.stop(Duration.ofSeconds(30))));
    }

    static JsonNode buildAWorkflow() {
        return WorkflowBuilder.named("order-flow")
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
                .build(); // WorkflowBuilder.toJson() is the canonical text: keys sorted, no whitespace
    }
}
