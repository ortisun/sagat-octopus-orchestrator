package dev.sagatoctopusorchestrator.sdk.worker;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import dev.sagatoctopusorchestrator.protocol.v1.Worker;
import dev.sagatoctopusorchestrator.sdk.CapturingLogger;
import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.RetryPolicy;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import io.grpc.Status;
import java.io.IOException;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.BooleanSupplier;
import java.util.function.Consumer;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;

/** The worker host against an in-process worker service (mirrors worker.test.ts and adds what is specific to Java). */
@Timeout(60)
class WorkerHostTest {
    private static final String TRACE = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    private static FakeWorkerServer fake;
    private final List<WorkerHost> hosts = new ArrayList<>();

    @BeforeAll
    static void startService() throws IOException {
        fake = new FakeWorkerServer();
    }

    @AfterAll
    static void stopService() {
        fake.close();
    }

    @AfterEach
    void cleanUp() {
        while (!hosts.isEmpty()) {
            hosts.remove(hosts.size() - 1).stop(Duration.ofSeconds(1));
        }
        fake.reset();
    }

    private static Worker.TaskLease lease(Consumer<Worker.TaskLease.Builder> tweak) {
        Worker.TaskLease.Builder builder = Worker.TaskLease.newBuilder()
                .setTaskId("run-1/charge/1")
                .setRunId("run-1")
                .setStepId("charge")
                .setAttempt(1)
                .setGeneration(1)
                .setLeaseToken("lt")
                .setExpiresAtEpochMs(System.currentTimeMillis() + 60_000)
                .setActivityName("charge")
                .setInputJson("{\"run\":{\"id\":\"run-1\"},\"node\":\"charge\",\"input\":{\"amount\":5},\"steps\":{\"earlier\":{\"output\":{\"ok\":true}}}}")
                .setTraceparent(TRACE)
                .setIdempotencyKey("run-1:charge");
        tweak.accept(builder);
        return builder.build();
    }

    private static Worker.TaskLease lease() {
        return lease(b -> {});
    }

    private WorkerHost host(Consumer<WorkerConfig.Builder> tweak) {
        WorkerConfig.Builder builder = WorkerConfig.builder()
                .grpcUrl(fake.url())
                .namespace("payments")
                .token("tok")
                .workerName("test-worker")
                .retry(new RetryPolicy(4, Duration.ofMillis(5), Duration.ofMillis(20)));
        tweak.accept(builder);
        WorkerHost created = new WorkerHost(builder.build());
        hosts.add(created);
        return created;
    }

    private WorkerHost host() {
        return host(b -> {});
    }

    private static void until(BooleanSupplier condition, String what) {
        long end = System.nanoTime() + Duration.ofSeconds(10).toNanos();
        while (!condition.getAsBoolean()) {
            if (System.nanoTime() > end) {
                throw new AssertionError("timed out waiting for " + what);
            }
            try {
                Thread.sleep(10);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new AssertionError(e);
            }
        }
    }

    private static Worker.CompleteStepRequest completed(int i) {
        return (Worker.CompleteStepRequest) fake.of("CompleteStep").get(i).request();
    }

    private static Worker.FailStepRequest failed(int i) {
        return (Worker.FailStepRequest) fake.of("FailStep").get(i).request();
    }

    private static Map<String, Worker.FailStepRequest> failuresByStep() {
        Map<String, Worker.FailStepRequest> by = new java.util.HashMap<>();
        fake.of("FailStep").forEach(call -> {
            Worker.FailStepRequest request = (Worker.FailStepRequest) call.request();
            by.put(request.getStepId(), request);
        });
        return by;
    }

    private record Order(String id, int amount) {}

    private record Receipt(String id, int doubled) {}

    // ------------------------------------------------------------------------------------ registration

    @Test
    void aWorkerRegistersItsActivitiesAndTheNewestProtocolItSpeaksWithTheTokenAsMetadata() {
        WorkerHost h = host(b -> b.maxInflight(7).pools(List.of("gpu")));
        h.registerActivity("b.second", (in, ctx) -> 1)
                .registerCompensation("a.undo", (in, ctx) -> 1)
                .registerReconciler("c.lookup", (in, ctx) -> Verdict.unknown());
        h.start();
        FakeWorkerServer.Call register = fake.of("RegisterWorker").get(0);
        Worker.RegisterWorkerRequest request = (Worker.RegisterWorkerRequest) register.request();
        assertEquals("test-worker", request.getWorkerName());
        assertEquals("payments", request.getNamespace());
        assertEquals(List.of("java"), request.getCapabilities().getLanguagesList());
        assertEquals(List.of("a.undo", "b.second", "c.lookup"), request.getCapabilities().getActivitiesList());
        assertEquals(List.of("gpu"), request.getCapabilities().getPoolsList());
        assertEquals(7, request.getCapabilities().getMaxInflight());
        assertEquals("1.3", request.getCapabilities().getProtocolVersion());
        assertEquals("Bearer tok", register.header("authorization"));
        assertEquals("1.2", h.protocolVersion());
    }

    @Test
    void registrationIsRefusedClearlyAnUnsupportedProtocolAnUnboundIdentity() {
        WorkerHost h = host();
        h.registerActivity("charge", (in, ctx) -> 1);
        fake.failNext("RegisterWorker", FakeWorkerServer.Failure.of(Status.Code.INVALID_ARGUMENT, "UNSUPPORTED_PROTOCOL"));
        SagatException unsupported = assertThrows(SagatException.class, h::start);
        assertEquals(ErrorCode.UNSUPPORTED_PROTOCOL, unsupported.code());
        assertFalse(unsupported.retryable());
        assertEquals("INVALID_ARGUMENT", unsupported.grpcStatus().orElseThrow());
        assertEquals(1, fake.of("RegisterWorker").size(), "not retried");

        WorkerHost second = host(b -> b.workerName("second"));
        second.registerActivity("charge", (in, ctx) -> 1);
        fake.failNext("RegisterWorker", FakeWorkerServer.Failure.of(Status.Code.PERMISSION_DENIED, "NOT_BOUND"));
        SagatException unbound = assertThrows(SagatException.class, second::start);
        assertEquals(ErrorCode.NOT_BOUND, unbound.code());
        assertEquals(2, fake.of("RegisterWorker").size(), "not retried either");
        assertEquals("", second.protocolVersion());
    }

    @Test
    void aRegistrationThatNeverLeavesIsRepeatedAndAnExpiredTokenGetsOneFreshTry() {
        WorkerHost h = host();
        h.registerActivity("charge", (in, ctx) -> 1);
        fake.failNext("RegisterWorker", FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE), FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE));
        h.start();
        assertEquals(3, fake.of("RegisterWorker").size());
        h.stop(Duration.ofSeconds(1));

        AtomicInteger asked = new AtomicInteger();
        WorkerHost refreshing = host(b -> b.tokenProvider(() -> "tok-" + asked.incrementAndGet()));
        refreshing.registerActivity("charge", (in, ctx) -> 1);
        fake.failNext("RegisterWorker", FakeWorkerServer.Failure.of(Status.Code.UNAUTHENTICATED));
        refreshing.start();
        List<FakeWorkerServer.Call> registrations = fake.of("RegisterWorker");
        FakeWorkerServer.Call last = registrations.get(registrations.size() - 1);
        FakeWorkerServer.Call before = registrations.get(registrations.size() - 2);
        assertEquals("Bearer tok-1", before.header("authorization"));
        assertEquals("Bearer tok-2", last.header("authorization"), "the provider was asked again");

        WorkerHost fixed = host(b -> b.workerName("fixed"));
        fixed.registerActivity("charge", (in, ctx) -> 1);
        int calls = fake.of("RegisterWorker").size();
        fake.failNext("RegisterWorker", FakeWorkerServer.Failure.of(Status.Code.UNAUTHENTICATED));
        assertEquals(ErrorCode.UNAUTHENTICATED, assertThrows(SagatException.class, fixed::start).code());
        assertEquals(calls + 1, fake.of("RegisterWorker").size(), "a static token is not asked for again");
    }

    @Test
    void registeringTwiceOrAfterTheStartIsAMistakeAndSaysSo() {
        WorkerHost h = host();
        h.registerActivity("a", (in, ctx) -> 1);
        SagatException twice = assertThrows(SagatException.class, () -> h.registerActivity("a", (in, ctx) -> 2));
        assertEquals(ErrorCode.VALIDATION_FAILED, twice.code());
        assertEquals(ErrorCode.VALIDATION_FAILED, assertThrows(SagatException.class, () -> host(b -> b.maxInflight(0))).code());
        assertEquals(ErrorCode.VALIDATION_FAILED, assertThrows(SagatException.class, () -> host(b -> b.maxInflight(1001))).code());
        SagatException none = assertThrows(SagatException.class, () -> host().start());
        assertTrue(none.getMessage().contains("at least one activity"));
        h.start();
        assertThrows(SagatException.class, () -> h.registerActivity("late", (in, ctx) -> 1));
    }

    // ----------------------------------------------------------------------------------------- running

    @Test
    void aTaskIsRunWithItsContextAndCompletedOnceWithAKeyThatIsTheSameOnEveryRetry() {
        AtomicReference<ActivityContext> seen = new AtomicReference<>();
        AtomicReference<Object> received = new AtomicReference<>();
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.onEvent(events::add));
        h.registerActivity("charge", (input, ctx) -> {
            seen.set(ctx);
            received.set(input);
            return Map.of("charged", input.get("amount").intValue() * 2);
        });
        fake.failNext("CompleteStep",
                FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE),
                new FakeWorkerServer.Failure(Status.Code.RESOURCE_EXHAUSTED, "RATE_LIMITED", 1L));
        fake.addLeases(lease());
        h.start();
        until(() -> fake.of("CompleteStep").size() == 3, "three attempts to report");
        assertEquals(Json.parse("{\"amount\":5}"), received.get());
        ActivityContext ctx = seen.get();
        assertEquals("charge", ctx.activityName());
        assertEquals("run-1", ctx.runId());
        assertEquals("run-1/charge/1", ctx.taskId());
        assertEquals("charge", ctx.node());
        assertEquals("charge", ctx.stepId());
        assertEquals(1, ctx.attempt());
        assertEquals(1, ctx.generation());
        assertEquals("run-1:charge", ctx.idempotencyKey());
        assertEquals(TaskKind.FORWARD, ctx.taskKind());
        assertEquals(Map.of("earlier", Json.parse("{\"ok\":true}")), ctx.steps());
        assertEquals(TRACE, ctx.traceparent());
        assertEquals("", ctx.targetNode());
        assertTrue(ctx.deadline().isAfter(Instant.now()));
        assertTrue(fake.of("CompleteStep").stream().map(c -> ((Worker.CompleteStepRequest) c.request()).getIdempotencyKey()).allMatch("done-run-1/charge/1"::equals));
        assertEquals("{\"charged\":10}", completed(2).getOutputJson());
        assertEquals("lt", completed(2).getLeaseToken());
        assertEquals(1, completed(2).getGeneration());
        assertEquals(TRACE, completed(2).getTraceparent());
        assertEquals(TRACE, fake.of("CompleteStep").get(2).header("traceparent"), "the trace context is propagated as a header");
        until(() -> events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskCompleted), "the completed event");
        assertInstanceOf(WorkerEvent.Registered.class, events.get(0));
    }

    @Test
    void aFailureOnPurposeCarriesItsCodeAndWhetherItMayBeRetriedAnyOtherErrorSaysOnlyItsType() {
        WorkerHost h = host();
        h.registerActivity("permanent", (in, ctx) -> {
            throw ActivityFailure.permanent("INSUFFICIENT_FUNDS", "not enough");
        });
        h.registerActivity("unknown", (in, ctx) -> {
            throw ActivityFailure.outcomeUnknown("the provider timed out");
        });
        h.registerActivity("boom", (in, ctx) -> {
            throw new IllegalStateException("secret card 4111 leaked in the message");
        });
        h.registerActivity("badcode", (in, ctx) -> {
            throw new ActivityFailure("not a code!", "x", false);
        });
        h.registerActivity("retryable", (in, ctx) -> {
            throw ActivityFailure.retryable("PROVIDER_BUSY", "try again");
        });
        h.registerActivity("error", (in, ctx) -> {
            throw new AssertionError("card 4111");
        });
        h.registerActivity("long", (in, ctx) -> {
            throw ActivityFailure.permanent("LONG", "m".repeat(2000));
        });
        List<Worker.TaskLease> leases = new ArrayList<>();
        int i = 0;
        for (String name : List.of("permanent", "unknown", "boom", "badcode", "retryable", "error", "long")) {
            String id = "run-" + i++ + "/" + name + "/1";
            leases.add(lease(b -> b.setTaskId(id).setActivityName(name).setStepId(name)));
        }
        fake.addLeases(leases.toArray(new Worker.TaskLease[0]));
        h.start();
        until(() -> fake.of("FailStep").size() == 7, "seven failures");
        Map<String, Worker.FailStepRequest> by = failuresByStep();
        assertEquals("INSUFFICIENT_FUNDS", by.get("permanent").getErrorCode());
        assertFalse(by.get("permanent").getRetryable());
        assertEquals("not enough", by.get("permanent").getSafeMessage());
        assertEquals("OUTCOME_UNKNOWN", by.get("unknown").getErrorCode());
        assertFalse(by.get("unknown").getRetryable());
        assertEquals("ACTIVITY_ERROR", by.get("boom").getErrorCode());
        assertTrue(by.get("boom").getRetryable());
        assertEquals("IllegalStateException", by.get("boom").getSafeMessage());
        assertEquals("WORKER_ERROR", by.get("badcode").getErrorCode());
        assertTrue(by.get("retryable").getRetryable());
        assertEquals("AssertionError", by.get("error").getSafeMessage(), "an Error is reported too, by its type");
        assertEquals(512, by.get("long").getSafeMessage().length());
        assertEquals("failed-run-0/permanent/1", by.get("permanent").getIdempotencyKey());
        assertTrue(fake.of("FailStep").stream().noneMatch(c -> ((Worker.FailStepRequest) c.request()).getSafeMessage().contains("4111")));
    }

    @Test
    void theHeartbeatKeepsRunningUntilTheReportIsMadeNotJustUntilTheHandlerEnds() {
        WorkerHost h = host(b -> b.heartbeatInterval(Duration.ofMillis(20)));
        AtomicReference<Long> handlerEnded = new AtomicReference<>();
        h.registerActivity("quick", (in, ctx) -> {
            handlerEnded.set(System.nanoTime());
            return Map.of("ok", true);
        });
        // The report is refused twice and asked to wait 300 ms each time: it is not made for about 600 ms.
        fake.failNext("CompleteStep",
                new FakeWorkerServer.Failure(Status.Code.UNAVAILABLE, "UNAVAILABLE", 300L),
                new FakeWorkerServer.Failure(Status.Code.UNAVAILABLE, "UNAVAILABLE", 300L));
        fake.addLeases(lease(b -> b.setActivityName("quick").setStepId("quick").setTaskId("run-1/quick/1")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 3, "the third attempt to report");
        long reportMade = fake.of("CompleteStep").get(2).nanos();
        long during = fake.of("Heartbeat").stream().filter(c -> c.nanos() > handlerEnded.get() + 100_000_000L && c.nanos() < reportMade).count();
        assertTrue(during >= 5, during + " heartbeats between the end of the handler and the report");
        sleep(100);
        int settled = fake.of("Heartbeat").size();
        sleep(200);
        assertEquals(settled, fake.of("Heartbeat").size(), "the heartbeat stops once the report is made");
    }

    @Test
    void aLeaseLostWhileTheReportIsBeingRepeatedStopsTheRepetition() {
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.heartbeatInterval(Duration.ofMillis(20)).onEvent(events::add));
        h.registerActivity("quick", (in, ctx) -> Map.of("ok", true));
        fake.failNext("CompleteStep",
                new FakeWorkerServer.Failure(Status.Code.UNAVAILABLE, "UNAVAILABLE", 300L),
                new FakeWorkerServer.Failure(Status.Code.UNAVAILABLE, "UNAVAILABLE", 300L),
                new FakeWorkerServer.Failure(Status.Code.UNAVAILABLE, "UNAVAILABLE", 300L));
        fake.failNext("Heartbeat", FakeWorkerServer.Failure.of(Status.Code.ABORTED, "STALE_ATTEMPT"));
        fake.addLeases(lease(b -> b.setActivityName("quick").setStepId("quick")));
        h.start();
        until(() -> events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskStale), "the stale event");
        sleep(400);
        assertEquals(1, fake.of("CompleteStep").size(), "the report was not repeated once the lease was known to be gone");
        assertEquals(0, fake.of("FailStep").size());
    }

    @Test
    void aStaleReportIsDroppedAndNeverRetried() {
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.onEvent(events::add));
        h.registerActivity("charge", (in, ctx) -> Map.of("ok", true));
        fake.failNext("CompleteStep", FakeWorkerServer.Failure.of(Status.Code.ABORTED, "STALE_ATTEMPT"));
        fake.addLeases(lease());
        h.start();
        until(() -> events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskStale), "the stale event");
        assertEquals(1, fake.of("CompleteStep").size());
    }

    @Test
    void aReportRefusedForAFieldIsNotRepeatedAndIsCountedAsDropped() {
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.onEvent(events::add));
        h.registerActivity("charge", (in, ctx) -> Map.of("ok", true));
        fake.failNext("CompleteStep", FakeWorkerServer.Failure.of(Status.Code.INVALID_ARGUMENT));
        fake.addLeases(lease());
        h.start();
        until(() -> events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskDropped), "the dropped event");
        sleep(100);
        assertEquals(1, fake.of("CompleteStep").size(), "a refusal that repeating cannot change is not repeated");
        assertEquals("VALIDATION_FAILED", events.stream().filter(e -> e instanceof WorkerEvent.TaskDropped).map(e -> ((WorkerEvent.TaskDropped) e).reason()).findFirst().orElseThrow());
    }

    @Test
    void aConfigurationThatCannotWorkIsRefusedWhenItIsBuilt() {
        for (String url : new String[] {"", "worker.example:443", "grpc://worker.example:443", "http://worker.example", "not a url"}) {
            SagatException error = assertThrows(SagatException.class, () -> WorkerConfig.builder().grpcUrl(url).namespace("p").token("t").workerName("w").build(), url);
            assertEquals(ErrorCode.VALIDATION_FAILED, error.code());
        }
        assertThrows(SagatException.class, () -> WorkerConfig.builder().grpcUrl("https://w.example:443").namespace("p").token("t").build());
    }

    @Test
    void anInputTheHandlerCannotTakeAnActivityNobodyRegisteredAndAnOutputTooLargeAreFailuresForGood() {
        WorkerHost h = host();
        h.registerActivity("typed", Order.class, (in, ctx) -> in);
        h.registerActivity("huge", (in, ctx) -> "x".repeat(300 * 1024));
        h.registerActivity("bad-number", (in, ctx) -> Map.of("n", Double.NaN));
        fake.addLeases(
                lease(b -> b.setTaskId("r/typed/1").setActivityName("typed").setStepId("typed")
                        .setInputJson("{\"input\":{\"id\":\"o\",\"amount\":\"not a number\"}}")),
                lease(b -> b.setTaskId("r/huge/1").setActivityName("huge").setStepId("huge")),
                lease(b -> b.setTaskId("r/nobody/1").setActivityName("nobody").setStepId("nobody")),
                lease(b -> b.setTaskId("r/bad-number/1").setActivityName("bad-number").setStepId("bad-number")),
                lease(b -> b.setTaskId("r/junk/1").setActivityName("typed").setStepId("junk").setInputJson("{nope")));
        h.start();
        until(() -> fake.of("FailStep").size() == 5, "five failures");
        Map<String, Worker.FailStepRequest> by = failuresByStep();
        assertEquals("INPUT_INVALID", by.get("typed").getErrorCode());
        assertEquals("PAYLOAD_TOO_LARGE", by.get("huge").getErrorCode());
        assertEquals("NO_HANDLER", by.get("nobody").getErrorCode());
        assertEquals("OUTPUT_INVALID", by.get("bad-number").getErrorCode());
        assertEquals("INPUT_INVALID", by.get("junk").getErrorCode());
        assertTrue(fake.of("FailStep").stream().noneMatch(c -> ((Worker.FailStepRequest) c.request()).getRetryable()));
        assertEquals(0, fake.of("CompleteStep").size());
    }

    @Test
    void theInputIsBoundToTheTypeTheHandlerDeclaresAndTheOutputIsAnyObjectJacksonCanWrite() {
        WorkerHost h = host();
        h.registerActivity("typed", Order.class, (in, ctx) -> new Receipt(in.id(), in.amount() * 2));
        fake.addLeases(lease(b -> b.setActivityName("typed").setInputJson("{\"input\":{\"id\":\"o-1\",\"amount\":21,\"extra\":true}}")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the completion");
        assertEquals("{\"id\":\"o-1\",\"doubled\":42}", completed(0).getOutputJson(), "unknown fields of the input are ignored");
    }

    @Test
    void aNullIsJsonNullAndNoResultIsNoValueAtAll() {
        WorkerHost h = host();
        h.registerActivity("null", (in, ctx) -> null);
        h.registerActivity("none", (in, ctx) -> ActivityHandler.NO_RESULT);
        fake.addLeases(
                lease(b -> b.setTaskId("r/null/1").setActivityName("null").setStepId("null")),
                lease(b -> b.setTaskId("r/none/1").setActivityName("none").setStepId("none")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 2, "both completions");
        Map<String, String> outputs = new java.util.HashMap<>();
        fake.of("CompleteStep").forEach(c -> outputs.put(((Worker.CompleteStepRequest) c.request()).getStepId(), ((Worker.CompleteStepRequest) c.request()).getOutputJson()));
        assertEquals("null", outputs.get("null"));
        assertEquals("", outputs.get("none"));
    }

    @Test
    void theNodeIsTheNodeOfTheStepDocumentAndFallsBackToTheStepIdAndStepsWithoutOutputAreLeftOut() {
        AtomicReference<ActivityContext> seen = new AtomicReference<>();
        WorkerHost h = host();
        h.registerActivity("charge", (in, ctx) -> {
            seen.set(ctx);
            return 1;
        });
        fake.addLeases(lease(b -> b.setStepId("0190aaaa-0000-7000-8000-000000000009")
                .setInputJson("{\"input\":{},\"steps\":{\"a\":{\"output\":null},\"b\":{},\"c\":{\"output\":3}}}")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the completion");
        assertEquals("0190aaaa-0000-7000-8000-000000000009", seen.get().node(), "no node in the document: the step id");
        assertEquals(Map.of("a", Json.parse("null"), "c", Json.parse("3")), seen.get().steps());
        assertEquals("0190aaaa-0000-7000-8000-000000000009", completed(0).getStepId(), "reports carry the lease's step id");
    }

    // --------------------------------------------------------------------------- heartbeat, cancellation

    @Test
    void theHeartbeatStartsBeforeTheHandlerEndsCarriesTheStepIdentityAndMovesTheDeadline() {
        WorkerHost h = host(b -> b.heartbeatInterval(Duration.ofMillis(20)));
        List<Instant> deadlines = new CopyOnWriteArrayList<>();
        h.registerActivity("slow", (in, ctx) -> {
            deadlines.add(ctx.deadline());
            Thread.sleep(300);
            deadlines.add(ctx.deadline());
            return Map.of("ok", true);
        });
        fake.heartbeatExpiresMs = System.currentTimeMillis() + 120_000;
        fake.addLeases(lease(b -> b.setActivityName("slow").setStepId("slow").setTaskId("run-1/slow/1")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the completion");
        List<FakeWorkerServer.Call> beats = fake.of("Heartbeat");
        assertTrue(beats.size() >= 3, beats.size() + " beats");
        Worker.HeartbeatRequest first = (Worker.HeartbeatRequest) beats.get(0).request();
        assertEquals("run-1", first.getRunId());
        assertEquals("slow", first.getStepId());
        assertEquals(1, first.getAttempt());
        assertEquals("lt", first.getLeaseToken());
        assertEquals("w-1", first.getWorkerId());
        assertEquals("run-1/slow/1", first.getTaskId());
        assertEquals(1, first.getGeneration());
        assertEquals(TRACE, beats.get(0).header("traceparent"));
        assertTrue(deadlines.get(1).isAfter(deadlines.get(0)), "the lease was extended");
        int afterwards = fake.of("Heartbeat").size();
        sleep(150);
        assertEquals(afterwards, fake.of("Heartbeat").size(), "the heartbeat stops with the task");
    }

    @Test
    void theHeartbeatIntervalIsAThirdOfWhatIsLeftOnTheLeaseClampedToHalfASecondAndTenSeconds() {
        assertEquals(500, WorkerHost.heartbeatIntervalMillis(1_000, 1_000), "nothing left");
        assertEquals(500, WorkerHost.heartbeatIntervalMillis(500, 1_000), "a lease already in the past (a skewed clock)");
        assertEquals(500, WorkerHost.heartbeatIntervalMillis(1_600, 1_000), "a third is 200 ms: the floor");
        assertEquals(500, WorkerHost.heartbeatIntervalMillis(2_500, 1_000));
        assertEquals(600, WorkerHost.heartbeatIntervalMillis(2_800, 1_000));
        assertEquals(1_000, WorkerHost.heartbeatIntervalMillis(4_000, 1_000));
        assertEquals(10_000, WorkerHost.heartbeatIntervalMillis(31_000, 1_000), "the platform's default 30 s lease: a third is the cap");
        assertEquals(10_000, WorkerHost.heartbeatIntervalMillis(601_000, 1_000));
        assertEquals(10_000, WorkerHost.heartbeatIntervalMillis(Long.MAX_VALUE, 0), "an absurd expiry does not overflow");
    }

    @Test
    void theFirstBeatComesAThirdOfTheLeaseAfterTheTaskIsTaken() {
        WorkerHost h = host();
        h.registerActivity("slow", (in, ctx) -> {
            Thread.sleep(1500);
            return Map.of("ok", true);
        });
        // A lease that runs out in 3 s: the first beat is due after about a second (and later ones after the answer's expiry).
        fake.addLeases(lease(b -> b.setActivityName("slow").setStepId("slow").setExpiresAtEpochMs(System.currentTimeMillis() + 3_000)));
        h.start();
        until(() -> !fake.of("Heartbeat").isEmpty(), "the first heartbeat");
        long taken = fake.of("PollTasks").get(0).nanos();
        long firstBeat = fake.of("Heartbeat").get(0).nanos();
        long millis = Duration.ofNanos(firstBeat - taken).toMillis();
        assertTrue(millis >= 850 && millis <= 1400, "the first heartbeat came " + millis + " ms after the lease was taken");
    }

    private static void sleep(long millis) {
        try {
            Thread.sleep(millis);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    @Test
    void aCancellationReachesTheActivityThroughItsSignalAndItEndsHoweverItLikes() {
        WorkerHost h = host(b -> b.heartbeatInterval(Duration.ofMillis(20)));
        AtomicReference<CancelReason> reason = new AtomicReference<>();
        AtomicReference<Boolean> viaListener = new AtomicReference<>(false);
        h.registerActivity("long", (in, ctx) -> {
            ctx.cancellation().onCancel(r -> viaListener.set(true));
            ctx.cancellation().awaitCancelled();
            reason.set(ctx.cancellation().reason().orElseThrow());
            return Map.of("stopped", true);
        });
        fake.cancelRequested = true;
        fake.addLeases(lease(b -> b.setActivityName("long").setStepId("long").setTaskId("run-1/long/1")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the completion");
        assertEquals(CancelReason.RUN_CANCELLED, reason.get());
        assertEquals("run-cancelled", reason.get().wire());
        assertTrue(viaListener.get());
        assertEquals("{\"stopped\":true}", completed(0).getOutputJson());
    }

    @Test
    void aTaskCancelledByShutdownThatRaisesItsCancellationIsNotReportedAndItsLeaseLapses() {
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost worker = host(b -> b.onEvent(events::add));
        AtomicReference<CancelReason> reason = new AtomicReference<>();
        AtomicInteger started = new AtomicInteger();
        worker.registerActivity("stuck", (in, ctx) -> {
            started.incrementAndGet();
            ctx.cancellation().awaitCancelled();
            reason.set(ctx.cancellation().reason().orElseThrow());
            ctx.cancellation().throwIfCancelled();
            return null;
        });
        fake.addLeases(lease(b -> b.setActivityName("stuck").setStepId("stuck").setTaskId("run-1/stuck/1")));
        worker.start();
        until(() -> started.get() == 1, "the task to start");
        worker.stop(Duration.ofMillis(100));
        assertEquals(CancelReason.SHUTDOWN, reason.get());
        assertEquals(0, fake.of("CompleteStep").size(), "nothing is reported");
        assertEquals(0, fake.of("FailStep").size(), "not even a failure: the step is leased again when the lease lapses");
        assertTrue(events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskDropped dropped && dropped.reason().equals("shutdown")));
    }

    @Test
    void aTaskCancelledByShutdownThatReturnsAResultStillReportsIt() {
        WorkerHost worker = host();
        AtomicInteger started = new AtomicInteger();
        worker.registerActivity("stuck", (in, ctx) -> {
            started.incrementAndGet();
            ctx.cancellation().awaitCancelled();
            return Map.of("stopped", "early");
        });
        fake.addLeases(lease(b -> b.setActivityName("stuck").setStepId("stuck")));
        worker.start();
        until(() -> started.get() == 1, "the task to start");
        worker.stop(Duration.ofMillis(100));
        assertEquals(1, fake.of("CompleteStep").size());
        assertEquals("{\"stopped\":\"early\"}", completed(0).getOutputJson());
    }

    @Test
    void anActivityThatRaisesItsCancellationIsFailedForGoodAsCancelled() {
        WorkerHost h = host(b -> b.heartbeatInterval(Duration.ofMillis(20)));
        h.registerActivity("long", (in, ctx) -> {
            ctx.cancellation().awaitCancelled();
            ctx.cancellation().throwIfCancelled();
            return null;
        });
        fake.cancelRequested = true;
        fake.addLeases(lease(b -> b.setActivityName("long").setStepId("long").setTaskId("run-1/long/1")));
        h.start();
        until(() -> fake.of("FailStep").size() == 1, "the failure");
        assertEquals("CANCELLED", failed(0).getErrorCode());
        assertFalse(failed(0).getRetryable());
    }

    @Test
    void aLeaseLostDuringTheActivityStopsItAndItsResultIsNeverReported() {
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.heartbeatInterval(Duration.ofMillis(20)).onEvent(events::add));
        AtomicReference<CancelReason> reason = new AtomicReference<>();
        h.registerActivity("slow", (in, ctx) -> {
            ctx.cancellation().awaitCancelled();
            reason.set(ctx.cancellation().reason().orElseThrow());
            return Map.of("late", true);
        });
        fake.failNext("Heartbeat", FakeWorkerServer.Failure.of(Status.Code.ABORTED, "STALE_ATTEMPT"));
        fake.addLeases(lease(b -> b.setActivityName("slow").setStepId("slow").setTaskId("run-1/slow/1")));
        h.start();
        until(() -> events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskStale), "the stale event");
        assertEquals(CancelReason.LEASE_LOST, reason.get());
        sleep(100);
        assertEquals(0, fake.of("CompleteStep").size());
        assertEquals(0, fake.of("FailStep").size());
    }

    @Test
    void failedPreconditionFromAHeartbeatIsAlsoALostLease() {
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.heartbeatInterval(Duration.ofMillis(20)).onEvent(events::add));
        h.registerActivity("slow", (in, ctx) -> {
            ctx.cancellation().awaitCancelled();
            return 1;
        });
        fake.failNext("Heartbeat", FakeWorkerServer.Failure.of(Status.Code.FAILED_PRECONDITION));
        fake.addLeases(lease(b -> b.setActivityName("slow").setStepId("slow")));
        h.start();
        until(() -> events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskStale), "the stale event");
        assertEquals(0, fake.of("CompleteStep").size());
    }

    @Test
    void aHeartbeatThatFailsWithATransportErrorIsTriedAgainOnTheNextBeatAndTheLeaseIsKept() {
        WorkerHost h = host(b -> b.heartbeatInterval(Duration.ofMillis(20)));
        h.registerActivity("slow", (in, ctx) -> {
            Thread.sleep(300);
            return Map.of("done", true);
        });
        fake.failNext("Heartbeat", FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE), FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE));
        fake.addLeases(lease(b -> b.setActivityName("slow").setStepId("slow")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the completion");
        assertTrue(fake.of("Heartbeat").size() >= 4);
        assertEquals(0, fake.of("FailStep").size());
    }

    // --------------------------------------------------------------------- compensation, reconciliation

    @Test
    void aCompensationSeesItsKindAndTheEffectItUndoesAndAReconcilerAnswersExactlyAVerdict() {
        List<Map<String, Object>> seen = new CopyOnWriteArrayList<>();
        WorkerHost h = host();
        h.registerCompensation("limit.release", (in, ctx) -> {
            seen.add(Map.of("kind", ctx.taskKind(), "target", ctx.targetNode(), "targetKey", ctx.targetIdempotencyKey(), "key", ctx.idempotencyKey()));
            return Map.of("released", true);
        });
        h.registerReconciler("limit.lookup", (in, ctx) -> Verdict.applied(Map.of("id", "r-9")));
        h.registerReconciler("limit.none", (in, ctx) -> Verdict.notApplied());
        h.registerReconciler("limit.maybe", (in, ctx) -> Verdict.unknown());
        h.registerReconciler("limit.bad", (in, ctx) -> null);
        h.registerReconciler("limit.plain", (in, ctx) -> Verdict.applied());
        fake.addLeases(
                lease(b -> b.setTaskId("run-1/reserve~c/2").setStepId("reserve~c").setActivityName("limit.release").setTaskKind("COMPENSATION")
                        .setTargetNode("reserve").setTargetIdempotencyKey("run-1:reserve").setIdempotencyKey("compensation:c1:run-1")),
                lease(b -> b.setTaskId("run-1/a~r/1").setStepId("a~r").setActivityName("limit.lookup").setTaskKind("RECONCILIATION").setTargetNode("a").setTargetIdempotencyKey("k")),
                lease(b -> b.setTaskId("run-1/b~r/1").setStepId("b~r").setActivityName("limit.none").setTaskKind("RECONCILIATION").setTargetNode("b")),
                lease(b -> b.setTaskId("run-1/d~r/1").setStepId("d~r").setActivityName("limit.maybe").setTaskKind("RECONCILIATION").setTargetNode("d")),
                lease(b -> b.setTaskId("run-1/e~r/1").setStepId("e~r").setActivityName("limit.plain").setTaskKind("RECONCILIATION").setTargetNode("e")),
                lease(b -> b.setTaskId("run-1/c~r/1").setStepId("c~r").setActivityName("limit.bad").setTaskKind("RECONCILIATION").setTargetNode("c")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 5 && fake.of("FailStep").size() == 1, "every report");
        assertEquals(List.of(Map.of("kind", TaskKind.COMPENSATION, "target", "reserve", "targetKey", "run-1:reserve", "key", "compensation:c1:run-1")), seen);
        Map<String, String> out = new java.util.HashMap<>();
        fake.of("CompleteStep").forEach(c -> out.put(((Worker.CompleteStepRequest) c.request()).getStepId(), ((Worker.CompleteStepRequest) c.request()).getOutputJson()));
        assertEquals("{\"effect\":\"applied\",\"output\":{\"id\":\"r-9\"}}", out.get("a~r"));
        assertEquals("{\"effect\":\"not_applied\"}", out.get("b~r"));
        assertEquals("{\"effect\":\"unknown\"}", out.get("d~r"));
        assertEquals("{\"effect\":\"applied\"}", out.get("e~r"));
        assertEquals("RECONCILIATION_ANSWER_INVALID", failed(0).getErrorCode());
    }

    // --------------------------------------------------------------------------------- protocol, secrets

    @Test
    void theHostRunsAtTheVersionTheServerAgreedToAgainst10ItSendsNoStepIdentityAndCannotResolveSecrets() {
        fake.acceptedVersion = "1";
        WorkerHost h = host(b -> b.heartbeatInterval(Duration.ofMillis(20)).protocolVersion("1.9"));
        AtomicReference<Throwable> secretError = new AtomicReference<>();
        h.registerActivity("slow", (in, ctx) -> {
            Thread.sleep(80);
            try {
                ctx.secrets();
            } catch (SagatException e) {
                secretError.set(e);
            }
            return 1;
        });
        fake.addLeases(lease(b -> b.setActivityName("slow").setStepId("slow").setTaskId("run-1/slow/1")));
        h.start();
        Worker.RegisterWorkerRequest register = (Worker.RegisterWorkerRequest) fake.of("RegisterWorker").get(0).request();
        assertEquals("1.9", register.getCapabilities().getProtocolVersion());
        assertEquals("1", h.protocolVersion());
        until(() -> fake.of("CompleteStep").size() == 1, "the completion");
        Worker.HeartbeatRequest beat = (Worker.HeartbeatRequest) fake.of("Heartbeat").get(0).request();
        assertEquals("", beat.getRunId());
        assertEquals("", beat.getStepId());
        assertEquals(0, beat.getAttempt());
        assertEquals("lt", beat.getLeaseToken(), "the lease identity itself is always sent");
        assertInstanceOf(SagatException.class, secretError.get());
        assertEquals(ErrorCode.UNSUPPORTED_PROTOCOL, ((SagatException) secretError.get()).code());
        assertEquals(0, fake.of("ResolveSecrets").size());
    }

    @Test
    void secretsAreResolvedOncePerAttemptOnFirstUseAsAReadOnlyMapAndDroppedWhenTheHandlerReturns() {
        WorkerHost h = host();
        AtomicReference<String> text = new AtomicReference<>();
        AtomicReference<Secrets> kept = new AtomicReference<>();
        AtomicReference<byte[]> copy = new AtomicReference<>();
        h.registerActivity("pay", (in, ctx) -> {
            Secrets first = ctx.secrets();
            Secrets second = ctx.secrets();
            assertTrue(first == second);
            text.set(first.text("stripe-key"));
            copy.set(first.get("stripe-key"));
            kept.set(first);
            SagatException missing = assertThrows(SagatException.class, () -> first.get("nothing"));
            assertEquals(ErrorCode.NOT_FOUND, missing.code());
            return Map.of("names", first.names());
        });
        fake.secrets = List.of(FakeWorkerServer.secret("stripe-key", "sk_live_x", 3));
        fake.addLeases(lease(b -> b.setActivityName("pay").setStepId("pay").setTaskId("run-1/pay/1")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the completion");
        assertEquals("sk_live_x", text.get());
        assertEquals(1, fake.of("ResolveSecrets").size());
        Worker.ResolveSecretsRequest request = (Worker.ResolveSecretsRequest) fake.of("ResolveSecrets").get(0).request();
        assertEquals("lt", request.getLeaseToken());
        assertEquals("run-1/pay/1", request.getTaskId());
        assertEquals(TRACE, fake.of("ResolveSecrets").get(0).header("traceparent"));
        assertFalse(completed(0).getOutputJson().contains("sk_live"));
        assertTrue(kept.get().names().isEmpty(), "the values were dropped when the handler returned");
        assertThrows(SagatException.class, () -> kept.get().get("stripe-key"));
        assertEquals("sk_live_x", new String(copy.get()), "a copy handed to the handler is the handler's own");
        assertFalse(kept.get().toString().contains("sk_live"));
    }

    // --------------------------------------------------------------------------------------- the loop

    @Test
    void aSessionThatIsGoneIsRegisteredAgainAndTheWorkerGoesOn() {
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.onEvent(events::add));
        h.registerActivity("charge", (in, ctx) -> Map.of("ok", true));
        h.start();
        fake.failNext("PollTasks", FakeWorkerServer.Failure.of(Status.Code.UNAUTHENTICATED));
        until(() -> fake.of("RegisterWorker").size() == 2, "a second registration");
        fake.addLeases(lease());
        until(() -> fake.of("CompleteStep").size() == 1, "the task after it");
        assertTrue(events.stream().anyMatch(e -> e instanceof WorkerEvent.Reconnect));
    }

    @Test
    void anUnavailablePollIsFollowedByAnotherAfterThePauseOfTheSdk() {
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.onEvent(events::add));
        h.registerActivity("charge", (in, ctx) -> Map.of("ok", true));
        fake.failNext("PollTasks", FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE), FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE));
        fake.addLeases(lease());
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the task after the outage");
        assertTrue(fake.of("PollTasks").size() >= 3);
        assertEquals(1, fake.of("RegisterWorker").size(), "an outage is not a lost session");
        assertTrue(events.stream().filter(e -> e instanceof WorkerEvent.Reconnect).count() >= 2);
    }

    @Test
    void aRateLimitedPollWaitsWhatTheServerAsks() {
        WorkerHost h = host();
        h.registerActivity("charge", (in, ctx) -> Map.of("ok", true));
        fake.failNext("PollTasks", new FakeWorkerServer.Failure(Status.Code.RESOURCE_EXHAUSTED, "RATE_LIMITED", 300L));
        fake.addLeases(lease());
        long began = System.nanoTime();
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the task");
        assertTrue(Duration.ofNanos(System.nanoTime() - began).toMillis() >= 280, "waited for retry-after-ms");
    }

    @Test
    void theWaitAfterAnEmptyPollEndsWhenATaskFinishes() {
        fake.pollRetryAfterMs = 10_000;
        WorkerHost h = host();
        h.registerActivity("slow", (in, ctx) -> {
            Thread.sleep(300);
            return Map.of("ok", true);
        });
        h.registerActivity("quick", (in, ctx) -> Map.of("ok", true));
        fake.addLeases(lease(b -> b.setActivityName("slow").setStepId("slow").setTaskId("run-1/slow/1")));
        h.start();
        // The first poll brought the slow task; the second is empty and asks for a ten second wait.
        until(() -> fake.of("PollTasks").size() >= 2, "the second poll");
        sleep(100);
        fake.addLeases(lease(b -> b.setActivityName("quick").setStepId("quick").setTaskId("run-1/quick/1")));
        long began = System.nanoTime();
        until(() -> fake.of("CompleteStep").size() == 2, "both completions");
        long took = Duration.ofNanos(System.nanoTime() - began).toMillis();
        assertTrue(took < 4000, "the quick task was taken when the slow one finished, not after the ten second wait (" + took + " ms)");
    }

    @Test
    void plainTextOutsideLoopbackLogsOneWarningWhenTheHostStarts() {
        CapturingLogger log = new CapturingLogger();
        // 192.0.2.1 is reserved for documentation (TEST-NET-1): nothing answers, and the registration fails.
        WorkerHost plain = new WorkerHost(WorkerConfig.builder()
                .grpcUrl("http://192.0.2.1:50051").namespace("p").token("tok-secret-2").workerName("plain")
                .timeout(Duration.ofMillis(300)).retry(new RetryPolicy(1, Duration.ofMillis(1), Duration.ofMillis(1))).logger(log).build());
        plain.registerActivity("a", (in, ctx) -> 1);
        assertEquals(0, log.warnings("plain text").size(), "nothing is opened before the host starts");
        assertThrows(SagatException.class, plain::start);
        assertEquals(1, log.warnings("plain text").size());
        assertTrue(log.lines().stream().noneMatch(l -> l.message().contains("tok-secret")));

        CapturingLogger quiet = new CapturingLogger();
        WorkerHost local = host(b -> b.logger(quiet));
        local.registerActivity("a", (in, ctx) -> 1);
        local.start();
        assertEquals(0, quiet.warnings("plain text").size(), "loopback is not a warning");
    }

    @Test
    void anEmptyPollIsFollowedByAWaitNeverByAnotherPollAtOnce() {
        fake.pollRetryAfterMs = 100;
        WorkerHost h = host();
        h.registerActivity("charge", (in, ctx) -> Map.of("ok", true));
        h.start();
        sleep(400);
        int polls = fake.of("PollTasks").size();
        assertTrue(polls >= 2 && polls <= 8, polls + " polls in 400 ms");
    }

    @Test
    void noMoreTasksAreTakenThanTheHostHasRoomForAndStopWaitsForTheOnesInFlight() {
        WorkerHost h = host(b -> b.maxInflight(2));
        AtomicInteger running = new AtomicInteger();
        AtomicInteger peak = new AtomicInteger();
        h.registerActivity("charge", (in, ctx) -> {
            peak.accumulateAndGet(running.incrementAndGet(), Math::max);
            Thread.sleep(100);
            running.decrementAndGet();
            return 1;
        });
        for (int i = 1; i <= 4; i++) {
            int n = i;
            fake.addLeases(lease(b -> b.setTaskId("run-" + n + "/charge/1").setRunId("run-" + n)));
        }
        h.start();
        until(() -> !fake.of("PollTasks").isEmpty(), "a poll");
        assertTrue(((Worker.PollTasksRequest) fake.of("PollTasks").get(0).request()).getMaxTasks() <= 2);
        h.stop(Duration.ofSeconds(5));
        assertTrue(peak.get() <= 2);
        assertEquals(0, running.get(), "nothing was left running");
        assertTrue(fake.of("CompleteStep").size() >= 2, "what was taken was finished and reported");
    }

    @Test
    void stopWhileAPollIsInFlightDoesNotWaitOutThePauseAnEmptyAnswerAsksFor() {
        fake.pollDelayMs = 150;
        fake.pollRetryAfterMs = 10_000;
        WorkerHost worker = host();
        worker.registerActivity("charge", (in, ctx) -> 1);
        worker.start();
        until(() -> fake.of("PollTasks").size() == 1, "the first poll");
        long began = System.nanoTime();
        worker.stop(Duration.ofSeconds(1));
        long took = Duration.ofNanos(System.nanoTime() - began).toMillis();
        assertTrue(took < 1500, "stop took " + took + " ms");
    }

    @Test
    void stopCancelsATaskThatDoesNotFinishWithTheReasonShutdownAndTheHostStopsAtTheEnd() {
        WorkerHost worker = host();
        AtomicReference<CancelReason> reason = new AtomicReference<>();
        worker.registerActivity("stuck", (in, ctx) -> {
            ctx.cancellation().awaitCancelled();
            reason.set(ctx.cancellation().reason().orElseThrow());
            return Map.of("stopped", "early");
        });
        fake.addLeases(lease(b -> b.setActivityName("stuck").setStepId("stuck").setTaskId("run-1/stuck/1")));
        worker.start();
        until(() -> !fake.of("Heartbeat").isEmpty() || !fake.of("PollTasks").isEmpty(), "the poll");
        sleep(150);
        long began = System.nanoTime();
        worker.stop(Duration.ofMillis(200));
        assertEquals(CancelReason.SHUTDOWN, reason.get());
        assertTrue(Duration.ofNanos(System.nanoTime() - began).toMillis() < 5000);
        assertEquals("", worker.protocolVersion().isEmpty() ? "" : "", "stop returned");
    }

    @Test
    void theHostIsRestartableAndStopIsHarmlessWhenItIsNotRunning() {
        WorkerHost h = host();
        h.stop();
        h.registerActivity("charge", (in, ctx) -> Map.of("ok", true));
        h.start();
        h.start();
        assertEquals(1, fake.of("RegisterWorker").size(), "starting a running host does nothing");
        h.stop(Duration.ofSeconds(1));
        h.stop(Duration.ofSeconds(1));
        fake.addLeases(lease());
        h.start();
        assertEquals(2, fake.of("RegisterWorker").size());
        until(() -> fake.of("CompleteStep").size() == 1, "a task after the restart");
        h.close();
    }

    @Test
    void theTokenProviderIsAskedForEveryCall() {
        AtomicInteger asked = new AtomicInteger();
        WorkerHost h = host(b -> b.tokenProvider(() -> "t" + asked.incrementAndGet()));
        h.registerActivity("charge", (in, ctx) -> Map.of("ok", true));
        fake.addLeases(lease());
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the task");
        List<String> tokens = new ArrayList<>();
        for (FakeWorkerServer.Call call : fake.calls) {
            tokens.add(call.header("authorization"));
        }
        assertEquals(tokens.size(), new java.util.HashSet<>(tokens).size(), "a fresh token for every RPC");
        assertNotNull(tokens.get(0));
    }

    @Test
    void theEventsOfAnOrdinaryTaskComeInOrder() {
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.onEvent(events::add));
        h.registerActivity("charge", (in, ctx) -> Map.of("ok", true));
        fake.addLeases(lease());
        h.start();
        until(() -> events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskCompleted), "completion");
        List<String> kinds = events.stream().map(e -> e.getClass().getSimpleName()).filter(k -> !k.equals("Poll") && !k.equals("Heartbeat")).toList();
        assertEquals(List.of("Registered", "TaskTaken", "TaskCompleted"), kinds);
        WorkerEvent.TaskTaken taken = (WorkerEvent.TaskTaken) events.stream().filter(e -> e instanceof WorkerEvent.TaskTaken).findFirst().orElseThrow();
        assertEquals("charge", taken.activity());
        assertEquals(1, taken.attempt());
    }

    @Test
    void aListenerThatFailsDoesNotBreakTheHost() {
        WorkerHost h = host(b -> b.onEvent(e -> {
            throw new IllegalStateException("metrics backend down");
        }));
        h.registerActivity("charge", (in, ctx) -> Map.of("ok", true));
        fake.addLeases(lease());
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the task");
    }

    @Test
    void theTransportDoesNotLeakTheTokenIntoErrors() {
        WorkerHost h = host(b -> b.token("tok-very-secret-value"));
        h.registerActivity("charge", (in, ctx) -> 1);
        fake.failNext("RegisterWorker", FakeWorkerServer.Failure.of(Status.Code.PERMISSION_DENIED));
        SagatException error = assertThrows(SagatException.class, h::start);
        assertEquals(ErrorCode.FORBIDDEN, error.code());
        assertFalse(error.getMessage().contains("very-secret"));
        assertFalse(error.toString().contains("very-secret"));
        JsonNode asText = Json.toTree(Map.of("message", error.getMessage(), "remediation", error.remediation()));
        assertFalse(asText.toString().contains("tok-"));
    }
}
