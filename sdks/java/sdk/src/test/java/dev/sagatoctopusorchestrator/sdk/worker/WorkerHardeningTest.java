package dev.sagatoctopusorchestrator.sdk.worker;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import dev.sagatoctopusorchestrator.protocol.v1.Worker;
import dev.sagatoctopusorchestrator.sdk.CapturingLogger;
import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.RetryPolicy;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import dev.sagatoctopusorchestrator.sdk.ThrowingLogger;
import io.grpc.Status;
import java.io.IOException;
import java.math.BigDecimal;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.BooleanSupplier;
import java.util.function.Consumer;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;

/**
 * What the worker host does with a hostile or broken service and with the hooks of a careless caller (SPEC sections
 * 5, 6.2, 6.7, 10): storms, waits, sizes, hooks that throw, the number of tasks run, and the gaps mutation found.
 */
@Timeout(120)
class WorkerHardeningTest {
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

    private static Worker.TaskLease lease(String task, Consumer<Worker.TaskLease.Builder> tweak) {
        Worker.TaskLease.Builder builder = Worker.TaskLease.newBuilder()
                .setTaskId(task)
                .setRunId("run-1")
                .setStepId("charge")
                .setAttempt(1)
                .setGeneration(1)
                .setLeaseToken("lt")
                .setExpiresAtEpochMs(System.currentTimeMillis() + 60_000)
                .setActivityName("charge")
                .setInputJson("{\"run\":{\"id\":\"run-1\"},\"node\":\"charge\",\"input\":{\"amount\":5},\"steps\":{}}")
                .setTraceparent(TRACE)
                .setIdempotencyKey("run-1:charge");
        tweak.accept(builder);
        return builder.build();
    }

    private static Worker.TaskLease lease(String task) {
        return lease(task, b -> {});
    }

    private WorkerHost host(Consumer<WorkerConfig.Builder> tweak) {
        WorkerConfig.Builder builder = WorkerConfig.builder()
                .grpcUrl(fake.url())
                .namespace("payments")
                .token("tok")
                .workerName("hardening")
                .retry(new RetryPolicy(4, Duration.ofMillis(5), Duration.ofMillis(20)));
        tweak.accept(builder);
        WorkerHost created = new WorkerHost(builder.build());
        hosts.add(created);
        return created;
    }

    private static void until(BooleanSupplier condition, String what) {
        long end = System.nanoTime() + Duration.ofSeconds(15).toNanos();
        while (!condition.getAsBoolean()) {
            if (System.nanoTime() > end) {
                throw new AssertionError("timed out waiting for " + what);
            }
            sleep(10);
        }
    }

    private static void sleep(long millis) {
        try {
            Thread.sleep(millis);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new AssertionError(e);
        }
    }

    private static FakeWorkerServer.Failure[] many(int n, FakeWorkerServer.Failure failure) {
        FakeWorkerServer.Failure[] all = new FakeWorkerServer.Failure[n];
        Arrays.fill(all, failure);
        return all;
    }

    // ------------------------------------------------------------------------------------ retry storm

    @Test
    void aSessionRefusedAgainRightAfterRegisteringIsBackedOffNotHammered() {
        fake.failNext("PollTasks", many(200_000, FakeWorkerServer.Failure.of(Status.Code.UNAUTHENTICATED)));
        WorkerHost h = host(b -> b.retry(RetryPolicy.DEFAULT));
        h.registerActivity("charge", (in, ctx) -> 1);
        h.start();
        sleep(2000);
        int registrations = fake.of("RegisterWorker").size();
        assertTrue(registrations >= 2 && registrations <= 10, registrations + " registrations in 2 s");
        assertTrue(fake.of("PollTasks").size() <= 12, fake.of("PollTasks").size() + " polls in 2 s");
    }

    @Test
    void theBackoffIsResetByASuccessfulPollSoALaterLostSessionIsRegisteredAgainAtOnce() {
        WorkerHost h = host(b -> b.retry(new RetryPolicy(4, Duration.ofSeconds(2), Duration.ofSeconds(10))));
        h.registerActivity("charge", (in, ctx) -> 1);
        // Three refusals in a row: the second and third registration wait, the first does not.
        fake.failNext("PollTasks", many(3, FakeWorkerServer.Failure.of(Status.Code.UNAUTHENTICATED)));
        long began = System.nanoTime();
        h.start();
        until(() -> fake.of("RegisterWorker").size() == 4, "the registrations after three refusals");
        assertTrue(Duration.ofNanos(System.nanoTime() - began).toMillis() >= 1500, "the repeated refusals were waited out");
        // A poll then succeeds: the streak is over. A new refusal is registered again without a wait.
        until(() -> fake.of("PollTasks").size() >= 5, "a successful poll");
        int before = fake.of("RegisterWorker").size();
        long refused = System.nanoTime();
        fake.failNext("PollTasks", FakeWorkerServer.Failure.of(Status.Code.UNAUTHENTICATED));
        until(() -> fake.of("RegisterWorker").size() == before + 1, "the next registration");
        assertTrue(Duration.ofNanos(System.nanoTime() - refused).toMillis() < 1500, "a first refusal is not made to wait");
    }

    // ----------------------------------------------------------------------------------- Jackson ReDoS

    private record Priced(BigDecimal price) {}

    @Test
    void aNumberFieldFedAHugeStringIsRefusedInTimeNotParsedForSeconds() {
        WorkerHost h = host(b -> {});
        AtomicInteger ran = new AtomicInteger();
        h.registerActivity("priced", Priced.class, (in, ctx) -> ran.incrementAndGet());
        String evil = "0".repeat(40_000) + "x";
        fake.addLeases(lease("run-1/priced/1", b -> b.setActivityName("priced").setStepId("priced").setInputJson("{\"input\":{\"price\":\"" + evil + "\"}}")));
        long began = System.nanoTime();
        h.start();
        until(() -> fake.of("FailStep").size() == 1, "the failure");
        long took = Duration.ofNanos(System.nanoTime() - began).toMillis();
        assertEquals("INPUT_INVALID", ((Worker.FailStepRequest) fake.of("FailStep").get(0).request()).getErrorCode());
        assertEquals(0, ran.get());
        assertTrue(took < 2500, "a 40 000 character string for a number took " + took + " ms to refuse");
    }

    // ---------------------------------------------------------------------- hooks that throw or fail

    @Test
    void aLoggerThatThrowsDoesNotEndTheHeartbeat() {
        ThrowingLogger sink = new ThrowingLogger().armed(true);
        WorkerHost h = host(b -> b.logger(sink).heartbeatInterval(Duration.ofMillis(100)));
        h.registerActivity("slow", (in, ctx) -> {
            Thread.sleep(1500);
            return Map.of("ok", true);
        });
        fake.failNext("Heartbeat", FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE));
        fake.addLeases(lease("run-1/slow/1", b -> b.setActivityName("slow").setStepId("slow")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the completion");
        assertTrue(fake.of("Heartbeat").size() >= 5, fake.of("Heartbeat").size() + " heartbeats: the first one failed and the warning about it threw");
    }

    @Test
    void aLoggerThatThrowsDoesNotEndThePollerOrDropATask() {
        ThrowingLogger sink = new ThrowingLogger().armed(true);
        WorkerHost h = host(b -> b.logger(sink));
        AtomicInteger ran = new AtomicInteger();
        h.registerActivity("charge", (in, ctx) -> ran.incrementAndGet());
        h.start();
        fake.failNext("PollTasks", FakeWorkerServer.Failure.of(Status.Code.UNAUTHENTICATED));
        until(() -> fake.of("RegisterWorker").size() == 2, "the registration after the refused poll");
        fake.addLeases(lease("run-1/charge/1"));
        until(() -> fake.of("CompleteStep").size() == 1, "the task after it");
        assertEquals(1, ran.get());
        assertTrue(Thread.getAllStackTraces().keySet().stream().anyMatch(t -> t.getName().endsWith("-poller") && t.isAlive()));
    }

    @Test
    void anErrorFromTheLoggerOrTheEventHookIsSurvivedToo() {
        List<Throwable> errors = new CopyOnWriteArrayList<>();
        ThrowingLogger sink = new ThrowingLogger(() -> new AssertionError("sink")).armed(true);
        WorkerHost h = host(b -> b.logger(sink).onEvent(e -> {
            errors.add(new AssertionError("hook"));
            throw new AssertionError("metrics hook");
        }));
        h.registerActivity("charge", (in, ctx) -> Map.of("ok", true));
        fake.failNext("PollTasks", FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE));
        fake.addLeases(lease("run-1/charge/1"));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the task completes though every hook throws an Error");
        assertFalse(errors.isEmpty());
        fake.addLeases(lease("run-2/charge/1", b -> b.setRunId("run-2")));
        until(() -> fake.of("CompleteStep").size() == 2, "and so does the next one");
    }

    @Test
    void aLostLeaseWithAThrowingLoggerDropsTheResultAndTheHostGoesOn() {
        ThrowingLogger sink = new ThrowingLogger().armed(true);
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.logger(sink).heartbeatInterval(Duration.ofMillis(20)).onEvent(events::add));
        h.registerActivity("slow", (in, ctx) -> {
            ctx.cancellation().awaitCancelled();
            return 1;
        });
        h.registerActivity("charge", (in, ctx) -> 2);
        fake.failNext("Heartbeat", FakeWorkerServer.Failure.of(Status.Code.ABORTED, "STALE_ATTEMPT"));
        fake.addLeases(lease("run-1/slow/1", b -> b.setActivityName("slow").setStepId("slow")));
        h.start();
        until(() -> events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskStale), "the stale event");
        assertEquals(0, fake.of("CompleteStep").size());
        fake.addLeases(lease("run-2/charge/1", b -> b.setRunId("run-2")));
        until(() -> fake.of("CompleteStep").size() == 1, "the next task");
    }

    @Test
    void aTokenProviderThatFailsForAWhileDoesNotEndTheHeartbeat() {
        AtomicBoolean failing = new AtomicBoolean();
        WorkerHost h = host(b -> b.tokenProvider(() -> {
            if (failing.get()) {
                throw new IOException("idp down");
            }
            return "tok";
        }).heartbeatInterval(Duration.ofMillis(30)));
        h.registerActivity("slow", (in, ctx) -> {
            Thread.sleep(150);
            failing.set(true);
            Thread.sleep(300);
            failing.set(false);
            Thread.sleep(600);
            return 1;
        });
        fake.addLeases(lease("run-1/slow/1", b -> b.setActivityName("slow").setStepId("slow")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the completion");
        long lastBeat = fake.of("Heartbeat").get(fake.of("Heartbeat").size() - 1).nanos();
        long completed = fake.of("CompleteStep").get(0).nanos();
        assertTrue(Duration.ofNanos(completed - lastBeat).toMillis() < 400, "heartbeats resumed after the provider recovered");
    }

    // ------------------------------------------------------------------------------ waits from a service

    @Test
    void theDefaultCapOfAWaitTheServiceAsksForIsAMinute() {
        assertEquals(Duration.ofSeconds(60), WorkerConfig.builder().grpcUrl(fake.url()).namespace("p").token("t").workerName("w").build().maxServerWait());
    }

    @Test
    void aRefusedPollThatAsksForNineTrillionMillisecondsWaitsTheCapAndThenGoesOn() {
        WorkerHost h = host(b -> b.maxServerWait(Duration.ofMillis(300)));
        h.registerActivity("charge", (in, ctx) -> 1);
        fake.failNext("PollTasks", new FakeWorkerServer.Failure(Status.Code.RESOURCE_EXHAUSTED, "RATE_LIMITED", 9_000_000_000_000L));
        h.start();
        until(() -> !fake.of("PollTasks").isEmpty(), "the refused poll");
        fake.addLeases(lease("run-1/charge/1"));
        until(() -> fake.of("CompleteStep").size() == 1, "the task after the capped wait");
    }

    @Test
    void anEmptyPollThatAsksForTheLargestIntegerParksTheLoopOnlyForTheCap() {
        fake.pollRetryAfterMs = Integer.MAX_VALUE;
        WorkerHost h = host(b -> b.maxServerWait(Duration.ofMillis(300)));
        h.registerActivity("charge", (in, ctx) -> 1);
        h.start();
        until(() -> !fake.of("PollTasks").isEmpty(), "the first poll");
        sleep(100);
        fake.addLeases(lease("run-1/charge/1"));
        until(() -> fake.of("CompleteStep").size() == 1, "the task after the capped pause");
    }

    @Test
    void aReportRefusedWithAHugeWaitIsRepeatedAfterTheCap() {
        WorkerHost h = host(b -> b.maxServerWait(Duration.ofMillis(300)));
        h.registerActivity("charge", (in, ctx) -> 1);
        fake.failNext("CompleteStep", new FakeWorkerServer.Failure(Status.Code.UNAVAILABLE, "UNAVAILABLE", 9_000_000_000_000L));
        fake.addLeases(lease("run-1/charge/1"));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 2, "the repeated report");
    }

    @Test
    void aWaitFromTheServiceIsCappedInTheErrorItself() {
        fake.failNext("RegisterWorker", new FakeWorkerServer.Failure(Status.Code.RESOURCE_EXHAUSTED, "RATE_LIMITED", 9_000_000_000_000L));
        WorkerHost h = host(b -> b.retry(new RetryPolicy(1, Duration.ofMillis(1), Duration.ofMillis(1))));
        h.registerActivity("charge", (in, ctx) -> 1);
        SagatException error = assertThrows(SagatException.class, h::start);
        assertEquals(ErrorCode.RATE_LIMITED, error.code());
        assertEquals(60_000, error.retryAfterMs().getAsLong());
        assertTrue(error.reason().orElse("").length() <= 128);
    }

    @Test
    void aTrailerWithMoreDigitsThanALongHoldsIsAVeryLongWaitCappedAndAReasonIsBounded() {
        io.grpc.Metadata trailers = new io.grpc.Metadata();
        trailers.put(io.grpc.Metadata.Key.of("retry-after-ms", io.grpc.Metadata.ASCII_STRING_MARSHALLER), "99999999999999999999999");
        trailers.put(io.grpc.Metadata.Key.of("x-sagat-error", io.grpc.Metadata.ASCII_STRING_MARSHALLER), "R".repeat(5000));
        SagatException error = GrpcErrors.of("PollTasks", Status.RESOURCE_EXHAUSTED.withDescription("D".repeat(5000)).asRuntimeException(trailers));
        assertEquals(ErrorCode.RATE_LIMITED, error.code());
        assertEquals(60_000, error.retryAfterMs().getAsLong());
        assertEquals(128, error.reason().orElseThrow().length());
        assertTrue(error.getMessage().length() <= 512);
        assertEquals(null, error.getCause(), "the service's description is not kept as a cause");
    }

    // ------------------------------------------------------------------- the number of tasks that run

    @Test
    void aPollAnswerWithMoreTasksThanAskedRunsOnlyTheFirstOnesAndLeavesTheRestToLapse() throws Exception {
        fake.ignoreMax = true;
        CapturingLogger log = new CapturingLogger();
        WorkerHost h = host(b -> b.maxInflight(2).logger(log));
        AtomicInteger running = new AtomicInteger();
        AtomicInteger peak = new AtomicInteger();
        AtomicInteger started = new AtomicInteger();
        CountDownLatch release = new CountDownLatch(1);
        List<String> ranTasks = new CopyOnWriteArrayList<>();
        h.registerActivity("charge", (in, ctx) -> {
            peak.accumulateAndGet(running.incrementAndGet(), Math::max);
            started.incrementAndGet();
            ranTasks.add(ctx.taskId());
            release.await(5, TimeUnit.SECONDS);
            running.decrementAndGet();
            return 1;
        });
        for (int i = 1; i <= 12; i++) {
            fake.addLeases(lease("run-" + i + "/charge/1"));
        }
        h.start();
        until(() -> started.get() >= 2, "the first tasks");
        sleep(600);
        assertEquals(2, started.get(), "only as many as were asked for ran: " + ranTasks);
        assertEquals(2, peak.get());
        assertEquals(List.of("run-1/charge/1", "run-2/charge/1"), ranTasks.stream().sorted().toList(), "the first ones");
        release.countDown();
        sleep(300);
        assertEquals(2, started.get(), "the rest were left to lapse, not run later");
        assertEquals(0, fake.of("FailStep").size(), "and not failed either");
        assertEquals(1, log.warnings("lapse").size(), "one warning says that tasks were left to lapse");
        assertTrue(log.lines().stream().noneMatch(l -> l.message().contains("lt")), "no lease token in a log line");
    }

    @Test
    void theNumberOfTasksAskedForIsTheFreeCapacityAndAtMost64() {
        CountDownLatch release = new CountDownLatch(1);
        AtomicInteger started = new AtomicInteger();
        WorkerHost h = host(b -> b.maxInflight(3));
        h.registerActivity("charge", (in, ctx) -> {
            started.incrementAndGet();
            release.await(5, TimeUnit.SECONDS);
            return 1;
        });
        fake.addLeases(lease("run-1/charge/1"));
        h.start();
        until(() -> started.get() == 1, "one task in flight");
        until(() -> fake.of("PollTasks").size() >= 2, "a poll with a task in flight");
        List<Integer> asked = fake.of("PollTasks").stream().map(c -> ((Worker.PollTasksRequest) c.request()).getMaxTasks()).toList();
        assertEquals(3, asked.get(0), "all the room, with nothing in flight");
        assertEquals(2, asked.get(1), "the room that is left, with one task in flight");
        release.countDown();
        h.stop(Duration.ofSeconds(2));

        WorkerHost big = host(b -> b.workerName("big").maxInflight(500));
        big.registerActivity("charge", (in, ctx) -> 1);
        fake.calls.clear();
        big.start();
        until(() -> !fake.of("PollTasks").isEmpty(), "a poll");
        assertEquals(64, ((Worker.PollTasksRequest) fake.of("PollTasks").get(0).request()).getMaxTasks());
    }

    // ----------------------------------------------------------------------------- configuration

    @Test
    void aHeartbeatIntervalThatIsNotPositiveIsRefusedNotClampedToAHotLoop() {
        for (Duration bad : new Duration[] {Duration.ZERO, Duration.ofMillis(-5), Duration.ofNanos(500_000), Duration.ofSeconds(-1)}) {
            SagatException error = assertThrows(SagatException.class,
                    () -> WorkerConfig.builder().grpcUrl(fake.url()).namespace("p").token("t").workerName("w").heartbeatInterval(bad).build(), bad.toString());
            assertEquals(ErrorCode.VALIDATION_FAILED, error.code());
        }
        WorkerConfig ok = WorkerConfig.builder().grpcUrl(fake.url()).namespace("p").token("t").workerName("w").heartbeatInterval(Duration.ofMillis(1)).build();
        assertEquals(Duration.ofMillis(1), ok.heartbeatInterval());
        assertEquals(ErrorCode.VALIDATION_FAILED, assertThrows(SagatException.class,
                () -> WorkerConfig.builder().grpcUrl(fake.url()).namespace("p").token("t").workerName("w").maxServerWait(Duration.ZERO).build()).code());
    }

    @Test
    void aConfigAndAHostShowNoCredentialInTheirStringForm() {
        WorkerConfig config = WorkerConfig.builder().grpcUrl("https://w.example:443").namespace("p").token("CANARY-bearer-77").workerName("w").build();
        assertFalse(String.valueOf(config).contains("CANARY"));
        assertFalse(String.valueOf(config.tokenProvider()).contains("CANARY"));
        assertFalse(String.valueOf(new WorkerHost(config)).contains("CANARY"));
        assertFalse(String.valueOf(new WorkerEvent.Registered("1.2")).contains("CANARY"));
    }

    // ---------------------------------------------------------------- gaps that mutation had shown

    @Test
    void stopWakesAPausedLoopInsteadOfWaitingItOut() {
        fake.pollRetryAfterMs = 30_000;
        WorkerHost h = host(b -> {});
        h.registerActivity("charge", (in, ctx) -> 1);
        h.start();
        until(() -> !fake.of("PollTasks").isEmpty(), "the first (empty) poll");
        sleep(200);
        long began = System.nanoTime();
        h.stop(Duration.ofSeconds(10));
        long took = Duration.ofNanos(System.nanoTime() - began).toMillis();
        assertTrue(took < 1500, "stop took " + took + " ms with the loop parked in a 30 s pause");
    }

    @Test
    void theFirstReasonOfACancellationIsTheOneThatStays() {
        CancellationSource source = new CancellationSource();
        List<CancelReason> told = new ArrayList<>();
        source.onCancel(told::add);
        assertTrue(source.cancel(CancelReason.RUN_CANCELLED));
        assertFalse(source.cancel(CancelReason.LEASE_LOST));
        assertFalse(source.cancel(CancelReason.SHUTDOWN));
        assertEquals(CancelReason.RUN_CANCELLED, source.reason().orElseThrow());
        assertEquals(List.of(CancelReason.RUN_CANCELLED), told, "a listener is told once, with the first reason");
        List<CancelReason> late = new ArrayList<>();
        source.onCancel(late::add);
        assertEquals(List.of(CancelReason.RUN_CANCELLED), late, "a listener added afterwards is told at once");
    }

    @Test
    void aTaskCancelledByTheRunAndThenByShutdownIsStillReportedAsCancelled() {
        WorkerHost h = host(b -> b.heartbeatInterval(Duration.ofMillis(20)));
        AtomicReference<CancelReason> firstSeen = new AtomicReference<>();
        AtomicReference<CancelReason> lastSeen = new AtomicReference<>();
        h.registerActivity("long", (in, ctx) -> {
            ctx.cancellation().awaitCancelled();
            firstSeen.set(ctx.cancellation().reason().orElseThrow());
            Thread.sleep(500);
            lastSeen.set(ctx.cancellation().reason().orElseThrow());
            ctx.cancellation().throwIfCancelled();
            return null;
        });
        fake.cancelRequested = true;
        fake.addLeases(lease("run-1/long/1", b -> b.setActivityName("long").setStepId("long")));
        h.start();
        until(() -> firstSeen.get() != null, "the cancellation by the run");
        h.stop(Duration.ofMillis(100));
        assertEquals(CancelReason.RUN_CANCELLED, firstSeen.get());
        assertEquals(CancelReason.RUN_CANCELLED, lastSeen.get(), "the later shutdown did not replace the reason");
        assertEquals(1, fake.of("FailStep").size(), "so the cancellation error is a failure for good, not a lapsed lease");
        assertEquals("CANCELLED", ((Worker.FailStepRequest) fake.of("FailStep").get(0).request()).getErrorCode());
    }

    @Test
    void aFailedResolutionOfSecretsIsNotRememberedForTheRestOfTheAttempt() {
        WorkerHost h = host(b -> {});
        AtomicReference<String> text = new AtomicReference<>();
        AtomicReference<ErrorCode> first = new AtomicReference<>();
        h.registerActivity("pay", (in, ctx) -> {
            try {
                ctx.secrets();
            } catch (SagatException e) {
                first.set(e.code());
            }
            text.set(ctx.secrets().text("db"));
            return 1;
        });
        fake.secrets = List.of(FakeWorkerServer.secret("db", "hunter2", 1));
        fake.failNext("ResolveSecrets", FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE, "UNAVAILABLE"));
        fake.addLeases(lease("run-1/pay/1", b -> b.setActivityName("pay").setStepId("pay")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the completion");
        assertEquals(ErrorCode.UNAVAILABLE, first.get());
        assertEquals("hunter2", text.get());
        assertEquals(2, fake.of("ResolveSecrets").size(), "asked again, because the first answer was a failure");
    }

    @Test
    void aReportIsAttemptedFiveTimesAtMostAndThenDropped() {
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.onEvent(events::add));
        h.registerActivity("charge", (in, ctx) -> 1);
        fake.failNext("CompleteStep", many(50, FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE)));
        fake.addLeases(lease("run-1/charge/1"));
        h.start();
        until(() -> events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskDropped), "the dropped report");
        sleep(200);
        assertEquals(5, fake.of("CompleteStep").size(), "five attempts with one key, no more");
        assertEquals(1, fake.of("CompleteStep").stream().map(c -> ((Worker.CompleteStepRequest) c.request()).getIdempotencyKey()).distinct().count());
        assertEquals(0, fake.of("FailStep").size());
    }

    @Test
    void against11SecretsAreRefusedAndATaskKindIsNotRead() {
        fake.acceptedVersion = "1.1";
        WorkerHost h = host(b -> {});
        AtomicReference<ErrorCode> secretError = new AtomicReference<>();
        AtomicReference<TaskKind> kind = new AtomicReference<>();
        AtomicReference<String> target = new AtomicReference<>();
        h.registerActivity("limit.release", (in, ctx) -> {
            kind.set(ctx.taskKind());
            target.set(ctx.targetNode());
            try {
                ctx.secrets();
            } catch (SagatException e) {
                secretError.set(e.code());
            }
            return 1;
        });
        fake.addLeases(lease("run-1/rel/1", b -> b.setActivityName("limit.release").setStepId("rel").setTaskKind("COMPENSATION").setTargetNode("reserve").setTargetIdempotencyKey("k")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the completion");
        assertEquals(ErrorCode.UNSUPPORTED_PROTOCOL, secretError.get(), "ResolveSecrets came with 1.2");
        assertEquals(0, fake.of("ResolveSecrets").size());
        assertEquals(TaskKind.FORWARD, kind.get(), "a server at 1.1 never sends the other kinds: the field is not read");
        assertEquals("", target.get());
    }

    @Test
    void aSafeMessageIsCutByCodePointsAndIsValidUtf8() {
        WorkerHost h = host(b -> {});
        h.registerActivity("emoji", (in, ctx) -> {
            throw ActivityFailure.permanent("EMOJI", "😀".repeat(600));
        });
        h.registerActivity("lone", (in, ctx) -> {
            throw ActivityFailure.permanent("LONE", "abc\uD800def");
        });
        fake.addLeases(
                lease("run-1/emoji/1", b -> b.setActivityName("emoji").setStepId("emoji")),
                lease("run-1/lone/1", b -> b.setActivityName("lone").setStepId("lone")));
        h.start();
        until(() -> fake.of("FailStep").size() == 2, "both failures");
        for (FakeWorkerServer.Call call : fake.of("FailStep")) {
            Worker.FailStepRequest request = (Worker.FailStepRequest) call.request();
            String message = request.getSafeMessage();
            if (request.getStepId().equals("emoji")) {
                assertEquals(512, message.codePointCount(0, message.length()));
                assertEquals(1024, message.length());
            } else {
                assertEquals("abc�def", message, "a lone surrogate is U+FFFD on the wire");
            }
        }
    }

    // ============================================================================== round 4

    // ---------------------------------------------------------------- stop cancels tokens and nothing else

    @Test
    void stopNeverInterruptsAHandlerThatIgnoresItsToken() {
        WorkerHost h = host(b -> {});
        AtomicBoolean sawInterrupt = new AtomicBoolean();
        AtomicBoolean finished = new AtomicBoolean();
        CountDownLatch started = new CountDownLatch(1);
        h.registerActivity("charge", (in, ctx) -> {
            started.countDown();
            long end = System.nanoTime() + Duration.ofMillis(3500).toNanos();
            while (System.nanoTime() < end) { // never looks at its token
                if (Thread.currentThread().isInterrupted()) {
                    sawInterrupt.set(true);
                }
                try {
                    Thread.sleep(10);
                } catch (InterruptedException e) {
                    sawInterrupt.set(true);
                    Thread.currentThread().interrupt();
                }
            }
            finished.set(true);
            return Map.of("done", true);
        });
        fake.addLeases(lease("run-1/charge/1"));
        h.start();
        until(() -> started.getCount() == 0, "the task to start");
        long began = System.nanoTime();
        h.stop(Duration.ofMillis(100));
        assertTrue(Duration.ofNanos(System.nanoTime() - began).toMillis() < 4000, "stop waited the grace period, not the handler");
        until(finished::get, "the handler to finish its own work");
        assertFalse(sawInterrupt.get(), "the host cancels the token and nothing else: it never interrupts a handler");
    }

    // ------------------------------------------------------------------ only OutOfMemoryError is fatal

    @Test
    void aLoggerThatOverflowsTheStackEndsNeitherTheHeartbeatNorThePoller() {
        ThrowingLogger sink = new ThrowingLogger(StackOverflowError::new).armed(true);
        WorkerHost h = host(b -> b.logger(sink).heartbeatInterval(Duration.ofMillis(100)));
        AtomicInteger ran = new AtomicInteger();
        h.registerActivity("slow", (in, ctx) -> {
            Thread.sleep(1200);
            return ran.incrementAndGet();
        });
        h.registerActivity("charge", (in, ctx) -> ran.incrementAndGet());
        fake.failNext("Heartbeat", FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE));
        fake.addLeases(lease("run-1/slow/1", b -> b.setActivityName("slow").setStepId("slow")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the slow task");
        assertTrue(fake.of("Heartbeat").size() >= 5, fake.of("Heartbeat").size() + " heartbeats");
        fake.failNext("PollTasks", FakeWorkerServer.Failure.of(Status.Code.UNAUTHENTICATED));
        until(() -> fake.of("RegisterWorker").size() == 2, "the registration after the refused poll");
        fake.addLeases(lease("run-2/charge/1", b -> b.setRunId("run-2")));
        until(() -> fake.of("CompleteStep").size() == 2, "the next task");
    }

    private static int recurse(int n) {
        return recurse(n + 1) + 1;
    }

    @Test
    void anEventHookThatReallyOverflowsTheStackIsSurvived() {
        WorkerHost h = host(b -> b.onEvent(e -> recurse(0)));
        h.registerActivity("charge", (in, ctx) -> Map.of("ok", true));
        fake.addLeases(lease("run-1/charge/1"));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the task though every event overflows the stack");
        fake.addLeases(lease("run-2/charge/1", b -> b.setRunId("run-2")));
        until(() -> fake.of("CompleteStep").size() == 2, "and the next one");
    }

    // -------------------------------------------------- a token provider that fails is repeated, privately

    @Test
    void aTokenProviderHiccupAtReportTimeDoesNotDropTheResult() {
        AtomicBoolean armed = new AtomicBoolean();
        AtomicInteger asked = new AtomicInteger();
        WorkerHost h = host(b -> b.heartbeatInterval(Duration.ofSeconds(30)).tokenProvider(() -> {
            asked.incrementAndGet();
            if (armed.compareAndSet(true, false)) {
                throw new IOException("idp down for a moment");
            }
            return "tok";
        }));
        h.registerActivity("charge", (in, ctx) -> {
            armed.set(true); // the next call that asks for a token is the report of this result
            return Map.of("computed", true);
        });
        fake.addLeases(lease("run-1/charge/1"));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the result, though the provider failed once while it was reported");
        assertEquals("{\"computed\":true}", ((Worker.CompleteStepRequest) fake.of("CompleteStep").get(0).request()).getOutputJson());
        assertEquals(0, fake.of("FailStep").size());
    }

    @Test
    void aProviderThatFailsWhilePollingIsWaitedOutAndTheSessionIsNotRegisteredAgain() {
        AtomicInteger failing = new AtomicInteger();
        WorkerHost h = host(b -> b.tokenProvider(() -> {
            if (failing.getAndDecrement() > 0) {
                throw new IOException("idp down");
            }
            return "tok";
        }));
        h.registerActivity("charge", (in, ctx) -> 1);
        h.start();
        failing.set(3);
        fake.addLeases(lease("run-1/charge/1"));
        until(() -> fake.of("CompleteStep").size() == 1, "the task once the provider recovers");
        assertEquals(1, fake.of("RegisterWorker").size(), "a provider that fails is not a session that is gone");
    }

    @Test
    void aReasonNamedLikeTheProviderFailureInAServerAnswerNeverMakesAReportRepeatable() {
        for (Status.Code code : new Status.Code[] {Status.Code.UNAUTHENTICATED, Status.Code.PERMISSION_DENIED, Status.Code.INVALID_ARGUMENT, Status.Code.ABORTED, Status.Code.FAILED_PRECONDITION}) {
            fake.reset();
            List<WorkerEvent> events = new CopyOnWriteArrayList<>();
            WorkerHost h = host(b -> b.workerName("reason-" + code).onEvent(events::add));
            h.registerActivity("charge", (in, ctx) -> 1);
            fake.failNext("CompleteStep", many(10, FakeWorkerServer.Failure.of(code, "TOKEN_PROVIDER_FAILED")));
            fake.addLeases(lease("run-1/charge/1"));
            h.start();
            until(() -> events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskDropped || e instanceof WorkerEvent.TaskStale), "the end of the report for " + code);
            sleep(150);
            assertTrue(fake.of("CompleteStep").size() <= 1, code + ": " + fake.of("CompleteStep").size() + " attempts for a refusal that only the server's words call the provider");
            h.stop(Duration.ofSeconds(1));
        }
    }

    // ------------------------------------------------------------------ pauses: cap after jitter, no early end

    @Test
    void theJitterOfAPauseNeverTakesItPastTheCap() {
        assertEquals(60_000, WorkerHost.jitteredWait(60_000, 0.999, 60_000), "the cap is applied after the jitter: 72 s before");
        assertEquals(60_000, WorkerHost.jitteredWait(Integer.MAX_VALUE, 1.0, 60_000));
        assertEquals(300, WorkerHost.jitteredWait(1000, 0.5, 300));
        assertEquals(800, WorkerHost.jitteredWait(1000, 0.0, 60_000), "-20 %");
        assertEquals(1200, WorkerHost.jitteredWait(1000, 1.0, 60_000), "+20 %");
        assertEquals(1000, WorkerHost.jitteredWait(1000, 0.5, 60_000));
    }

    @Test
    void noPollPauseIsEverLongerThanTheCapWhatEverTheJitter() {
        fake.pollRetryAfterMs = Integer.MAX_VALUE;
        WorkerHost h = host(b -> b.maxServerWait(Duration.ofMillis(200)));
        h.registerActivity("charge", (in, ctx) -> 1);
        h.start();
        until(() -> fake.of("PollTasks").size() >= 15, "fifteen polls");
        List<FakeWorkerServer.Call> polls = fake.of("PollTasks");
        long longest = 0;
        for (int i = 1; i < 15; i++) {
            longest = Math.max(longest, Duration.ofNanos(polls.get(i).nanos() - polls.get(i - 1).nanos()).toMillis());
        }
        assertTrue(longest <= 200 + 40, "the longest pause between polls was " + longest + " ms with a cap of 200 ms");
    }

    @Test
    void aBackoffAfterARefusedRegistrationIsNotCutShortByATaskThatFinishes() {
        WorkerHost h = host(b -> b.retry(new RetryPolicy(4, Duration.ofSeconds(2), Duration.ofSeconds(10))));
        CountDownLatch started = new CountDownLatch(1);
        h.registerActivity("slow", (in, ctx) -> {
            started.countDown();
            Thread.sleep(600);
            return 1;
        });
        fake.addLeases(lease("run-1/slow/1", b -> b.setActivityName("slow").setStepId("slow")));
        h.start();
        until(() -> started.getCount() == 0, "the task to start");
        fake.failNext("PollTasks", many(2, FakeWorkerServer.Failure.of(Status.Code.UNAUTHENTICATED)));
        until(() -> fake.of("RegisterWorker").size() == 2, "the first registration after a refusal (immediate)");
        long second = System.nanoTime();
        until(() -> fake.of("RegisterWorker").size() == 3, "the registration after the second refusal (backed off)");
        long waited = Duration.ofNanos(System.nanoTime() - second).toMillis();
        assertTrue(waited >= 900, "the backoff (1 to 2 s) lasted " + waited + " ms: the task that finished at 600 ms did not end it");
    }

    // ---------------------------------------------------------------- the session ends with the session

    @Test
    void theSessionTokenIsClearedWhenTheSessionEnds() throws Exception {
        WorkerHost h = host(b -> {});
        h.registerActivity("charge", (in, ctx) -> 1);
        java.lang.reflect.Field session = WorkerHost.class.getDeclaredField("session");
        java.lang.reflect.Field workerId = WorkerHost.class.getDeclaredField("workerId");
        session.setAccessible(true);
        workerId.setAccessible(true);
        assertEquals("", session.get(h));
        h.start();
        assertEquals("session-1", session.get(h), "held while the session lives");
        h.stop(Duration.ofSeconds(1));
        assertEquals("", session.get(h), "gone with the session");
        assertEquals("", workerId.get(h));
        h.start();
        assertEquals("session-1", session.get(h), "and a restart makes a new one");
    }

    @Test
    void secretsOfAnAttemptThatEndedAreRefusedAsAValidationError() {
        WorkerHost h = host(b -> {});
        AtomicReference<Secrets> kept = new AtomicReference<>();
        AtomicReference<ActivityContext> context = new AtomicReference<>();
        h.registerActivity("pay", (in, ctx) -> {
            kept.set(ctx.secrets());
            context.set(ctx);
            return 1;
        });
        fake.secrets = List.of(FakeWorkerServer.secret("db", "hunter2", 1));
        fake.addLeases(lease("run-1/pay/1", b -> b.setActivityName("pay").setStepId("pay")));
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the completion");
        assertEquals(ErrorCode.VALIDATION_FAILED, assertThrows(SagatException.class, () -> kept.get().get("db")).code());
        assertEquals(ErrorCode.VALIDATION_FAILED, assertThrows(SagatException.class, () -> kept.get().text("db")).code());
        assertEquals(ErrorCode.VALIDATION_FAILED, assertThrows(SagatException.class, () -> context.get().secrets()).code());
        // a name the step never declared is still a plain NOT_FOUND while the attempt lives: not this case
    }
}
