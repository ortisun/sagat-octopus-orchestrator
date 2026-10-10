using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.Internal;

namespace SagatOctopusOrchestrator.Sdk.Workers;

/// <summary>The worker host of the SDK (SPEC section 6).</summary>
public sealed partial class WorkerHost : IWorkerHost
{
    private const int MaxOutputBytes = 256 * 1024;
    private const int MaxPoll = 64;
    private const int ReportAttempts = 5;
    private const int MaxMessage = 512;

    private readonly SagatOptions options;
    private readonly ILoggerFactory loggerFactory;
    private readonly ILogger logger;
    private readonly JsonSerializerOptions json;
    private readonly int maxInflight;
    private readonly Dictionary<string, ActivityRegistration> handlers = new(StringComparer.Ordinal);
    private readonly ConcurrentDictionary<string, InFlight> inflight = new(StringComparer.Ordinal);
    private readonly SemaphoreSlim lifecycle = new(1, 1);
    private readonly object wakeGate = new();
    private readonly TagList tags;

    private Session? current;
    private string workerId = string.Empty;
    private string sessionToken = string.Empty;
    private string accepted = string.Empty;
    private int minor;
    private volatile bool running;
    private volatile bool stopping;
    private Task loop = Task.CompletedTask;
    private CancellationTokenSource? loopCancel;
    private TaskCompletionSource? wake;

    /// <summary>Creates a host. <see cref="SagatOptions.GrpcUrl"/>, <see cref="SagatOptions.Namespace"/>, a token and <see cref="WorkerOptions.Name"/> are required to start it.</summary>
    public WorkerHost(SagatOptions options, ILoggerFactory? loggerFactory = null)
    {
        ArgumentNullException.ThrowIfNull(options);
        this.options = options.Snapshot();
        maxInflight = this.options.Worker.MaxInflight;
        if (maxInflight is < 1 or > 1000)
        {
            throw SagatException.Invalid("MaxInflight is between 1 and 1000");
        }

        this.loggerFactory = loggerFactory ?? NullLoggerFactory.Instance;
        logger = this.loggerFactory.CreateLogger<WorkerHost>();
        json = Wire.OptionsOf(this.options);
        tags = new TagList { { "sagat.worker", this.options.Worker.Name ?? string.Empty } };
    }

    /// <inheritdoc/>
    public string ProtocolVersion => accepted;

    /// <inheritdoc/>
    public bool IsRunning => running;

    [GeneratedRegex("^[A-Z0-9_]{1,64}$", RegexOptions.CultureInvariant)]
    private static partial Regex CodePattern();

    [GeneratedRegex("^1(?:\\.(\\d+))?$", RegexOptions.CultureInvariant)]
    private static partial Regex VersionPattern();

    // ------------------------------------------------------------------------------------ registration

    /// <inheritdoc/>
    public IWorkerHost RegisterActivity<TInput, TOutput>(string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task<TOutput>> handler) =>
        Add(name, new ValueRegistration<TInput, TOutput>(RegistrationKind.Activity, Required(handler)));

    /// <inheritdoc/>
    public IWorkerHost RegisterActivity<TInput>(string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task> handler) =>
        Add(name, new VoidRegistration<TInput>(RegistrationKind.Activity, Required(handler)));

    /// <inheritdoc/>
    public IWorkerHost RegisterCompensation<TInput, TOutput>(string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task<TOutput>> handler) =>
        Add(name, new ValueRegistration<TInput, TOutput>(RegistrationKind.Compensation, Required(handler)));

    /// <inheritdoc/>
    public IWorkerHost RegisterCompensation<TInput>(string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task> handler) =>
        Add(name, new VoidRegistration<TInput>(RegistrationKind.Compensation, Required(handler)));

    /// <inheritdoc/>
    public IWorkerHost RegisterReconciler<TInput>(string name, Func<TInput, ActivityExecutionContext, CancellationToken, Task<Verdict>> handler) =>
        Add(name, new ReconcilerRegistration<TInput>(Required(handler)));

    private static T Required<T>(T handler)
        where T : class =>
        handler ?? throw new ArgumentNullException(nameof(handler));

    private WorkerHost Add(string name, ActivityRegistration registration)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(name);
        lock (handlers)
        {
            if (running)
            {
                throw SagatException.Invalid("activities are registered before the host starts");
            }

            if (!handlers.TryAdd(name, registration))
            {
                throw SagatException.Invalid($"an activity named {name} is registered already");
            }
        }

        return this;
    }

    // ----------------------------------------------------------------------------------------- lifecycle

    /// <inheritdoc/>
    public async Task StartAsync(CancellationToken cancellationToken = default)
    {
        await lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            if (running)
            {
                return;
            }

            Validate();
            var channel = new WorkerRpc(options, logger);
            try
            {
                await RegisterAsync(channel, cancellationToken).ConfigureAwait(false);
            }
            catch
            {
                channel.Dispose();
                throw;
            }

            var started = new Session(channel);
            current = started;
            stopping = false;
            loopCancel = new CancellationTokenSource();
            running = true;
            var token = loopCancel.Token;

            // The loop lives as long as the host: it must not carry the caller's ambient trace or request.
            using (ExecutionContext.SuppressFlow())
            {
                loop = Task.Run(() => RunLoopAsync(started, token), CancellationToken.None);
            }
        }
        finally
        {
            lifecycle.Release();
        }
    }

    /// <inheritdoc/>
    public async Task StopAsync(TimeSpan? timeout = null, CancellationToken cancellationToken = default)
    {
        await lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            if (!running)
            {
                return;
            }

            stopping = true;
            Wake();

            // The poll in progress ends and no other starts; what it leased is in flight from here on. Cancelling
            // the token ends the wait for that poll at once (the host is going away).
            using (cancellationToken.Register(static state => ((CancellationTokenSource)state!).Cancel(), loopCancel))
            {
                await loop.ConfigureAwait(false);
            }

            var pending = Task.WhenAll(inflight.Values.Select(task => task.Completion.Task));
            var late = false;
            try
            {
                await pending.WaitAsync(timeout ?? options.Worker.ShutdownTimeout, cancellationToken).ConfigureAwait(false);
            }
            catch (Exception error) when (error is TimeoutException or OperationCanceledException)
            {
                late = true;
            }

            if (late)
            {
                Log.ShutdownCancelled(logger);
                foreach (var task in inflight.Values)
                {
                    task.Runtime.Cancel(CancellationReasons.Shutdown);
                }

                try
                {
                    await pending.WaitAsync(TimeSpan.FromSeconds(2), CancellationToken.None).ConfigureAwait(false);
                }
                catch (TimeoutException)
                {
                    // The handler ignored its token: the lease lapses and the server leases the step again.
                }
            }

            running = false;
            current?.Close();
            current = null;
        }
        finally
        {
            lifecycle.Release();
        }
    }

    /// <inheritdoc/>
    public async ValueTask DisposeAsync()
    {
        await StopAsync().ConfigureAwait(false);
        lifecycle.Dispose();
    }

    private void Validate()
    {
        options.ValidateCommon();
        if (string.IsNullOrWhiteSpace(options.Worker.Name))
        {
            throw SagatException.Invalid("Worker.Name is required");
        }

        if (string.IsNullOrWhiteSpace(options.GrpcUrl) || !Uri.TryCreate(options.GrpcUrl, UriKind.Absolute, out var url) || url.Scheme is not ("http" or "https"))
        {
            throw SagatException.Invalid("GrpcUrl is required and is an http or https URL");
        }

        lock (handlers)
        {
            if (handlers.Count == 0)
            {
                throw SagatException.Invalid("register at least one activity before start");
            }

            if (handlers.Count > 256)
            {
                throw SagatException.Invalid("a worker serves 1 to 256 activities");
            }
        }
    }

    // ------------------------------------------------------------------------------------- registration

    private async Task RegisterAsync(WorkerRpc channel, CancellationToken cancellationToken)
    {
        string[] names;
        lock (handlers)
        {
            names = [.. handlers.Keys.Order(StringComparer.Ordinal)];
        }

        var request = new RegisterWorkerRequest
        {
            WorkerName = options.Worker.Name,
            Namespace = options.Namespace,
            Capabilities = new WorkerCapabilities
            {
                MaxInflight = maxInflight,
                ProtocolVersion = options.Worker.ProtocolVersion ?? SdkInfo.ProtocolVersion,
            },
        };
        request.Capabilities.Languages.AddRange(options.Worker.Languages);
        request.Capabilities.Activities.AddRange(names);
        request.Capabilities.Pools.AddRange(options.Worker.Pools);

        var refreshed = false;
        for (var n = 0; ; n++)
        {
            try
            {
                var response = await channel.RegisterAsync(request, cancellationToken).ConfigureAwait(false);
                workerId = response.WorkerId;
                sessionToken = response.SessionToken;
                accepted = response.AcceptedProtocolVersion;
                var match = VersionPattern().Match(accepted);
                minor = match.Success ? (match.Groups[1].Success ? int.Parse(match.Groups[1].Value, System.Globalization.CultureInfo.InvariantCulture) : 0) : 0;
                Log.WorkerRegistered(logger, options.Worker.Name!, accepted);
                return;
            }
            catch (SagatException error)
            {
                // A token the provider can refresh gets one more chance (SPEC section 5).
                if (error.Code == SagatErrorCode.Unauthenticated && !refreshed && options.TokenProvider is not null)
                {
                    refreshed = true;
                    continue;
                }

                if (!error.Retryable || n + 1 >= options.Retry.MaxAttempts)
                {
                    throw;
                }

                await Task.Delay(options.Retry.DelayAfter(n, error), cancellationToken).ConfigureAwait(false);
            }
        }
    }

    // ------------------------------------------------------------------------------------------- loop

    private void Wake()
    {
        TaskCompletionSource? waiting;
        lock (wakeGate)
        {
            waiting = wake;
        }

        waiting?.TrySetResult();
    }

    // The pause between polls. A host that is stopping does not pause: `StopAsync` may have run while a poll was in
    // flight, with nothing to wake. A task that finishes wakes the loop (there is room again).
    private async Task IdleAsync(TimeSpan wait)
    {
        var waiting = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        lock (wakeGate)
        {
            wake = waiting;
        }

        try
        {
            if (stopping)
            {
                return;
            }

            using var timer = new CancellationTokenSource();
            var delay = Task.Delay(wait, timer.Token);
            await Task.WhenAny(delay, waiting.Task).ConfigureAwait(false);
            await timer.CancelAsync().ConfigureAwait(false);
        }
        finally
        {
            lock (wakeGate)
            {
                if (ReferenceEquals(wake, waiting))
                {
                    wake = null;
                }
            }
        }
    }

    private static TimeSpan Jitter(TimeSpan wait) =>
        TimeSpan.FromMilliseconds(Math.Round(wait.TotalMilliseconds * (0.8 + (Random.Shared.NextDouble() * 0.4))));

    private async Task RunLoopAsync(Session session, CancellationToken cancellationToken)
    {
        var failures = 0;
        var channel = session.Rpc;
        while (!stopping)
        {
            try
            {
                var free = maxInflight - inflight.Count;
                if (free <= 0)
                {
                    await IdleAsync(TimeSpan.FromMilliseconds(50)).ConfigureAwait(false);
                    continue;
                }

                var response = await channel.PollAsync(
                    new PollTasksRequest { WorkerId = workerId, SessionToken = sessionToken, MaxTasks = Math.Min(free, MaxPoll) },
                    cancellationToken).ConfigureAwait(false);
                failures = 0;
                Telemetry.Polls.Add(1, tags);
                foreach (var lease in response.Tasks)
                {
                    Dispatch(lease, session);
                }

                if (response.Tasks.Count == 0)
                {
                    var wait = Math.Max(50, response.RetryAfterMs > 0 ? response.RetryAfterMs : 1000);
                    await IdleAsync(Jitter(TimeSpan.FromMilliseconds(wait))).ConfigureAwait(false);
                }
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                return;
            }
            catch (SagatException error)
            {
                if (stopping)
                {
                    return;
                }

                if (error.Code == SagatErrorCode.Unauthenticated)
                {
                    Log.SessionGone(logger);
                    Telemetry.Reconnects.Add(1, tags);
                    try
                    {
                        await RegisterAsync(channel, cancellationToken).ConfigureAwait(false);
                        continue;
                    }
                    catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
                    {
                        return;
                    }
                    catch (SagatException again)
                    {
                        Log.RegisteringAgainFailed(logger, again.Code.ToWireName());
                    }
                }

                if (error.Code == SagatErrorCode.RateLimited)
                {
                    await IdleAsync(Max(error.RetryAfter, TimeSpan.FromMilliseconds(100))).ConfigureAwait(false);
                    continue;
                }

                Telemetry.Reconnects.Add(1, tags);
                var backoff = new RetryPolicy { InitialDelay = options.Retry.InitialDelay, MaxDelay = TimeSpan.FromSeconds(30) }.Backoff(failures++);
                var pause = Min(TimeSpan.FromSeconds(30), Max(error.RetryAfter, backoff));
                Log.PollFailed(logger, error.Code.ToWireName(), (long)pause.TotalMilliseconds);
                await IdleAsync(pause).ConfigureAwait(false);
            }
#pragma warning disable CA1031 // The loop must outlive a bug in anything it calls: it logs the type and goes on.
            catch (Exception error)
#pragma warning restore CA1031
            {
                if (stopping)
                {
                    return;
                }

                Log.PollLoopFailed(logger, error.GetType().Name);
                await IdleAsync(TimeSpan.FromSeconds(1)).ConfigureAwait(false);
            }
        }
    }

    /// <summary>
    /// The heartbeat interval for a lease with <paramref name="timeLeft"/> on it when the task is taken (the lease's expiry minus the
    /// SDK's clock at receipt: the lease carries no TTL): a third of it, never under 500 ms and never over 10 s. A clock skewed by more
    /// than the lease's TTL distorts it, which is why it is clamped; a host whose clock is not synchronized sets
    /// <see cref="WorkerOptions.HeartbeatInterval"/>.
    /// </summary>
    public static TimeSpan HeartbeatIntervalFor(TimeSpan timeLeft)
    {
        var third = TimeSpan.FromTicks(timeLeft.Ticks / 3);
        return third < TimeSpan.FromMilliseconds(500) ? TimeSpan.FromMilliseconds(500) : Min(third, TimeSpan.FromSeconds(10));
    }

    private static TimeSpan Max(TimeSpan? a, TimeSpan b) => a is { } value && value > b ? value : b;

    private static TimeSpan Min(TimeSpan a, TimeSpan b) => a < b ? a : b;

    // ------------------------------------------------------------------------------------------- tasks

    private void Dispatch(TaskLease lease, Session session)
    {
        var entry = new InFlight(new TaskRuntime(lease.ExpiresAtEpochMs));
        inflight[lease.TaskId] = entry;
        Telemetry.TasksTaken.Add(1, tags);
        Log.TaskTaken(logger, lease.TaskId, lease.ActivityName, lease.Attempt);
        _ = Task.Run(async () =>
        {
            try
            {
                await ExecuteAsync(lease, entry.Runtime, session).ConfigureAwait(false);
            }
#pragma warning disable CA1031 // A bug in one task must not take the host down.
            catch (Exception error)
#pragma warning restore CA1031
            {
                Log.TaskCrashed(logger, lease.TaskId, error.GetType().Name);
            }
            finally
            {
                inflight.TryRemove(lease.TaskId, out _);
                entry.Completion.TrySetResult();
                Wake();
            }
        });
    }

    private async Task ExecuteAsync(TaskLease lease, TaskRuntime runtime, Session session)
    {
        var started = Stopwatch.GetTimestamp();
        ActivityRegistration? registered;
        lock (handlers)
        {
            handlers.TryGetValue(lease.ActivityName, out registered);
        }

        var kind = lease.TaskKind switch
        {
            "COMPENSATION" => TaskKind.Compensation,
            "RECONCILIATION" => TaskKind.Reconciliation,
            _ => TaskKind.Forward,
        };

        using var span = StartSpan(lease, kind);
        using var scope = logger.BeginScope(new Dictionary<string, object?>
        {
            ["sagat.task_id"] = lease.TaskId,
            ["sagat.run_id"] = lease.RunId,
            ["sagat.attempt"] = lease.Attempt,
        });

        if (registered is null)
        {
            await FailAsync(session, lease, "NO_HANDLER", "no handler is registered for the activity", retryable: false, started, span).ConfigureAwait(false);
            return;
        }

        JsonDocument? document = null;
        try
        {
            try
            {
                document = lease.InputJson.Length == 0 ? null : JsonDocument.Parse(lease.InputJson);
            }
            catch (JsonException)
            {
                await FailAsync(session, lease, "INPUT_INVALID", "the step document is not JSON", retryable: false, started, span).ConfigureAwait(false);
                return;
            }

            var root = document is { RootElement.ValueKind: JsonValueKind.Object } ? document.RootElement : default;
            var hasRoot = root.ValueKind == JsonValueKind.Object;
            JsonElement? input = hasRoot && root.TryGetProperty("input", out var inputElement) ? inputElement : null;
            var node = hasRoot && root.TryGetProperty("node", out var nodeElement) && nodeElement.ValueKind == JsonValueKind.String && nodeElement.GetString() is { Length: > 0 } nodeId
                ? nodeId
                : lease.StepId;

            object? typed;
            try
            {
                typed = registered.ParseInput(input, json);
            }
#pragma warning disable CA1031 // Whatever the deserializer raises, the input does not have the type of the handler.
            catch (Exception)
#pragma warning restore CA1031
            {
                await FailAsync(session, lease, "INPUT_INVALID", "the input does not have the type of the handler", retryable: false, started, span).ConfigureAwait(false);
                return;
            }

            var steps = new Dictionary<string, JsonElement>(StringComparer.Ordinal);
            if (hasRoot && root.TryGetProperty("steps", out var stepsElement) && stepsElement.ValueKind == JsonValueKind.Object)
            {
                foreach (var step in stepsElement.EnumerateObject())
                {
                    if (step.Value.ValueKind == JsonValueKind.Object && step.Value.TryGetProperty("output", out var output))
                    {
                        steps[step.Name] = output.Clone();
                    }
                }
            }

            using var secretsLoader = new SecretsLoader(this, lease, session);
            var context = new ActivityExecutionContext(
                runtime,
                lease.ActivityName,
                lease.TaskId,
                lease.RunId,
                node,
                lease.StepId,
                lease.Attempt,
                lease.Generation,
                lease.IdempotencyKey,
                lease.Traceparent,
                kind,
                lease.TargetNode,
                lease.TargetIdempotencyKey,
                steps,
                loggerFactory.CreateLogger("SagatOctopusOrchestrator.Activity." + lease.ActivityName),
                secretsLoader.GetAsync);

            await RunAndReportAsync(session, lease, runtime, registered, typed, context, started, span).ConfigureAwait(false);
        }
        finally
        {
            document?.Dispose();
        }
    }

    /// <summary>
    /// Runs the handler and reports what it did. The heartbeat starts before the handler and runs until the report is made, not just
    /// until the handler ends: a report that is slow or retried does not lose the lease.
    /// </summary>
    private async Task RunAndReportAsync(
        Session session,
        TaskLease lease,
        TaskRuntime runtime,
        ActivityRegistration registered,
        object? typed,
        ActivityExecutionContext context,
        long started,
        Activity? span)
    {
        using var stopBeat = new CancellationTokenSource();
        var heartbeat = BeatAsync(session, lease, runtime, stopBeat.Token);
        try
        {
            ActivityResult? result = null;
            ActivityFailure? failure = null;
            try
            {
                result = await registered.InvokeAsync(typed, context, runtime.Abort.Token).ConfigureAwait(false);
            }
            catch (ActivityFailure failed)
            {
                failure = failed;
            }
            catch (Exception error) when (IsCancellation(error) && runtime.Abort.IsCancellationRequested)
            {
                if (runtime.Reason == CancellationReasons.Shutdown)
                {
                    // The host is going away and the handler said it stopped: the step is not failed, its lease lapses and the
                    // server leases it again (SPEC section 6.4).
                    Telemetry.TasksDropped.Add(1, tags);
                    Log.ShutdownAbandoned(logger, lease.TaskId);
                    span?.SetTag("sagat.outcome", "shutdown");
                    return;
                }

                failure = ActivityFailure.Permanent("CANCELLED", "the activity was cancelled");
            }
#pragma warning disable CA1031 // An activity may raise anything: the report says only the type.
            catch (Exception error)
#pragma warning restore CA1031
            {
                failure = ActivityFailure.Retryable("ACTIVITY_ERROR", TypeNameOf(error));
            }

            if (runtime.Lost)
            {
                Telemetry.TasksStale.Add(1, tags);
                Log.LeaseLost(logger, lease.TaskId);
                span?.SetTag("sagat.outcome", "lease-lost");
                return;
            }

            if (failure is not null)
            {
                await FailAsync(session, lease, failure.Code, failure.Message, failure.IsRetryable, started, span).ConfigureAwait(false);
                return;
            }

            if (result is VerdictResult { Verdict: null })
            {
                await FailAsync(session, lease, "RECONCILIATION_ANSWER_INVALID", "a reconciler answers applied, notApplied or unknown", retryable: false, started, span).ConfigureAwait(false);
                return;
            }

            string output;
            try
            {
                output = result!.ToJson(json) ?? string.Empty;
            }
            catch (SagatException error)
            {
                await FailAsync(session, lease, "OUTPUT_INVALID", error.Message, retryable: false, started, span).ConfigureAwait(false);
                return;
            }

            if (Encoding.UTF8.GetByteCount(output) > MaxOutputBytes)
            {
                await FailAsync(session, lease, "PAYLOAD_TOO_LARGE", "the output is over 256 KiB", retryable: false, started, span).ConfigureAwait(false);
                return;
            }

            await CompleteAsync(session, lease, output, started, span).ConfigureAwait(false);
        }
        finally
        {
            await stopBeat.CancelAsync().ConfigureAwait(false);
            await heartbeat.ConfigureAwait(false);
        }
    }

    private static bool IsCancellation(Exception error) =>
        error is OperationCanceledException || (error is SagatException { Code: SagatErrorCode.Cancelled });

    private static string TypeNameOf(Exception error)
    {
        if (error is AggregateException { InnerExceptions.Count: 1 } aggregate)
        {
            error = aggregate.InnerExceptions[0];
        }

        return error.GetType().Name;
    }

    private Activity? StartSpan(TaskLease lease, TaskKind kind)
    {
        var name = "sagat activity " + lease.ActivityName;
        Activity? span;
        if (lease.Traceparent.Length > 0 && System.Diagnostics.ActivityContext.TryParse(lease.Traceparent, null, out var parent))
        {
            span = Telemetry.Source.StartActivity(name, ActivityKind.Consumer, parent);
        }
        else
        {
            span = Telemetry.Source.StartActivity(name, ActivityKind.Consumer, parentContext: default(System.Diagnostics.ActivityContext));
        }

        if (span is not null)
        {
            span.SetTag("sagat.worker", options.Worker.Name);
            span.SetTag("sagat.namespace", options.Namespace);
            span.SetTag("sagat.activity", lease.ActivityName);
            span.SetTag("sagat.task.id", lease.TaskId);
            span.SetTag("sagat.run.id", lease.RunId);
            span.SetTag("sagat.step.id", lease.StepId);
            span.SetTag("sagat.attempt", lease.Attempt);
            span.SetTag("sagat.task.kind", kind.ToString());
        }

        return span;
    }

    // ----------------------------------------------------------------------------------- heartbeat

    private Task BeatAsync(Session session, TaskLease lease, TaskRuntime runtime, CancellationToken stop)
    {
        var channel = session.Rpc;
        var identity = minor >= 1;

        var interval = options.Worker.HeartbeatInterval ?? runtime.HeartbeatInterval;

        return Task.Run(
            async () =>
            {
                while (!stop.IsCancellationRequested)
                {
                    try
                    {
                        await Task.Delay(interval, stop).ConfigureAwait(false);
                        var answer = await channel.HeartbeatAsync(
                            new HeartbeatRequest
                            {
                                TaskId = lease.TaskId,
                                WorkerId = workerId,
                                Generation = lease.Generation,
                                LeaseToken = lease.LeaseToken,

                                // The step identity came with 1.1.
                                RunId = identity ? lease.RunId : string.Empty,
                                StepId = identity ? lease.StepId : string.Empty,
                                Attempt = identity ? lease.Attempt : 0,
                            },
                            lease.Traceparent.Length > 0 ? lease.Traceparent : null,
                            stop).ConfigureAwait(false);
                        Telemetry.Heartbeats.Add(1, tags);
                        if (answer.LeaseExpiresAtEpochMs > 0)
                        {
                            runtime.SetLease(answer.LeaseExpiresAtEpochMs);
                        }

                        if (answer.CancelRequested && !runtime.Abort.IsCancellationRequested)
                        {
                            runtime.Cancel(CancellationReasons.RunCancelled);
                        }
                    }
                    catch (OperationCanceledException) when (stop.IsCancellationRequested)
                    {
                        return;
                    }
                    catch (SagatException error) when (error.Code is SagatErrorCode.StaleAttempt or SagatErrorCode.Conflict)
                    {
                        runtime.MarkLost();
                        runtime.Cancel(CancellationReasons.LeaseLost);
                        return;
                    }
                    catch (SagatException error)
                    {
                        // Anything else: the next beat tries again; the lease is ours until it runs out.
                        Log.HeartbeatFailed(logger, lease.TaskId, error.Code.ToWireName());
                    }
                }
            },
            CancellationToken.None);
    }

    // ------------------------------------------------------------------------------------ reporting

    /// <summary>Runs a report with the retries of SPEC section 5; false when the server says the lease is gone.</summary>
    private async Task<bool> WithRetriesAsync(Session session, Func<CancellationToken, Task> call)
    {
        var token = session.ReportToken;
        for (var n = 0; ; n++)
        {
            try
            {
                await call(token).ConfigureAwait(false);
                return true;
            }
            catch (SagatException error)
            {
                if (error.Code == SagatErrorCode.StaleAttempt)
                {
                    return false;
                }

                if (error.Retryable && n + 1 < ReportAttempts)
                {
                    await Task.Delay(options.Retry.DelayAfter(n, error), token).ConfigureAwait(false);
                    continue;
                }

                throw;
            }
        }
    }

    private async Task CompleteAsync(Session session, TaskLease lease, string output, long started, Activity? span)
    {
        var channel = session.Rpc;
        try
        {
            var reported = await WithRetriesAsync(session, token => channel.CompleteStepAsync(
                new CompleteStepRequest
                {
                    TaskId = lease.TaskId,
                    RunId = lease.RunId,
                    StepId = lease.StepId,
                    Attempt = lease.Attempt,
                    Generation = lease.Generation,
                    LeaseToken = lease.LeaseToken,
                    IdempotencyKey = "done-" + lease.TaskId,
                    OutputJson = output,
                    OutputBlobRef = string.Empty,
                    OutputSha256 = string.Empty,
                    Traceparent = lease.Traceparent,
                },
                lease.Traceparent.Length > 0 ? lease.Traceparent : null,
                token)).ConfigureAwait(false);
            if (!reported)
            {
                Telemetry.TasksStale.Add(1, tags);
                span?.SetTag("sagat.outcome", "stale");
                return;
            }

            Telemetry.TasksCompleted.Add(1, tags);
            Telemetry.ActivityDuration.Record(Stopwatch.GetElapsedTime(started).TotalSeconds, tags);
            span?.SetTag("sagat.outcome", "completed");
            Log.TaskReported(logger, lease.TaskId, "completed");
        }
        catch (Exception error) when (error is SagatException or OperationCanceledException)
        {
            Telemetry.TasksDropped.Add(1, tags);
            span?.SetStatus(ActivityStatusCode.Error);
            Log.CompletionNotReported(logger, lease.TaskId, error is SagatException known ? known.Code.ToWireName() : "CANCELLED");
        }
    }

    private async Task FailAsync(Session session, TaskLease lease, string code, string message, bool retryable, long started, Activity? span)
    {
        var channel = session.Rpc;
        var safeCode = CodePattern().IsMatch(code) ? code : "WORKER_ERROR";
        try
        {
            var reported = await WithRetriesAsync(session, token => channel.FailStepAsync(
                new FailStepRequest
                {
                    TaskId = lease.TaskId,
                    RunId = lease.RunId,
                    StepId = lease.StepId,
                    Attempt = lease.Attempt,
                    Generation = lease.Generation,
                    LeaseToken = lease.LeaseToken,
                    IdempotencyKey = "failed-" + lease.TaskId,
                    ErrorCode = safeCode,
                    SafeMessage = Cut(message),
                    Retryable = retryable,
                    Traceparent = lease.Traceparent,
                },
                lease.Traceparent.Length > 0 ? lease.Traceparent : null,
                token)).ConfigureAwait(false);
            if (!reported)
            {
                Telemetry.TasksStale.Add(1, tags);
                span?.SetTag("sagat.outcome", "stale");
                return;
            }

            var failed = tags;
            failed.Add("sagat.error.code", safeCode);
            Telemetry.TasksFailed.Add(1, failed);
            Telemetry.ActivityDuration.Record(Stopwatch.GetElapsedTime(started).TotalSeconds, tags);
            span?.SetStatus(ActivityStatusCode.Error);
            span?.SetTag("error.type", safeCode);
            span?.SetTag("sagat.outcome", "failed");
            Log.TaskReported(logger, lease.TaskId, "failed");
        }
        catch (Exception error) when (error is SagatException or OperationCanceledException)
        {
            Telemetry.TasksDropped.Add(1, tags);
            span?.SetStatus(ActivityStatusCode.Error);
            Log.FailureNotReported(logger, lease.TaskId, error is SagatException known ? known.Code.ToWireName() : "CANCELLED");
        }
    }

    private static string Cut(string message)
    {
        if (message.Length <= MaxMessage)
        {
            return message;
        }

        var cut = message[..MaxMessage];
        return char.IsHighSurrogate(cut[^1]) ? cut[..^1] : cut;
    }

    // --------------------------------------------------------------------------------------- secrets

    private async Task<ActivitySecrets> ResolveSecretsAsync(Session session, TaskLease lease, CancellationToken cancellationToken)
    {
        if (minor < 2)
        {
            throw new SagatException(SagatErrorCode.UnsupportedProtocol, "the server speaks a protocol older than 1.2, which has no ResolveSecrets");
        }

        var answer = await session.Rpc.ResolveSecretsAsync(
            new ResolveSecretsRequest
            {
                TaskId = lease.TaskId,
                RunId = lease.RunId,
                StepId = lease.StepId,
                Attempt = lease.Attempt,
                Generation = lease.Generation,
                LeaseToken = lease.LeaseToken,
                Traceparent = lease.Traceparent,
            },
            lease.Traceparent.Length > 0 ? lease.Traceparent : null,
            cancellationToken).ConfigureAwait(false);
        var values = new Dictionary<string, byte[]>(StringComparer.Ordinal);
        foreach (var secret in answer.Secrets)
        {
            values[secret.Name] = secret.Value.ToByteArray();
        }

        return new ActivitySecrets(values);
    }

    /// <summary>Resolves the secrets of one attempt once, on first use, and clears them when the attempt is over.</summary>
    private sealed class SecretsLoader(WorkerHost host, TaskLease lease, Session session) : IDisposable
    {
        private readonly SemaphoreSlim gate = new(1, 1);
        private ActivitySecrets? loaded;

        internal async Task<ActivitySecrets> GetAsync(CancellationToken cancellationToken)
        {
            await gate.WaitAsync(cancellationToken).ConfigureAwait(false);
            try
            {
                return loaded ??= await host.ResolveSecretsAsync(session, lease, cancellationToken).ConfigureAwait(false);
            }
            finally
            {
                gate.Release();
            }
        }

        public void Dispose()
        {
            loaded?.Dispose();
            gate.Dispose();
        }
    }

    /// <summary>The channel of one start and the token that ends the reports still running when it is closed.</summary>
#pragma warning disable CA1001 // The token source has no timer or handle to release, and a report that outlives the close may still read its token.
    private sealed class Session(WorkerRpc rpc)
#pragma warning restore CA1001
    {
        private readonly CancellationTokenSource report = new();

        internal WorkerRpc Rpc { get; } = rpc;

        internal CancellationToken ReportToken => report.Token;

        internal void Close()
        {
            report.Cancel();
            Rpc.Dispose();
        }
    }

    private sealed class InFlight(TaskRuntime runtime)
    {
        internal TaskRuntime Runtime { get; } = runtime;

        internal TaskCompletionSource Completion { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
    }
}
