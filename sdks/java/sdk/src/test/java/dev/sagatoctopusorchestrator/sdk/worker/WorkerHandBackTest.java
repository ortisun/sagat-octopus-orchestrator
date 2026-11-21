package dev.sagatoctopusorchestrator.sdk.worker;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import dev.sagatoctopusorchestrator.protocol.v1.Worker;
import dev.sagatoctopusorchestrator.sdk.CapturingLogger;
import dev.sagatoctopusorchestrator.sdk.RetryPolicy;
import io.grpc.Status;
import java.io.IOException;
import java.time.Duration;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.BooleanSupplier;
import java.util.function.Consumer;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;

/**
 * The hand-back of SPEC section 6.2: a lease the host received and will not run is reported with FailStep
 * TASK_NOT_STARTED, before any handler of it runs, once, and never for a task whose handler started (mirrors
 * handback_test.go of the Go SDK and adds the lease the task pool refuses).
 */
@Timeout(60)
class WorkerHandBackTest {
    private static final String TRACE = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    private static FakeWorkerServer fake;
    private final List<WorkerHost> hosts = new ArrayList<>();
    private final CountDownLatch released = new CountDownLatch(1);

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
        // A handler that a failed test left waiting is let go, so that no thread outlives the test.
        released.countDown();
        while (!hosts.isEmpty()) {
            hosts.remove(hosts.size() - 1).stop(Duration.ofSeconds(1));
        }
        fake.reset();
    }

    private static Worker.TaskLease numbered(int n, Consumer<Worker.TaskLease.Builder> tweak) {
        Worker.TaskLease.Builder builder = Worker.TaskLease.newBuilder()
                .setTaskId("run-" + n + "/charge/1")
                .setRunId("run-" + n)
                .setStepId("charge")
                .setAttempt(1)
                .setGeneration(1)
                .setLeaseToken("LEASE-TOKEN-" + n)
                .setExpiresAtEpochMs(System.currentTimeMillis() + 60_000)
                .setActivityName("charge")
                .setInputJson("{\"run\":{\"id\":\"run-" + n + "\"},\"node\":\"charge\",\"input\":{}}")
                .setTraceparent(TRACE)
                .setIdempotencyKey("run-" + n + ":charge");
        tweak.accept(builder);
        return builder.build();
    }

    private static Worker.TaskLease numbered(int n) {
        return numbered(n, b -> {});
    }

    private WorkerHost host(Consumer<WorkerConfig.Builder> tweak) {
        WorkerConfig.Builder builder = WorkerConfig.builder()
                .grpcUrl(fake.url())
                .namespace("payments")
                .token("tok")
                .workerName("hand-back-worker")
                .retry(new RetryPolicy(4, Duration.ofMillis(5), Duration.ofMillis(20)));
        tweak.accept(builder);
        WorkerHost created = new WorkerHost(builder.build());
        hosts.add(created);
        return created;
    }

    private static void until(BooleanSupplier condition, String what) {
        until(condition, what, Duration.ofSeconds(10));
    }

    private static void until(BooleanSupplier condition, String what, Duration patience) {
        long end = System.nanoTime() + patience.toNanos();
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

    private static void await(CountDownLatch latch, String what) {
        try {
            if (!latch.await(10, TimeUnit.SECONDS)) {
                throw new AssertionError("timed out waiting for " + what);
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new AssertionError(e);
        }
    }

    /** A handler that waits until the test lets it go (or the test ends), then answers 1. */
    private Object held() throws InterruptedException {
        released.await(10, TimeUnit.SECONDS);
        return 1;
    }

    /**
     * The task ids of the FailStep calls, sorted, after checking that each is the whole hand-back: the code
     * TASK_NOT_STARTED, retryable, the fixed message, the failure's idempotency key and the lease's fence and identity.
     */
    private static List<String> handedBack() {
        List<String> ids = new ArrayList<>();
        for (FakeWorkerServer.Call call : fake.of("FailStep")) {
            Worker.FailStepRequest request = (Worker.FailStepRequest) call.request();
            String n = request.getRunId().substring("run-".length());
            assertEquals("TASK_NOT_STARTED", request.getErrorCode(), "every failure here is a hand-back: " + request);
            assertTrue(request.getRetryable(), "a hand-back is retryable");
            assertEquals("the host did not start this task", request.getSafeMessage());
            assertEquals("failed-" + request.getTaskId(), request.getIdempotencyKey());
            assertEquals("run-" + n + "/charge/1", request.getTaskId());
            assertEquals("charge", request.getStepId());
            assertEquals(1, request.getAttempt());
            assertEquals(1, request.getGeneration());
            assertEquals("LEASE-TOKEN-" + n, request.getLeaseToken(), "the fence of the lease itself");
            assertEquals(TRACE, request.getTraceparent());
            ids.add(request.getTaskId());
        }
        ids.sort(null);
        return ids;
    }

    private static long beatsOf(String task) {
        return fake.of("Heartbeat").stream().filter(c -> ((Worker.HeartbeatRequest) c.request()).getTaskId().equals(task)).count();
    }

    private static CompletableFuture<Void> stopInBackground(WorkerHost host) {
        return CompletableFuture.runAsync(() -> host.stop(Duration.ofSeconds(5)));
    }

    @Test
    void aPollAnswerBeyondTheCreditIsHandedBack() {
        fake.ignoreMax = true;
        fake.addLeases(numbered(1), numbered(2), numbered(3));
        CapturingLogger log = new CapturingLogger();
        List<String> ran = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.maxInflight(1).logger(log));
        h.registerActivity("charge", (in, ctx) -> {
            ran.add(ctx.taskId());
            return 1;
        });
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the kept task to complete");
        h.stop(Duration.ofSeconds(5));
        assertEquals(List.of("run-2/charge/1", "run-3/charge/1"), handedBack(), "the two leases beyond the credit");
        assertEquals(List.of("run-1/charge/1"), ran, "only the first lease runs");
        assertEquals(0, beatsOf("run-2/charge/1") + beatsOf("run-3/charge/1"), "a lease that is handed back is never beaten for");
        assertEquals(1, log.warnings("task=run-2/charge/1 reason=credit").size(), "a warning names the task and the reason");
        assertTrue(log.lines().stream().noneMatch(l -> l.message().contains("LEASE-TOKEN")), "never the lease token in a log line");
    }

    @Test
    void aStreamFrameBeyondTheCreditIsHandedBackAndTheStartedTaskIsNot() {
        fake.acceptedVersion = "1.4";
        fake.ignoreMax = true;
        fake.holdStreamOpen = true;
        fake.addLeases(numbered(1), numbered(2), numbered(3));
        WorkerHost h = host(b -> b.maxInflight(1).heartbeatInterval(Duration.ofMillis(20)));
        h.registerActivity("charge", (in, ctx) -> held());
        h.start();
        until(() -> fake.of("FailStep").size() == 2, "the two hand-backs");
        until(() -> beatsOf("run-1/charge/1") > 0, "the started task to beat");
        released.countDown();
        until(() -> fake.of("CompleteStep").size() == 1, "the kept task to complete");
        h.stop(Duration.ofSeconds(5));
        assertEquals(List.of("run-2/charge/1", "run-3/charge/1"), handedBack());
        assertEquals(2, fake.of("FailStep").size(), "the started task is never handed back");
        assertEquals(0, beatsOf("run-2/charge/1") + beatsOf("run-3/charge/1"), "a lease that is handed back is never beaten for");
    }

    @Test
    void theRestOfAFrameOverTheInputBudgetIsHandedBack() {
        fake.acceptedVersion = "1.4";
        fake.holdStreamOpen = true;
        String big = "x".repeat(5 * 1024 * 1024);
        Consumer<Worker.TaskLease.Builder> heavy = b -> b.setInputJson("{\"run\":{\"id\":\"run-1\"},\"node\":\"charge\",\"input\":\"" + big + "\"}");
        fake.addLeases(numbered(1, heavy), numbered(2, heavy), numbered(3));
        CapturingLogger log = new CapturingLogger();
        WorkerHost h = host(b -> b.maxInflight(3).logger(log));
        h.registerActivity("charge", (in, ctx) -> 1);
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the kept task to complete");
        until(() -> fake.of("FailStep").size() == 2, "the hand-backs");
        h.stop(Duration.ofSeconds(5));
        assertEquals(List.of("run-2/charge/1", "run-3/charge/1"), handedBack(), "the lease over the budget and the rest of the frame");
        assertEquals(1, log.warnings("task=run-2/charge/1 reason=malformed").size());
    }

    @Test
    void aStreamFrameReadWhileStoppingIsHandedBackWhole() {
        fake.acceptedVersion = "1.4";
        fake.holdStreamOpen = true;
        fake.addLeases(numbered(1), numbered(2));
        CapturingLogger log = new CapturingLogger();
        WorkerHost h = host(b -> b.maxInflight(4).logger(log));
        CountDownLatch read = new CountDownLatch(1);
        CountDownLatch proceed = new CountDownLatch(1);
        h.frameReceived = leases -> {
            read.countDown();
            await(proceed, "the test to let the loop go on");
        };
        AtomicInteger ran = new AtomicInteger();
        h.registerActivity("charge", (in, ctx) -> ran.incrementAndGet());
        h.start();
        await(read, "the frame to be read");
        CompletableFuture<Void> stopped = stopInBackground(h);
        until(h::isStopping, "the stop to begin");
        proceed.countDown();
        stopped.join();
        // Stop returned: the hand-backs were made before the channel closed.
        assertEquals(List.of("run-1/charge/1", "run-2/charge/1"), handedBack(), "the whole frame");
        assertEquals(0, ran.get(), "a host that is stopping starts nothing new");
        assertEquals(1, log.warnings("task=run-1/charge/1 reason=stopping").size());
    }

    @Test
    void aPollAnswerThatArrivesWhileStoppingIsHandedBackAndTheRunningTaskIsNot() {
        fake.addLeases(numbered(1));
        WorkerHost h = host(b -> b.maxInflight(4));
        CountDownLatch started = new CountDownLatch(1);
        h.registerActivity("charge", (in, ctx) -> {
            started.countDown();
            return held();
        });
        // The second poll answer that carries leases is held in the loop until the stop has begun.
        CountDownLatch holding = new CountDownLatch(1);
        CountDownLatch hold = new CountDownLatch(1);
        AtomicInteger answers = new AtomicInteger();
        h.frameReceived = leases -> {
            if (leases > 0 && answers.incrementAndGet() == 2) {
                holding.countDown();
                await(hold, "the test to let the loop go on");
            }
        };
        h.start();
        await(started, "the first task to start");
        fake.addLeases(numbered(2), numbered(3));
        await(holding, "the second answer to be read");
        CompletableFuture<Void> stopped = stopInBackground(h);
        until(h::isStopping, "the stop to begin");
        hold.countDown();
        until(() -> fake.of("FailStep").size() == 2, "the hand-backs");
        released.countDown();
        stopped.join();
        assertEquals(List.of("run-2/charge/1", "run-3/charge/1"), handedBack());
        assertEquals(1, fake.of("CompleteStep").size(), "the running task reports its own result");
        assertEquals("run-1/charge/1", ((Worker.CompleteStepRequest) fake.of("CompleteStep").get(0).request()).getTaskId());
    }

    @Test
    void aDuplicateLeaseIsIgnoredNeverHandedBack() {
        fake.acceptedVersion = "1.4";
        fake.ignoreMax = true;
        fake.holdStreamOpen = true;
        // The second is the running task's own lease again; the fourth repeats a lease handed back.
        fake.addLeases(numbered(1), numbered(1), numbered(2), numbered(2));
        CapturingLogger log = new CapturingLogger();
        WorkerHost h = host(b -> b.maxInflight(1).logger(log));
        h.registerActivity("charge", (in, ctx) -> held());
        h.start();
        until(() -> fake.of("FailStep").size() >= 1, "the hand-back");
        released.countDown();
        until(() -> fake.of("CompleteStep").size() == 1, "the kept task to complete");
        h.stop(Duration.ofSeconds(5));
        assertEquals(List.of("run-2/charge/1"), handedBack(), "a duplicate is never reported, a lease is handed back once");
        assertEquals(2, log.warnings("ignored").size(), "each duplicate is logged");
    }

    private static Worker.TaskLease sibling(int index) {
        return numbered(1, b -> b.setInstance(Worker.ExecutionInstanceIdentity.newBuilder()
                .setDefinitionNodeId("charge")
                .addInstancePath(Worker.ScopePathSegment.newBuilder().setScopeNodeId("each").setIterationIndex(index))));
    }

    @Test
    void siblingIterationsThatShareATaskIdAreNotDuplicates() {
        fake.acceptedVersion = "1.5";
        fake.holdStreamOpen = true;
        fake.addLeases(sibling(0), sibling(1));
        WorkerHost h = host(b -> b.maxInflight(2));
        h.registerActivity("charge", (in, ctx) -> 1);
        h.start();
        until(() -> fake.of("CompleteStep").size() == 2, "both iterations to complete");
        h.stop(Duration.ofSeconds(5));
        assertEquals(0, fake.of("FailStep").size(), "sibling iterations run, neither is handed back");
    }

    @Test
    void aSiblingIterationBeyondTheCreditIsHandedBackWithItsInstanceNotIgnored() {
        fake.acceptedVersion = "1.5";
        fake.ignoreMax = true;
        fake.holdStreamOpen = true;
        fake.addLeases(sibling(0), sibling(1));
        WorkerHost h = host(b -> b.maxInflight(1));
        h.registerActivity("charge", (in, ctx) -> held());
        h.start();
        until(() -> fake.of("FailStep").size() == 1, "the hand-back of the second iteration");
        released.countDown();
        until(() -> fake.of("CompleteStep").size() == 1, "the first iteration to complete");
        h.stop(Duration.ofSeconds(5));
        assertEquals(List.of("run-1/charge/1"), handedBack());
        Worker.FailStepRequest request = (Worker.FailStepRequest) fake.of("FailStep").get(0).request();
        assertEquals(sibling(1).getInstance(), request.getInstance(), "the hand-back names the iteration it gives back");
        Worker.CompleteStepRequest completed = (Worker.CompleteStepRequest) fake.of("CompleteStep").get(0).request();
        assertEquals(sibling(0).getInstance(), completed.getInstance());
    }

    @Test
    void aHandBackIsRetriedWithTheSameKeyAndEndsQuietlyWhenStale() {
        fake.ignoreMax = true;
        fake.addLeases(numbered(1), numbered(2), numbered(3));
        fake.failNext("FailStep", FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE), FakeWorkerServer.Failure.of(Status.Code.ABORTED, "STALE_ATTEMPT"));
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.maxInflight(1).onEvent(e -> {
            if (e instanceof WorkerEvent.TaskFailed || e instanceof WorkerEvent.TaskStale || e instanceof WorkerEvent.TaskDropped) {
                events.add(e);
            }
        }));
        h.registerActivity("charge", (in, ctx) -> 1);
        h.start();
        until(() -> events.size() == 2, "the outcome of both hand-backs");
        h.stop(Duration.ofSeconds(5));
        Map<String, Integer> keys = new HashMap<>();
        for (FakeWorkerServer.Call call : fake.of("FailStep")) {
            keys.merge(((Worker.FailStepRequest) call.request()).getIdempotencyKey(), 1, Integer::sum);
        }
        // Three calls for two leases: one answered UNAVAILABLE and repeated with its key, one stale.
        assertEquals(3, fake.of("FailStep").size(), "hand-back calls with keys " + keys);
        assertEquals(2, keys.size(), "hand-back calls with keys " + keys);
        handedBack();
        long failed = events.stream().filter(e -> e instanceof WorkerEvent.TaskFailed f && f.code().equals("TASK_NOT_STARTED") && f.retryable()).count();
        long stale = events.stream().filter(e -> e instanceof WorkerEvent.TaskStale).count();
        assertEquals(1, failed, "one hand-back counted as a failure with its code: " + events);
        assertEquals(1, stale, "one stale: " + events);
    }

    @Test
    void aLeaseTheTaskPoolRefusesIsHandedBackWithoutATaskTakenEvent() {
        fake.addLeases(numbered(1), numbered(2));
        CapturingLogger log = new CapturingLogger();
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.maxInflight(2).logger(log).onEvent(events::add));
        h.beforeTaskStarts = lease -> {
            if (lease.getTaskId().equals("run-2/charge/1")) {
                throw new RejectedExecutionException("refused by the test");
            }
        };
        h.registerActivity("charge", (in, ctx) -> 1);
        h.start();
        until(() -> fake.of("CompleteStep").size() == 1, "the started task to complete");
        until(() -> fake.of("FailStep").size() == 1, "the hand-back");
        h.stop(Duration.ofSeconds(5));
        assertEquals(List.of("run-2/charge/1"), handedBack());
        assertEquals(1, log.warnings("task=run-2/charge/1 reason=not_started").size());
        assertFalse(events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskTaken t && t.taskId().equals("run-2/charge/1")),
                "a lease that never started was never taken");
        assertTrue(events.stream().anyMatch(e -> e instanceof WorkerEvent.TaskFailed f && f.taskId().equals("run-2/charge/1")
                && f.code().equals("TASK_NOT_STARTED")));
    }

    @Test
    void aFloodOfLeasesIsHandedBackAtMostEightAtATime() {
        fake.ignoreMax = true;
        Worker.TaskLease[] flood = new Worker.TaskLease[10_000];
        for (int i = 0; i < flood.length; i++) {
            flood[i] = numbered(i + 1);
        }
        fake.addLeases(flood);
        CountDownLatch gate = new CountDownLatch(1);
        fake.failStepGate = gate;
        List<String> ran = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.maxInflight(1).logger(new CapturingLogger()));
        h.registerActivity("charge", (in, ctx) -> {
            ran.add(ctx.taskId());
            return 1;
        });
        h.start();
        try {
            until(() -> fake.failStepsArrived.get() >= 8, "eight hand-backs held at the service");
            // While the eight are held, no ninth may arrive: this window only looks for an absence.
            long end = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(300);
            while (System.nanoTime() < end && fake.failStepsArrived.get() <= 8) {
                Thread.onSpinWait();
            }
            assertEquals(8, fake.failStepsArrived.get(), "at most eight hand-backs on their way at once");
        } finally {
            gate.countDown();
        }
        until(() -> fake.of("FailStep").size() == 9_999, "every lease beyond the first to be handed back", Duration.ofSeconds(120));
        h.stop(Duration.ofSeconds(30));
        assertTrue(fake.failStepsPeak.get() <= 8, "peak of " + fake.failStepsPeak.get() + " hand-backs at once");
        List<String> ids = handedBack();
        assertEquals(9_999, ids.size());
        assertEquals(9_999, new java.util.HashSet<>(ids).size(), "each lease is handed back once");
        assertFalse(ids.contains("run-1/charge/1"), "never the task that started");
        assertEquals(List.of("run-1/charge/1"), ran);
    }

    @Test
    void aLeaseLeasedAgainWhileItsHandBackIsOnItsWayIsIgnored() {
        fake.ignoreMax = true;
        fake.addLeases(numbered(1), numbered(2));
        CountDownLatch gate = new CountDownLatch(1);
        fake.failStepGate = gate;
        CapturingLogger log = new CapturingLogger();
        List<String> ran = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.maxInflight(1).logger(log));
        h.registerActivity("charge", (in, ctx) -> {
            ran.add(ctx.taskId());
            return 1;
        });
        // The number of polls when the answer that leases run-2 again is read: the next poll begins after it was looked at.
        AtomicInteger answers = new AtomicInteger();
        AtomicInteger pollsAtRepeat = new AtomicInteger(-1);
        h.frameReceived = leases -> {
            if (leases > 0 && answers.incrementAndGet() == 2) {
                pollsAtRepeat.set(fake.of("PollTasks").size());
            }
        };
        h.start();
        try {
            until(() -> fake.failStepsArrived.get() == 1, "the hand-back of run-2 held at the service");
            until(() -> fake.of("CompleteStep").size() == 1, "run-1 to complete");
            fake.addLeases(numbered(2));
            until(() -> pollsAtRepeat.get() >= 0 && fake.of("PollTasks").size() > pollsAtRepeat.get(),
                    "the answer that leases run-2 again to be looked at");
            assertEquals(List.of("run-1/charge/1"), ran, "a lease whose hand-back is on its way is in flight: never run");
            assertEquals(1, log.warnings("ignored task=run-2/charge/1").size(), "the duplicate is logged");
        } finally {
            gate.countDown();
        }
        h.stop(Duration.ofSeconds(5));
        assertEquals(List.of("run-2/charge/1"), handedBack(), "exactly one hand-back for it");
        assertEquals(List.of("run-1/charge/1"), ran);
    }

    @Test
    void aHandBackAfterTheStopClosedTheChannelIsDroppedNotThrown() throws Exception {
        fake.addLeases(numbered(1), numbered(2));
        CapturingLogger log = new CapturingLogger();
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        List<String> ran = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.maxInflight(2).logger(log).onEvent(events::add));
        h.registerActivity("charge", (in, ctx) -> {
            ran.add(ctx.taskId());
            return 1;
        });
        // The loop is held, with the answer of two leases in hand, on a latch that an interrupt does not end: stop's
        // interrupt and its two-second join leave it alive, and it goes on after stop returned.
        CountDownLatch holding = new CountDownLatch(1);
        CountDownLatch release = new CountDownLatch(1);
        h.frameReceived = leases -> {
            if (leases > 0 && holding.getCount() > 0) {
                holding.countDown();
                boolean interrupted = false;
                while (true) {
                    try {
                        release.await();
                        break;
                    } catch (InterruptedException e) {
                        interrupted = true;
                    }
                }
                if (interrupted) {
                    Thread.currentThread().interrupt();
                }
            }
        };
        h.start();
        await(holding, "the answer to be read");
        h.stop(Duration.ofMillis(100));
        release.countDown();
        until(() -> events.stream().filter(e -> e instanceof WorkerEvent.TaskDropped).count() == 2, "both leases to be dropped");
        List<String> dropped = events.stream()
                .filter(e -> e instanceof WorkerEvent.TaskDropped d && d.reason().equals("CANCELLED"))
                .map(e -> ((WorkerEvent.TaskDropped) e).taskId())
                .sorted()
                .toList();
        assertEquals(List.of("run-1/charge/1", "run-2/charge/1"), dropped, "each lease ends as dropped, CANCELLED");
        assertEquals(1, log.warnings("the hand-back could not be made: the host has stopped task=run-1/charge/1").size());
        assertEquals(1, log.warnings("the hand-back could not be made: the host has stopped task=run-2/charge/1").size());
        assertTrue(log.lines().stream().noneMatch(l -> l.message().contains("a hand-back failed") || l.message().contains("the poll loop failed")),
                "nothing is thrown: " + log.lines());
        assertEquals(0, fake.failStepsArrived.get(), "no FailStep reaches the service");
        assertEquals(List.of(), ran, "no handler ran");
    }

    @Test
    void handBacksStillOnTheirWayWhenTheGraceEndsAreDroppedOnceAndStopIsBounded() {
        fake.ignoreMax = true;
        for (int n = 1; n <= 13; n++) {
            fake.addLeases(numbered(n));
        }
        // The first hand-back is answered UNAVAILABLE and waits five to ten seconds before it repeats, past the stop; the
        // next seven are held at the service; the last four wait their turn at the host.
        fake.failNext("FailStep", FakeWorkerServer.Failure.of(Status.Code.UNAVAILABLE));
        CountDownLatch gate = new CountDownLatch(1);
        fake.failStepGate = gate;
        CapturingLogger log = new CapturingLogger();
        List<WorkerEvent> events = new CopyOnWriteArrayList<>();
        WorkerHost h = host(b -> b.maxInflight(1).logger(log).onEvent(events::add)
                .retry(new RetryPolicy(4, Duration.ofSeconds(10), Duration.ofSeconds(10))));
        h.registerActivity("charge", (in, ctx) -> 1);
        h.start();
        try {
            until(() -> fake.failStepsArrived.get() == 8, "one hand-back refused and seven held at the service");
            until(() -> fake.of("CompleteStep").size() == 1, "the kept task to complete");
            long began = System.nanoTime();
            h.stop(Duration.ofMillis(200));
            long took = Duration.ofNanos(System.nanoTime() - began).toMillis();
            // The grace, the two seconds stop gives after it, and the two seconds it waits for the channel to close.
            assertTrue(took < 200 + 2_000 + 2_000, "stop took " + took + " ms");
            until(() -> events.stream().filter(e -> e instanceof WorkerEvent.TaskDropped).count() == 12, "every hand-back to be dropped",
                    Duration.ofSeconds(30));
        } finally {
            gate.countDown();
        }
        // The calls held at the service end before this test does, so that none is recorded among the next test's calls.
        until(() -> fake.failStepsOnTheirWay.get() == 0, "the calls held at the service to end");
        Map<String, Integer> ends = new HashMap<>();
        for (WorkerEvent e : events) {
            if (e instanceof WorkerEvent.TaskDropped d) {
                ends.merge(d.taskId(), 1, Integer::sum);
            } else if (e instanceof WorkerEvent.TaskFailed f) {
                ends.merge(f.taskId(), 1, Integer::sum);
            } else if (e instanceof WorkerEvent.TaskStale s) {
                ends.merge(s.taskId(), 1, Integer::sum);
            }
        }
        assertEquals(12, ends.size(), "every lease beyond the first ends: " + ends);
        assertTrue(ends.values().stream().allMatch(n -> n == 1), "each ends once: " + ends);
        assertTrue(events.stream().noneMatch(e -> e instanceof WorkerEvent.TaskFailed), "none is reported after the stop");
        assertEquals(8, fake.failStepsArrived.get(), "the queued ones never reach the service");
        assertTrue(log.lines().stream().noneMatch(l -> l.message().contains("a hand-back failed") || l.message().contains("the poll loop failed")),
                "nothing is thrown: " + log.lines());
    }

    @Test
    void aFrameAfterTheCreditIsUsedUpIsReadAndHandedBack() {
        fake.acceptedVersion = "1.4";
        fake.holdStreamOpen = true;
        // One receive window: the first frame uses up the credit of one, the second carries two more.
        fake.streamFrames = List.of(List.of(numbered(1)), List.of(numbered(2), numbered(3)));
        WorkerHost h = host(b -> b.maxInflight(1));
        h.registerActivity("charge", (in, ctx) -> held());
        h.start();
        until(() -> fake.of("FailStep").size() == 2, "the two hand-backs");
        assertEquals(0, fake.of("CompleteStep").size(), "run-1 is still held");
        released.countDown();
        until(() -> fake.of("CompleteStep").size() == 1, "the kept task to complete");
        h.stop(Duration.ofSeconds(5));
        assertEquals(List.of("run-2/charge/1", "run-3/charge/1"), handedBack(), "the frame after the credit is read and handed back");
        assertEquals("run-1/charge/1", ((Worker.CompleteStepRequest) fake.of("CompleteStep").get(0).request()).getTaskId());
    }
}
