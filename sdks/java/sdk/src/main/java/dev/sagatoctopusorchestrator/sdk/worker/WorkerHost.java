package dev.sagatoctopusorchestrator.sdk.worker;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.NullNode;
import dev.sagatoctopusorchestrator.protocol.v1.Worker;
import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.RetryPolicy;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import dev.sagatoctopusorchestrator.sdk.SagatSdk;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.concurrent.CancellationException;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.ScheduledThreadPoolExecutor;
import java.util.concurrent.ThreadLocalRandom;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.Supplier;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * The worker host of the SDK (SPEC section 6). Register the activities, {@link #start()} it, and it polls the worker
 * service, runs each task on its own thread with a heartbeat, and reports the result. It is the work to wait for: its
 * threads are not daemons, so a process that has started a host stays alive until {@link #stop(Duration)} (or
 * {@link #close()}) is called.
 *
 * <p>Registering is for before {@code start}. The host is restartable.
 */
public final class WorkerHost implements AutoCloseable {
    private static final int MAX_OUTPUT_BYTES = 256 * 1024;
    private static final int MAX_POLL = 64;
    private static final int MAX_REPORT_ATTEMPTS = 5;
    private static final Pattern CODE = Pattern.compile("^[A-Z0-9_]{1,64}$");
    private static final Pattern VERSION = Pattern.compile("^1(?:\\.(\\d+))?$");

    private enum Kind {
        ACTIVITY,
        COMPENSATION,
        RECONCILER
    }

    private record Registered(Kind kind, ActivityHandler<Object, Object> handler, Class<?> inputType) {}

    private static final class Inflight {
        final CancellationSource cancel = new CancellationSource();
        final CompletableFuture<Void> done = new CompletableFuture<>();
    }

    private static final class LeaseState {
        volatile long deadline;
        volatile boolean lost;

        LeaseState(long deadline) {
            this.deadline = deadline;
        }
    }

    private final WorkerConfig config;
    private final System.Logger log;
    private final RetryPolicy retry;
    private final Object lifecycle = new Object();
    private final Map<String, Registered> handlers = new TreeMap<>();
    private final ConcurrentHashMap<String, Inflight> inflight = new ConcurrentHashMap<>();
    private final Object wakeLock = new Object();
    private boolean wakeFlag;

    private volatile boolean running;
    private volatile boolean stopping;
    private volatile WorkerRpc rpc;
    private volatile String workerId = "";
    private volatile String session = "";
    private volatile String accepted = "";
    private volatile Map<String, Registered> active = Map.of();
    private Thread loop;
    private ExecutorService tasks;
    private ScheduledThreadPoolExecutor beats;

    public WorkerHost(WorkerConfig config) {
        this.config = config;
        this.log = config.logger();
        this.retry = config.retry();
    }

    /** The version the server agreed to; empty before {@link #start()}. */
    public String protocolVersion() {
        return accepted;
    }

    // ------------------------------------------------------------------------------------ registration

    private WorkerHost add(Kind kind, String name, Class<?> inputType, ActivityHandler<?, ?> handler) {
        if (name == null || name.isEmpty()) {
            throw SagatException.invalid("an activity needs a name");
        }
        synchronized (lifecycle) {
            if (running) {
                throw SagatException.invalid("activities are registered before the host starts");
            }
            if (handlers.containsKey(name)) {
                throw SagatException.invalid("an activity named " + name + " is registered already");
            }
            @SuppressWarnings("unchecked")
            ActivityHandler<Object, Object> erased = (ActivityHandler<Object, Object>) handler;
            handlers.put(name, new Registered(kind, erased, inputType));
        }
        return this;
    }

    /** Registers the handler of an activity; the input arrives as a {@link JsonNode}. */
    public WorkerHost registerActivity(String name, ActivityHandler<JsonNode, ?> handler) {
        return add(Kind.ACTIVITY, name, JsonNode.class, handler);
    }

    /** Registers the handler of an activity whose input is bound to {@code inputType}; an input that cannot be is {@code INPUT_INVALID}. */
    public <I> WorkerHost registerActivity(String name, Class<I> inputType, ActivityHandler<I, ?> handler) {
        return add(Kind.ACTIVITY, name, inputType, handler);
    }

    /** Registers the handler of the activity a node names in {@code compensateWith}. It must be idempotent. */
    public WorkerHost registerCompensation(String name, ActivityHandler<JsonNode, ?> handler) {
        return add(Kind.COMPENSATION, name, JsonNode.class, handler);
    }

    public <I> WorkerHost registerCompensation(String name, Class<I> inputType, ActivityHandler<I, ?> handler) {
        return add(Kind.COMPENSATION, name, inputType, handler);
    }

    /** Registers the handler of the activity a node names in {@code reconcileWith}; it answers a {@link Verdict}. */
    public WorkerHost registerReconciler(String name, ActivityHandler<JsonNode, Verdict> handler) {
        return add(Kind.RECONCILER, name, JsonNode.class, handler);
    }

    public <I> WorkerHost registerReconciler(String name, Class<I> inputType, ActivityHandler<I, Verdict> handler) {
        return add(Kind.RECONCILER, name, inputType, handler);
    }

    // -------------------------------------------------------------------------------------- lifecycle

    /** Registers with the server and starts the loop; returns once registered. */
    public void start() {
        synchronized (lifecycle) {
            if (running) {
                return;
            }
            if (handlers.isEmpty()) {
                throw SagatException.invalid("register at least one activity before start");
            }
            if (handlers.size() > 256) {
                throw SagatException.invalid("a host serves at most 256 activities");
            }
            active = Map.copyOf(handlers);
            WorkerRpc opened = new WorkerRpc(config);
            rpc = opened;
            try {
                register();
            } catch (RuntimeException e) {
                rpc = null;
                opened.close();
                throw e;
            }
            stopping = false;
            running = true;
            synchronized (wakeLock) {
                wakeFlag = false;
            }
            String name = config.workerName();
            AtomicInteger taskCounter = new AtomicInteger();
            tasks = Executors.newCachedThreadPool(runnable -> new Thread(runnable, "sagat-worker-" + name + "-task-" + taskCounter.incrementAndGet()));
            AtomicInteger beatCounter = new AtomicInteger();
            beats = new ScheduledThreadPoolExecutor(Math.max(1, Math.min(config.maxInflight(), 8)), runnable -> {
                Thread thread = new Thread(runnable, "sagat-worker-" + name + "-beat-" + beatCounter.incrementAndGet());
                thread.setDaemon(true);
                return thread;
            });
            beats.setRemoveOnCancelPolicy(true);
            loop = new Thread(this::run, "sagat-worker-" + name + "-poller");
            loop.start();
        }
    }

    /** {@link #stop(Duration)} with 30 s. */
    public void stop() {
        stop(Duration.ofSeconds(30));
    }

    /**
     * Stops polling, waits up to {@code timeout} for the tasks in flight, then cancels the ones left (reason
     * {@code shutdown}) and returns. A task that did not finish keeps its lease until it lapses. Calling it on a
     * host that is not running does nothing.
     */
    public void stop(Duration timeout) {
        Thread loopThread;
        ExecutorService taskPool;
        ScheduledThreadPoolExecutor beatPool;
        WorkerRpc channel;
        synchronized (lifecycle) {
            if (!running) {
                return;
            }
            stopping = true;
            loopThread = loop;
            taskPool = tasks;
            beatPool = beats;
            channel = rpc;
        }
        wake();
        long end = System.nanoTime() + Math.max(0, timeout.toNanos());
        try {
            // The poll in progress ends and no other starts; what it leased is in flight from here on.
            loopThread.join(Math.max(1, TimeUnit.NANOSECONDS.toMillis(end - System.nanoTime())));
            if (loopThread.isAlive()) {
                channel.abort();
                loopThread.interrupt();
                loopThread.join(2000);
            }
            List<Inflight> pending = new ArrayList<>(inflight.values());
            boolean late = false;
            for (Inflight task : pending) {
                try {
                    task.done.get(Math.max(0, TimeUnit.NANOSECONDS.toMillis(end - System.nanoTime())), TimeUnit.MILLISECONDS);
                } catch (TimeoutException e) {
                    late = true;
                    break;
                } catch (ExecutionException | CancellationException e) {
                    // The task ended; how it ended was reported by the task itself.
                }
            }
            if (late) {
                for (Inflight task : pending) {
                    task.cancel.cancel(CancelReason.SHUTDOWN);
                }
                long grace = System.nanoTime() + TimeUnit.SECONDS.toNanos(2);
                for (Inflight task : pending) {
                    try {
                        task.done.get(Math.max(0, TimeUnit.NANOSECONDS.toMillis(grace - System.nanoTime())), TimeUnit.MILLISECONDS);
                    } catch (TimeoutException e) {
                        break;
                    } catch (ExecutionException | CancellationException e) {
                        // Ended.
                    }
                }
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        } finally {
            synchronized (lifecycle) {
                running = false;
                rpc = null;
            }
            beatPool.shutdownNow();
            taskPool.shutdown();
            if (!inflight.isEmpty()) {
                // A handler that ignored its cancellation: interrupt it, and let go of its lease.
                taskPool.shutdownNow();
            }
            channel.close();
        }
    }

    /** Same as {@link #stop()}. */
    @Override
    public void close() {
        stop();
    }

    private void emit(WorkerEvent event) {
        if (config.onEvent() == null) {
            return;
        }
        try {
            config.onEvent().accept(event);
        } catch (RuntimeException e) {
            log.log(System.Logger.Level.WARNING, "an event listener failed (" + e.getClass().getSimpleName() + ")");
        }
    }

    // ------------------------------------------------------------------------------- registration RPC

    private void register() {
        Worker.WorkerCapabilities.Builder capabilities = Worker.WorkerCapabilities.newBuilder()
                .addAllLanguages(config.languages())
                .addAllActivities(handlers.keySet())
                .addAllPools(config.pools())
                .setMaxInflight(config.maxInflight())
                .setProtocolVersion(config.protocolVersion() != null ? config.protocolVersion() : SagatSdk.PROTOCOL_VERSION);
        Worker.RegisterWorkerRequest request = Worker.RegisterWorkerRequest.newBuilder()
                .setWorkerName(config.workerName())
                .setNamespace(config.namespace())
                .setCapabilities(capabilities)
                .build();
        boolean refreshed = false;
        for (int n = 0; ; n++) {
            try {
                Worker.RegisterWorkerResponse response = rpc.registerWorker(request);
                workerId = response.getWorkerId();
                session = response.getSessionToken();
                accepted = response.getAcceptedProtocolVersion();
                log.log(System.Logger.Level.INFO, "worker registered worker=" + config.workerName() + " protocol=" + accepted);
                emit(new WorkerEvent.Registered(accepted));
                return;
            } catch (SagatException error) {
                // A token the provider can refresh gets one more chance (SPEC section 5).
                if (error.code() == ErrorCode.UNAUTHENTICATED && !refreshed && config.tokenProvider().refreshable()) {
                    refreshed = true;
                    continue;
                }
                if (!error.isRetriable() || n + 1 >= retry.maxAttempts()) {
                    throw error;
                }
                sleep(retry.delayAfterMillis(n, error, random()));
            }
        }
    }

    private static java.util.function.DoubleSupplier random() {
        return () -> ThreadLocalRandom.current().nextDouble();
    }

    private static void sleep(long millis) {
        try {
            Thread.sleep(millis);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw SagatException.builder(ErrorCode.CANCELLED, "interrupted while waiting").cause(e).build();
        }
    }

    // ---------------------------------------------------------------------------------------- the loop

    /** Wakes the pause between polls: a task finished, or the host is stopping. */
    private void wake() {
        synchronized (wakeLock) {
            wakeFlag = true;
            wakeLock.notifyAll();
        }
    }

    // The pause between polls. A host that is stopping does not pause: `stop` may have run while a poll was in
    // flight, with nothing left to wake.
    private void idle(long millis) {
        long end = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(millis);
        synchronized (wakeLock) {
            try {
                while (!wakeFlag && !stopping) {
                    long left = end - System.nanoTime();
                    if (left <= 0) {
                        break;
                    }
                    TimeUnit.NANOSECONDS.timedWait(wakeLock, left);
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            } finally {
                wakeFlag = false;
            }
        }
    }

    private void run() {
        int failures = 0;
        while (!stopping) {
            try {
                int free = config.maxInflight() - inflight.size();
                if (free <= 0) {
                    idle(50);
                    continue;
                }
                Worker.PollTasksResponse response = rpc.pollTasks(Worker.PollTasksRequest.newBuilder()
                        .setWorkerId(workerId)
                        .setSessionToken(session)
                        .setMaxTasks(Math.min(free, MAX_POLL))
                        .build());
                failures = 0;
                emit(new WorkerEvent.Poll(response.getTasksCount()));
                for (Worker.TaskLease lease : response.getTasksList()) {
                    dispatch(lease);
                }
                if (response.getTasksCount() == 0) {
                    long wait = Math.max(50, response.getRetryAfterMs() > 0 ? response.getRetryAfterMs() : 1000);
                    idle(Math.round(wait * (0.8 + ThreadLocalRandom.current().nextDouble() * 0.4)));
                }
            } catch (SagatException error) {
                if (stopping) {
                    return;
                }
                if (error.code() == ErrorCode.UNAUTHENTICATED) {
                    log.log(System.Logger.Level.WARNING, "the session is gone: registering again worker=" + config.workerName());
                    emit(new WorkerEvent.Reconnect());
                    try {
                        register();
                        continue;
                    } catch (SagatException again) {
                        log.log(System.Logger.Level.ERROR, "registering again failed code=" + again.code());
                    }
                }
                if (error.code() == ErrorCode.RATE_LIMITED) {
                    idle(Math.max(error.retryAfterMs().orElse(0L), 100));
                    continue;
                }
                emit(new WorkerEvent.Reconnect());
                RetryPolicy slow = retry.withMaxDelay(Duration.ofSeconds(30));
                idle(Math.min(30_000, Math.max(slow.backoffMillis(failures++, random()), error.retryAfterMs().orElse(0L))));
            } catch (RuntimeException error) {
                if (stopping) {
                    return;
                }
                log.log(System.Logger.Level.ERROR, "the poll loop failed (" + error.getClass().getSimpleName() + ")");
                idle(1000);
            }
        }
    }

    // ----------------------------------------------------------------------------------------- tasks

    private void dispatch(Worker.TaskLease lease) {
        WorkerRpc channel = rpc;
        Inflight task = new Inflight();
        if (inflight.putIfAbsent(lease.getTaskId(), task) != null) {
            log.log(System.Logger.Level.WARNING, "a task that is in flight was leased again, ignored task=" + lease.getTaskId());
            return;
        }
        emit(new WorkerEvent.TaskTaken(lease.getTaskId(), lease.getActivityName(), lease.getAttempt()));
        try {
            tasks.execute(() -> {
                try {
                    execute(channel, lease, task);
                } catch (Throwable t) {
                    log.log(System.Logger.Level.ERROR, "a task failed outside its handler task=" + lease.getTaskId() + " (" + t.getClass().getSimpleName() + ")");
                } finally {
                    inflight.remove(lease.getTaskId());
                    task.done.complete(null);
                    wake();
                }
            });
        } catch (RejectedExecutionException e) {
            inflight.remove(lease.getTaskId());
            task.done.complete(null);
        }
    }

    /** One third of the time left on the lease (SPEC section 6.4), never under 500 ms and never over 10 s. */
    static long heartbeatIntervalMillis(long expiresAtEpochMs, long nowEpochMs) {
        long left = Math.max(0, expiresAtEpochMs - nowEpochMs);
        return Math.min(10_000, Math.max(500, Math.round(left / 3.0)));
    }

    private static int minorOf(String version) {
        Matcher match = VERSION.matcher(version);
        if (!match.matches()) {
            return 0;
        }
        return match.group(1) == null ? 0 : Integer.parseInt(match.group(1));
    }

    private static boolean isCancellation(Throwable error) {
        return (error instanceof SagatException e && e.code() == ErrorCode.CANCELLED)
                || error instanceof CancellationException
                || error instanceof InterruptedException;
    }

    private static String typeName(Throwable error) {
        String name = error.getClass().getSimpleName();
        return name.isEmpty() ? error.getClass().getName() : name;
    }

    private void execute(WorkerRpc channel, Worker.TaskLease lease, Inflight task) {
        long started = System.nanoTime();
        Registered registered = active.get(lease.getActivityName());
        LeaseState state = new LeaseState(lease.getExpiresAtEpochMs());
        TaskKind kind = TaskKind.of(lease.getTaskKind());
        int minor = minorOf(accepted);

        if (registered == null) {
            fail(channel, lease, state, "NO_HANDLER", "no handler is registered for the activity", false, started);
            return;
        }

        JsonNode document;
        try {
            String text = lease.getInputJson();
            JsonNode parsed = text.isEmpty() ? null : Json.mapper().readTree(text);
            document = parsed != null && parsed.isObject() ? parsed : JsonNodeFactory.instance.objectNode();
        } catch (java.io.IOException e) {
            fail(channel, lease, state, "INPUT_INVALID", "the step document is not JSON", false, started);
            return;
        }
        JsonNode rawInput = document.has("input") ? document.get("input") : NullNode.getInstance();
        Object input;
        try {
            input = bind(rawInput, registered.inputType());
        } catch (Exception e) {
            fail(channel, lease, state, "INPUT_INVALID", "the input does not have the type of the handler", false, started);
            return;
        }
        Map<String, JsonNode> steps = new LinkedHashMap<>();
        document.path("steps").properties().forEach(entry -> {
            JsonNode value = entry.getValue();
            if (value != null && value.has("output")) {
                steps.put(entry.getKey(), value.get("output"));
            }
        });

        // The heartbeat starts before the handler and runs until the report is made (SPEC section 6.3).
        Beat beat = new Beat(channel, lease, minor, task.cancel, state);
        beat.start();
        try {
            String node = document.path("node").isTextual() && !document.path("node").textValue().isEmpty()
                    ? document.path("node").textValue()
                    : lease.getStepId();
            Context context = new Context(channel, lease, node, kind, Collections.unmodifiableMap(steps), state, task.cancel, minor);

            Object result = null;
            ActivityFailure failure = null;
            boolean unreported = false;
            try {
                result = registered.handler().handle(input, context);
            } catch (ActivityFailure e) {
                failure = e;
            } catch (Throwable e) {
                if (isCancellation(e) && task.cancel.isCancelled()) {
                    // Cancelled by the run: a failure for good. Cancelled because the host is stopping: nothing is
                    // reported, the lease lapses and the step is leased again. Lease lost: reported below as nothing.
                    if (task.cancel.reason().orElse(null) == CancelReason.SHUTDOWN) {
                        unreported = true;
                    } else {
                        failure = ActivityFailure.permanent("CANCELLED", "the activity was cancelled");
                    }
                } else {
                    failure = ActivityFailure.retryable("ACTIVITY_ERROR", typeName(e));
                }
            } finally {
                context.dropSecrets();
            }

            if (state.lost) {
                emit(new WorkerEvent.TaskStale(lease.getTaskId()));
                log.log(System.Logger.Level.WARNING, "the lease was lost: the result is dropped task=" + lease.getTaskId());
                return;
            }
            if (unreported) {
                emit(new WorkerEvent.TaskDropped(lease.getTaskId(), "shutdown"));
                log.log(System.Logger.Level.INFO, "the host is stopping: the cancelled task is left to its lease task=" + lease.getTaskId());
                return;
            }
            if (failure != null) {
                fail(channel, lease, state, failure.code(), failure.getMessage(), failure.isRetryable(), started);
                return;
            }

            String output;
            if (kind == TaskKind.RECONCILIATION && registered.kind() == Kind.RECONCILER) {
                if (!(result instanceof Verdict verdict)) {
                    fail(channel, lease, state, "RECONCILIATION_ANSWER_INVALID", "a reconciler answers applied, notApplied or unknown", false, started);
                    return;
                }
                output = Json.encode(verdict.toJson());
            } else if (result == ActivityHandler.NO_RESULT) {
                output = "";
            } else {
                try {
                    output = Json.encode(result instanceof Verdict verdict ? verdict.toJson() : Json.toTree(result));
                } catch (SagatException e) {
                    fail(channel, lease, state, "OUTPUT_INVALID", e.getMessage(), false, started);
                    return;
                }
            }
            if (output.getBytes(StandardCharsets.UTF_8).length > MAX_OUTPUT_BYTES) {
                fail(channel, lease, state, "PAYLOAD_TOO_LARGE", "the output is over 256 KiB", false, started);
                return;
            }
            complete(channel, lease, state, output, started);
        } finally {
            beat.stop();
        }
    }

    private static Object bind(JsonNode node, Class<?> type) throws java.io.IOException {
        if (type == null || type == JsonNode.class) {
            return node;
        }
        return Json.mapper().treeToValue(node, type);
    }

    // --------------------------------------------------------------------------------------- context

    private final class Context implements ActivityContext {
        private final WorkerRpc channel;
        private final Worker.TaskLease lease;
        private final String node;
        private final TaskKind kind;
        private final Map<String, JsonNode> steps;
        private final LeaseState state;
        private final CancellationSource cancel;
        private final int minor;
        private final ActivityLogger logger;
        private Secrets secrets;
        private boolean dropped;

        Context(WorkerRpc channel, Worker.TaskLease lease, String node, TaskKind kind, Map<String, JsonNode> steps, LeaseState state, CancellationSource cancel, int minor) {
            this.channel = channel;
            this.lease = lease;
            this.node = node;
            this.kind = kind;
            this.steps = steps;
            this.state = state;
            this.cancel = cancel;
            this.minor = minor;
            this.logger = (level, message) -> log.log(level, "task=" + lease.getTaskId() + " " + message);
        }

        @Override
        public String activityName() {
            return lease.getActivityName();
        }

        @Override
        public String taskId() {
            return lease.getTaskId();
        }

        @Override
        public String runId() {
            return lease.getRunId();
        }

        @Override
        public String node() {
            return node;
        }

        @Override
        public String stepId() {
            return lease.getStepId();
        }

        @Override
        public int attempt() {
            return lease.getAttempt();
        }

        @Override
        public long generation() {
            return lease.getGeneration();
        }

        @Override
        public String idempotencyKey() {
            return lease.getIdempotencyKey();
        }

        @Override
        public String traceparent() {
            return lease.getTraceparent();
        }

        @Override
        public TaskKind taskKind() {
            return kind;
        }

        @Override
        public String targetNode() {
            return lease.getTargetNode();
        }

        @Override
        public String targetIdempotencyKey() {
            return lease.getTargetIdempotencyKey();
        }

        @Override
        public Map<String, JsonNode> steps() {
            return steps;
        }

        @Override
        public Instant deadline() {
            return Instant.ofEpochMilli(state.deadline);
        }

        @Override
        public CancellationSignal cancellation() {
            return cancel;
        }

        @Override
        public ActivityLogger logger() {
            return logger;
        }

        @Override
        public synchronized Secrets secrets() {
            if (dropped) {
                throw SagatException.of(ErrorCode.NOT_FOUND, "the secrets of this attempt were dropped");
            }
            if (secrets == null) {
                secrets = resolveSecrets(channel, lease, minor);
            }
            return secrets;
        }

        synchronized void dropSecrets() {
            dropped = true;
            if (secrets != null) {
                secrets.drop();
            }
        }
    }

    private Secrets resolveSecrets(WorkerRpc channel, Worker.TaskLease lease, int minor) {
        if (minor < 2) {
            throw SagatException.of(ErrorCode.UNSUPPORTED_PROTOCOL, "the server speaks a protocol older than 1.2, which has no ResolveSecrets");
        }
        Worker.ResolveSecretsResponse answer = channel.resolveSecrets(
                Worker.ResolveSecretsRequest.newBuilder()
                        .setTaskId(lease.getTaskId())
                        .setRunId(lease.getRunId())
                        .setStepId(lease.getStepId())
                        .setAttempt(lease.getAttempt())
                        .setGeneration(lease.getGeneration())
                        .setLeaseToken(lease.getLeaseToken())
                        .setTraceparent(lease.getTraceparent())
                        .build(),
                lease.getTraceparent());
        Map<String, byte[]> values = new LinkedHashMap<>();
        for (Worker.ResolvedSecret secret : answer.getSecretsList()) {
            values.put(secret.getName(), secret.getValue().toByteArray());
        }
        return new Secrets(values);
    }

    // ------------------------------------------------------------------------------------- heartbeat

    private final class Beat {
        private final WorkerRpc channel;
        private final Worker.TaskLease lease;
        private final int minor;
        private final CancellationSource cancel;
        private final LeaseState state;
        private volatile boolean stopped;
        private volatile ScheduledFuture<?> next;

        Beat(WorkerRpc channel, Worker.TaskLease lease, int minor, CancellationSource cancel, LeaseState state) {
            this.channel = channel;
            this.lease = lease;
            this.minor = minor;
            this.cancel = cancel;
            this.state = state;
        }

        void start() {
            schedule(interval());
        }

        // One third of the time left on the lease (its expiry minus the SDK's clock now: the lease carries no TTL),
        // clamped to 0.5 to 10 s. The wait itself is then counted by the scheduler, on the monotonic clock.
        private long interval() {
            if (config.heartbeatInterval() != null) {
                return Math.max(1, config.heartbeatInterval().toMillis());
            }
            return heartbeatIntervalMillis(state.deadline, System.currentTimeMillis());
        }

        private void schedule(long millis) {
            if (stopped) {
                return;
            }
            try {
                next = beats.schedule(this::tick, millis, TimeUnit.MILLISECONDS);
            } catch (RejectedExecutionException e) {
                // The host is shutting down.
            }
        }

        private void tick() {
            if (stopped) {
                return;
            }
            try {
                Worker.HeartbeatRequest.Builder request = Worker.HeartbeatRequest.newBuilder()
                        .setTaskId(lease.getTaskId())
                        .setWorkerId(workerId)
                        .setGeneration(lease.getGeneration())
                        .setLeaseToken(lease.getLeaseToken());
                if (minor >= 1) {
                    // The step identity came with 1.1.
                    request.setRunId(lease.getRunId()).setStepId(lease.getStepId()).setAttempt(lease.getAttempt());
                }
                Worker.HeartbeatResponse answer = channel.heartbeat(request.build(), lease.getTraceparent());
                emit(new WorkerEvent.Heartbeat(lease.getTaskId()));
                if (answer.getLeaseExpiresAtEpochMs() > 0) {
                    state.deadline = answer.getLeaseExpiresAtEpochMs();
                }
                if (answer.getCancelRequested()) {
                    cancel.cancel(CancelReason.RUN_CANCELLED);
                }
            } catch (SagatException error) {
                if (error.code() == ErrorCode.STALE_ATTEMPT || error.code() == ErrorCode.CONFLICT) {
                    state.lost = true;
                    cancel.cancel(CancelReason.LEASE_LOST);
                    return;
                }
                // Anything else: the next beat tries again; the lease is ours until it runs out.
                log.log(System.Logger.Level.WARNING, "a heartbeat failed task=" + lease.getTaskId() + " code=" + error.code());
            } catch (RuntimeException error) {
                log.log(System.Logger.Level.WARNING, "a heartbeat failed task=" + lease.getTaskId() + " (" + error.getClass().getSimpleName() + ")");
            }
            if (!stopped) {
                schedule(interval());
            }
        }

        void stop() {
            stopped = true;
            ScheduledFuture<?> pending = next;
            if (pending != null) {
                pending.cancel(false);
            }
        }
    }

    // ----------------------------------------------------------------------------------- reporting

    /** Repeats a report up to five times with the same key and body; null means the attempt is stale (do not report). */
    private <T> T withRetries(WorkerRpc channel, LeaseState state, Supplier<T> call) {
        for (int n = 0; ; n++) {
            if (state.lost) {
                // The heartbeat found out the lease is gone while the report was being repeated: stop.
                return null;
            }
            if (channel.isClosed()) {
                throw SagatException.of(ErrorCode.CANCELLED, "the host stopped before the report was made");
            }
            try {
                return call.get();
            } catch (SagatException error) {
                if (error.code() == ErrorCode.STALE_ATTEMPT) {
                    return null;
                }
                if (error.isRetriable() && n + 1 < MAX_REPORT_ATTEMPTS) {
                    sleep(retry.delayAfterMillis(n, error, random()));
                    continue;
                }
                throw error;
            }
        }
    }

    private static long millisSince(long started) {
        return TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - started);
    }

    private void complete(WorkerRpc channel, Worker.TaskLease lease, LeaseState state, String output, long started) {
        try {
            Worker.CompleteStepRequest request = Worker.CompleteStepRequest.newBuilder()
                    .setTaskId(lease.getTaskId())
                    .setRunId(lease.getRunId())
                    .setStepId(lease.getStepId())
                    .setAttempt(lease.getAttempt())
                    .setGeneration(lease.getGeneration())
                    .setLeaseToken(lease.getLeaseToken())
                    .setIdempotencyKey("done-" + lease.getTaskId())
                    .setOutputJson(output)
                    .setTraceparent(lease.getTraceparent())
                    .build();
            Worker.StepCommandResponse answer = withRetries(channel, state, () -> channel.completeStep(request, lease.getTraceparent()));
            if (answer == null) {
                emit(new WorkerEvent.TaskStale(lease.getTaskId()));
                return;
            }
            emit(new WorkerEvent.TaskCompleted(lease.getTaskId(), Duration.ofMillis(millisSince(started))));
        } catch (SagatException error) {
            emit(new WorkerEvent.TaskDropped(lease.getTaskId(), error.code().name()));
            log.log(System.Logger.Level.ERROR, "the completion could not be reported task=" + lease.getTaskId() + " code=" + error.code());
        }
    }

    private void fail(WorkerRpc channel, Worker.TaskLease lease, LeaseState state, String code, String message, boolean retryable, long started) {
        String safeCode = CODE.matcher(code).matches() ? code : "WORKER_ERROR";
        try {
            Worker.FailStepRequest request = Worker.FailStepRequest.newBuilder()
                    .setTaskId(lease.getTaskId())
                    .setRunId(lease.getRunId())
                    .setStepId(lease.getStepId())
                    .setAttempt(lease.getAttempt())
                    .setGeneration(lease.getGeneration())
                    .setLeaseToken(lease.getLeaseToken())
                    .setIdempotencyKey("failed-" + lease.getTaskId())
                    .setErrorCode(safeCode)
                    .setSafeMessage(cut(message))
                    .setRetryable(retryable)
                    .setTraceparent(lease.getTraceparent())
                    .build();
            Worker.StepCommandResponse answer = withRetries(channel, state, () -> channel.failStep(request, lease.getTraceparent()));
            if (answer == null) {
                emit(new WorkerEvent.TaskStale(lease.getTaskId()));
                return;
            }
            emit(new WorkerEvent.TaskFailed(lease.getTaskId(), safeCode, retryable, Duration.ofMillis(millisSince(started))));
        } catch (SagatException error) {
            emit(new WorkerEvent.TaskDropped(lease.getTaskId(), error.code().name()));
            log.log(System.Logger.Level.ERROR, "the failure could not be reported task=" + lease.getTaskId() + " code=" + error.code());
        }
    }

    /** The first 512 characters, without cutting a surrogate pair in half. */
    private static String cut(String message) {
        if (message == null) {
            return "";
        }
        if (message.length() <= 512) {
            return message;
        }
        int end = Character.isHighSurrogate(message.charAt(511)) ? 511 : 512;
        return message.substring(0, end);
    }
}
