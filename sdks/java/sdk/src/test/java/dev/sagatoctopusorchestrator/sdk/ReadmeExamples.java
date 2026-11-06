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
import java.time.Instant;
import java.util.List;
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
            StartedRun run = client.start("order-flow", Map.of("orderId", "o-1", "amount", "19.99"),
                    StartOptions.create().idempotencyKey("order-o-1").correlationId("o-1"));
            Run finished = client.waitForCompletion(run.runId(), WaitOptions.create().timeout(Duration.ofMinutes(2)));
            System.out.println(finished.status());
            client.listAll(ListOptions.create().status(RunStatus.RUNNING)).limit(10).forEach(r -> System.out.println(r.runId()));
            client.startAsync("order-flow", Map.of("orderId", "o-2"), null).thenAccept(r -> System.out.println(r.runId()));
        } catch (SagatException e) {
            System.err.println(e.code() + " " + e.remediation() + " trace=" + e.traceId());
        }
    }

    static void schedulesBatchesAndReset() {
        ClientConfig config = ClientConfig.builder()
                .apiUrl("https://api.example/v1")
                .namespace("payments")
                .tokenProvider(() -> System.getenv("SAGAT_TOKEN"))
                .build();
        String namespaceId = "0190aaaa-0000-7000-8000-00000000aaaa";
        String workflowId = "0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee";
        try (SagatClient client = SagatClient.create(config)) {
            // A business identifier names the one open run of an order; a second start answers that run.
            StartedRun order = client.start("order-flow", Map.of("orderId", "o-1"), StartOptions.create()
                    .businessId("order-o-1")
                    .businessIdConflictPolicy(BusinessIdConflictPolicy.USE_EXISTING)
                    .startDelay(Duration.ofMinutes(5)));
            System.out.println(order.runId() + (order.started() ? " was started" : " already existed"));
            BusinessIdBinding binding = client.businessIds().get(namespaceId, "order-o-1", null);
            System.out.println(binding.currentRunId() + " open=" + binding.open());

            // A schedule: every day at 09:00 UTC. The instants are sent in UTC, whatever the zone of the process.
            Schedule schedule = client.schedules().create(namespaceId, ScheduleWrite.of("daily-report",
                    ScheduleWrite.Spec.create().cron("0 9 * * *").timezone("UTC").startAt(Instant.parse("2027-01-01T00:00:00Z")),
                    ScheduleWrite.Action.of(workflowId, Map.of("kind", "daily"))), null);
            client.schedules().pause(namespaceId, schedule.scheduleId(), "maintenance window", null);
            ScheduleDescription described = client.schedules().get(namespaceId, schedule.scheduleId(),
                    ScheduleGetOptions.create().nextCount(3).recentCount(5));
            System.out.println(described.counters() + " " + described.nextActions());
            client.schedules().delete(namespaceId, schedule.scheduleId(), schedule.revision(), null);

            // A batch job over many runs; the same key and body answer the first job.
            BatchJob job = client.batch().create(namespaceId, BatchRequest.of(
                    BatchSelector.runIds(List.of(order.runId())), "CANCEL", "clean up the held runs"),
                    CallOptions.create().idempotencyKey("cleanup-2027-01-01"));
            System.out.println(job.state() + " " + job.counts());

            // Signal a run by business identifier, starting it first when none is open; update it the same way.
            client.signalWithStart("order-flow", "order-o-1", Map.of("orderId", "o-1"), "payment.confirmed", Map.of("paid", "12.50"), null);
            UpdateWithStartResult updated = client.updateWithStart("order-flow", "order-o-1", Map.of("orderId", "o-1"),
                    "balance.set", "set-1", Map.of("balance", 25), UpdateWithStartOptions.create().expectedStateRevision(0L));
            System.out.println(updated.update().outcome());

            // Reset a run that ended to a point of its journal.
            Page<ResetPoint> points = client.resetPoints(order.runId(), null);
            ResetPlan plan = client.previewReset(order.runId(), points.items().get(0).sequence(), null, null);
            Reset reset = client.reset(order.runId(), ResetRequest.of(plan.sequence(), "run the steps again")
                    .acknowledgeReexecution(plan.requiresAcknowledgement()), null);
            System.out.println(reset.newRunId());
        } catch (SagatException e) {
            System.err.println(e.code() + " " + e.reason().orElse("") + " trace=" + e.traceId());
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
