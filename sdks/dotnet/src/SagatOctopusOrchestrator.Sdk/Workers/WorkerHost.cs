using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Runtime.CompilerServices;
using System.Text.RegularExpressions;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.Internal;

namespace SagatOctopusOrchestrator.Sdk.Workers;

/// <summary>The worker host of the SDK (SPEC section 6).</summary>
[DebuggerDisplay("{ToString(),nq}")]
public sealed partial class WorkerHost : IWorkerHost
{
    private const int MaxOutputBytes = 256 * 1024;
    private const int MaxPoll = 64;
    private const int ReportAttempts = 5;
    private const int MaxMessage = 512;

    [DebuggerBrowsable(DebuggerBrowsableState.Never)]
    private readonly SagatOptions options;
    private readonly ILoggerFactory loggerFactory;
    private readonly ILogger logger;
    private readonly JsonSerializerOptions json;
    private readonly int maxInflight;
    private readonly Dictionary<string, ActivityRegistration> handlers = new(StringComparer.Ordinal);
    private readonly ConcurrentDictionary<long, InFlight> inflight = new();
    private readonly SemaphoreSlim lifecycle = new(1, 1);
    private readonly object wakeGate = new();
    private readonly TagList tags;

    private Session? current;
    private string workerId = string.Empty;
    [DebuggerBrowsable(DebuggerBrowsableState.Never)]
    private string sessionToken = string.Empty;
    private string accepted = string.Empty;
    private int minor;
    private volatile bool running;
    private volatile bool stopping;
    private Task loop = Task.CompletedTask;
    private CancellationTokenSource? loopCancel;
    private TaskCompletionSource? wake;
    private TaskCompletionSource stopSignal = new(TaskCreationOptions.RunContinuationsAsynchronously);
    private bool wakePending;
    private long sequence;

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
        ILogger created = NullLogger.Instance;
        Safe.Run(() => created = this.loggerFactory.CreateLogger<WorkerHost>());
        logger = SafeLogger.Of(created);
        json = Wire.OptionsOf(this.options);
        tags = new TagList { { "sagat.worker", this.options.Worker.Name ?? string.Empty } };
    }

    /// <summary>The settings of the host, without any credential.</summary>
    public override string ToString() => $"WorkerHost {{ GrpcUrl = {SagatOptions.HostOf(options.GrpcUrl)}, Namespace = {options.Namespace}, Worker = {options.Worker.Name}, Running = {running} }}";

    /// <inheritdoc/>
    public string ProtocolVersion => accepted;

    /// <inheritdoc/>
    public bool IsRunning => running;

    [GeneratedRegex("^[A-Z0-9_]{1,64}\\z", RegexOptions.CultureInvariant)]
    private static partial Regex CodePattern();

    [GeneratedRegex("^1(?:\\.(\\d+))?\\z", RegexOptions.CultureInvariant)]
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
            lock (wakeGate)
            {
                stopSignal = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
                wakePending = false;
                wake = null;
            }

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
            WakeForStop();

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

            // The session ends with the host: its token is not kept (a restart registers again and gets another).
            sessionToken = string.Empty;
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
        if (options.Worker.HeartbeatInterval is { } beat && (beat <= TimeSpan.Zero || beat > TimeSpan.FromHours(1)))
        {
            throw SagatException.Invalid("Worker.HeartbeatInterval is between 0 and an hour (leave it null for a third of the lease)");
        }

        if (string.IsNullOrWhiteSpace(options.Worker.Name))
        {
            throw SagatException.Invalid("Worker.Name is required");
        }

        if (string.IsNullOrWhiteSpace(options.GrpcUrl) || !Uri.TryCreate(options.GrpcUrl, UriKind.Absolute, out var url) || url.Scheme is not ("http" or "https"))
        {
            throw SagatException.Invalid("GrpcUrl is required and is an http or https URL");
        }

        Internal.Tls.RequireTlsForServerName(options.Tls, url);

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

    /// <summary>A task finished (there is room again) or a task was released: the pause of the loop ends, or the next one is not taken.</summary>
    private void Wake()
    {
        TaskCompletionSource? waiting;
        lock (wakeGate)
        {
            waiting = wake;
            if (waiting is null)
            {
                // The loop is not parked now (it may be in the middle of a poll): the next pause must not be taken.
                wakePending = true;
            }
        }

        waiting?.TrySetResult();
    }

    private void WakeForStop()
    {
        TaskCompletionSource signal;
        lock (wakeGate)
        {
            signal = stopSignal;
        }

        signal.TrySetResult();
        Wake();
    }

    // The pause between polls, which is at most a minute whatever the server asked for. A host that is stopping does not pause. A task that
    // finishes ends it (there is room again), also when it finished while the poll was in flight (`wakePending`); the pause of a backoff
    // (`endsWithTask` false) is ended only by the stop.
    private async Task IdleAsync(TimeSpan wait, bool endsWithTask = true)
    {
        var waiting = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        Task stop;
        lock (wakeGate)
        {
            stop = stopSignal.Task;
            if (stopping)
            {
                return;
            }

            if (endsWithTask)
            {
                if (wakePending)
                {
                    wakePending = false;
                    return;
                }

                wake = waiting;
            }
        }

        try
        {
            using var timer = new CancellationTokenSource();
            var delay = Task.Delay(Limits.Capped(wait), timer.Token);
            await Task.WhenAny(delay, stop, waiting.Task).ConfigureAwait(false);
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

                wakePending = false;
            }
        }
    }

    private static TimeSpan Jitter(TimeSpan wait) =>
        TimeSpan.FromMilliseconds(Math.Round(wait.TotalMilliseconds * (0.8 + (Random.Shared.NextDouble() * 0.4))));

    private async Task RunLoopAsync(Session session, CancellationToken cancellationToken)
    {
        var failures = 0;
        var refused = 0;
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

                var asked = Math.Min(free, MaxPoll);
                var response = await channel.PollAsync(
                    new PollTasksRequest { WorkerId = workerId, SessionToken = sessionToken, MaxTasks = asked },
                    cancellationToken).ConfigureAwait(false);
                failures = 0;
                refused = 0;
                Safe.Add(Telemetry.Polls, tags);

                // The host runs at most as many tasks as it asked for: the others keep a lease that lapses and is given again.
                var got = response.Tasks.Count;
                if (got > asked)
                {
                    Log.TooManyTasks(logger, got, asked, got - asked);
                }

                for (var i = 0; i < Math.Min(got, asked); i++)
                {
                    Dispatch(response.Tasks[i], session);
                }

                if (got == 0)
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
                    Safe.Add(Telemetry.Reconnects, tags);

                    // The session is gone: register again and go on. A server that registers the worker and refuses it at once must not be
                    // hammered: from the second refusal in a row the registration waits with the backoff of section 5, until a poll succeeds.
                    if (refused > 0)
                    {
                        await IdleAsync(ReconnectBackoff(refused - 1), endsWithTask: false).ConfigureAwait(false);
                        if (stopping)
                        {
                            return;
                        }
                    }

                    refused++;
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

                Safe.Add(Telemetry.Reconnects, tags);
                var pause = Min(TimeSpan.FromSeconds(30), Max(error.RetryAfter, ReconnectBackoff(failures++)));
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

    /// <summary>The backoff of section 5 for a host that reconnects: no limit on attempts, a cap of 30 s.</summary>
    private TimeSpan ReconnectBackoff(int n) =>
        new RetryPolicy { InitialDelay = options.Retry.InitialDelay, MaxDelay = TimeSpan.FromSeconds(30) }.Backoff(n);

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
        try
        {
            var id = Interlocked.Increment(ref sequence);
            var entry = new InFlight(new TaskRuntime(lease.ExpiresAtEpochMs));
            inflight[id] = entry;
            try
            {
                Safe.Add(Telemetry.TasksTaken, tags);
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
                        // Its own entry, whatever else was leased under the same task id.
                        inflight.TryRemove(id, out _);
                        entry.Completion.TrySetResult();
                        Wake();
                    }
                });
            }
#pragma warning disable CA1031 // The task could not be started: it is not in flight, and its lease lapses and is given again.
            catch (Exception error)
#pragma warning restore CA1031
            {
                inflight.TryRemove(id, out _);
                entry.Completion.TrySetResult();
                Log.TaskCrashed(logger, lease.TaskId, error.GetType().Name);
            }
        }
#pragma warning disable CA1031 // One lease that cannot be dispatched must not cost the others of the same answer.
        catch (Exception error)
#pragma warning restore CA1031
        {
            Log.TaskCrashed(logger, lease.TaskId, error.GetType().Name);
        }
    }

    private async Task ExecuteAsync(TaskLease lease, TaskRuntime runtime, Session session)
    {
        var started = Stopwatch.GetTimestamp();
        ActivityRegistration? registered;
        lock (handlers)
        {
            handlers.TryGetValue(lease.ActivityName, out registered);
        }

        // `task_kind` came with 1.2: against an older server every task is a forward one, whatever the field says.
        var kind = minor < 2
            ? TaskKind.Forward
            : lease.TaskKind switch
            {
                "COMPENSATION" => TaskKind.Compensation,
                "RECONCILIATION" => TaskKind.Reconciliation,
                _ => TaskKind.Forward,
            };

        var span = StartSpan(lease, kind);
        using var scope = logger.BeginScope(new Dictionary<string, object?>
        {
            ["sagat.task_id"] = lease.TaskId,
            ["sagat.run_id"] = lease.RunId,
            ["sagat.attempt"] = lease.Attempt,
        });
        try
        {
            await ExecuteInAsync(lease, runtime, session, kind, started, registered, span).ConfigureAwait(false);
        }
        finally
        {
            Safe.Stop(span);
        }
    }

    private async Task ExecuteInAsync(TaskLease lease, TaskRuntime runtime, Session session, TaskKind kind, long started, ActivityRegistration? registered, Activity? span)
    {
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
                minor >= 2 ? lease.TargetNode : string.Empty,
                minor >= 2 ? lease.TargetIdempotencyKey : string.Empty,
                steps,
                ContextLogger(lease.ActivityName),
                secretsLoader.GetAsync,
                (source, blobOptions, cancellationToken) => WriteBlobAsync(session, lease, runtime, source, blobOptions, cancellationToken),
                (descriptor, cancellationToken) => ReadBlobAsync(session, lease, runtime, descriptor, cancellationToken));

            await RunAndReportAsync(session, lease, runtime, registered, typed, context, secretsLoader, started, span).ConfigureAwait(false);
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
        SecretsLoader secretsLoader,
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
                    Safe.Add(Telemetry.TasksDropped, tags);
                    Log.ShutdownAbandoned(logger, lease.TaskId);
                    Safe.Tag(span, "sagat.outcome", "shutdown");
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

            runtime.MarkFinished();

            // The secrets are credentials: gone when the handler is, before the result is reported.
            secretsLoader.Close();

            if (runtime.Lost)
            {
                Safe.Add(Telemetry.TasksStale, tags);
                Log.LeaseLost(logger, lease.TaskId);
                Safe.Tag(span, "sagat.outcome", "lease-lost");
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

            var blob = result!.Blob;
            string output;
            if (blob is not null)
            {
                if (minor < 3)
                {
                    await FailAsync(session, lease, "UNSUPPORTED_PROTOCOL", "blob output requires worker protocol 1.3", retryable: false, started, span).ConfigureAwait(false);
                    return;
                }

                try
                {
                    Blobs.Validate(blob);
                }
                catch (SagatException)
                {
                    await FailAsync(session, lease, "OUTPUT_INVALID", "the blob output descriptor is malformed", retryable: false, started, span).ConfigureAwait(false);
                    return;
                }

                output = string.Empty;
            }
            else
            {
                try
                {
                    output = result.ToJson(json) ?? string.Empty;
                }
                catch (SagatException error)
                {
                    await FailAsync(session, lease, "OUTPUT_INVALID", error.Message, retryable: false, started, span).ConfigureAwait(false);
                    return;
                }
            }

            if (Encoding.UTF8.GetByteCount(output) > MaxOutputBytes)
            {
                await FailAsync(session, lease, "PAYLOAD_TOO_LARGE", "the output is over 256 KiB", retryable: false, started, span).ConfigureAwait(false);
                return;
            }

            await CompleteAsync(session, lease, output, blob, started, span).ConfigureAwait(false);
        }
        finally
        {
            await stopBeat.CancelAsync().ConfigureAwait(false);
            await heartbeat.ConfigureAwait(false);
        }
    }

    private ILogger ContextLogger(string activity)
    {
        ILogger? created = null;
        Safe.Run(() => created = loggerFactory.CreateLogger("SagatOctopusOrchestrator.Activity." + activity));
        return SafeLogger.Of(created ?? NullLogger.Instance);
    }

    private static bool IsCancellation(Exception error) =>
        error is OperationCanceledException || (error is SagatException { Code: SagatErrorCode.Cancelled });

    private async Task<BlobDescriptor> WriteBlobAsync(
        Session session,
        TaskLease lease,
        TaskRuntime runtime,
        Stream source,
        BlobWriteOptions blobOptions,
        CancellationToken cancellationToken)
    {
        if (runtime.Finished)
        {
            throw SagatException.Invalid("the attempt is over");
        }

        if (minor < 3)
        {
            throw new SagatException(SagatErrorCode.UnsupportedProtocol, "blob streaming requires worker protocol 1.3");
        }

        using var linked = CancellationTokenSource.CreateLinkedTokenSource(runtime.Abort.Token, cancellationToken);
        return await Blobs.WriteAsync(session.Rpc, lease, source, blobOptions, linked.Token).ConfigureAwait(false);
    }

    private async IAsyncEnumerable<ReadOnlyMemory<byte>> ReadBlobAsync(
        Session session,
        TaskLease lease,
        TaskRuntime runtime,
        BlobDescriptor descriptor,
        [EnumeratorCancellation] CancellationToken cancellationToken)
    {
        if (runtime.Finished)
        {
            throw SagatException.Invalid("the attempt is over");
        }

        if (minor < 3)
        {
            throw new SagatException(SagatErrorCode.UnsupportedProtocol, "blob streaming requires worker protocol 1.3");
        }

        using var linked = CancellationTokenSource.CreateLinkedTokenSource(runtime.Abort.Token, cancellationToken);
        await foreach (var chunk in Blobs.ReadAsync(session.Rpc, lease, descriptor, linked.Token).ConfigureAwait(false))
        {
            yield return chunk;
        }
    }

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
        const string name = "sagat.worker.task";
        var parented = lease.Traceparent.Length > 0 && System.Diagnostics.ActivityContext.TryParse(lease.Traceparent, null, out var parent);
        var span = parented
            ? Safe.Start(name, ActivityKind.Consumer, parent)
            : Safe.Start(name, ActivityKind.Consumer, default(System.Diagnostics.ActivityContext));

        // Nobody listens to the SDK's source (no span of its own), but the handler is still in the trace of the run: the requests it makes carry it
        // (`Activity.Current` is what the SDK's client and every other instrumented client propagate).
        span ??= parented ? Safe.StartDetached(name, lease.Traceparent) : null;
        Safe.Tag(span, "sagat.worker", options.Worker.Name);
        Safe.Tag(span, "sagat.namespace", options.Namespace);
        Safe.Tag(span, "sagat.activity", lease.ActivityName);
        Safe.Tag(span, "sagat.task.id", lease.TaskId);
        Safe.Tag(span, "sagat.run.id", lease.RunId);
        Safe.Tag(span, "sagat.step.id", lease.StepId);
        Safe.Tag(span, "sagat.attempt", lease.Attempt);
        Safe.Tag(span, "sagat.task.kind", kind.ToString());
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
                        Safe.Add(Telemetry.Heartbeats, tags);
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
                        await Breathe(stop).ConfigureAwait(false);
                    }
#pragma warning disable CA1031 // A beat must never be the last: whatever else went wrong, the next one tries again.
                    catch (Exception error)
#pragma warning restore CA1031
                    {
                        Log.HeartbeatFailed(logger, lease.TaskId, error.GetType().Name);
                        await Breathe(stop).ConfigureAwait(false);
                    }
                }
            },
            CancellationToken.None);
    }

    /// <summary>A short pause inside a failure handler of a loop: whatever raised, even before the interval was waited for, the loop cannot spin.</summary>
    private static async Task Breathe(CancellationToken stop)
    {
        try
        {
            await Task.Delay(TimeSpan.FromMilliseconds(100), stop).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            // Stopping: the loop ends at its next check.
        }
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

    private async Task CompleteAsync(Session session, TaskLease lease, string output, BlobDescriptor? blob, long started, Activity? span)
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
                    OutputBlobRef = blob?.Id ?? string.Empty,
                    OutputSha256 = blob?.Sha256 ?? string.Empty,
                    Traceparent = lease.Traceparent,
                },
                lease.Traceparent.Length > 0 ? lease.Traceparent : null,
                token)).ConfigureAwait(false);
            if (!reported)
            {
                Safe.Add(Telemetry.TasksStale, tags);
                Safe.Tag(span, "sagat.outcome", "stale");
                return;
            }

            Safe.Add(Telemetry.TasksCompleted, tags);
            Safe.Record(Telemetry.ActivityDuration, Stopwatch.GetElapsedTime(started).TotalSeconds, tags);
            Safe.Tag(span, "sagat.outcome", "completed");
            Log.TaskReported(logger, lease.TaskId, "completed");
        }
#pragma warning disable CA1031 // A report that cannot be made, for whatever reason, is a dropped task: it never takes the host down.
        catch (Exception error)
#pragma warning restore CA1031
        {
            Safe.Add(Telemetry.TasksDropped, tags);
            Safe.Fail(span);
            Log.CompletionNotReported(logger, lease.TaskId, ReasonOf(error));
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
                Safe.Add(Telemetry.TasksStale, tags);
                Safe.Tag(span, "sagat.outcome", "stale");
                return;
            }

            var failed = tags;
            failed.Add("sagat.error.code", safeCode);
            Safe.Add(Telemetry.TasksFailed, failed);
            Safe.Record(Telemetry.ActivityDuration, Stopwatch.GetElapsedTime(started).TotalSeconds, tags);
            Safe.Fail(span);
            Safe.Tag(span, "error.type", safeCode);
            Safe.Tag(span, "sagat.outcome", "failed");
            Log.TaskReported(logger, lease.TaskId, "failed");
        }
#pragma warning disable CA1031 // A report that cannot be made, for whatever reason, is a dropped task: it never takes the host down.
        catch (Exception error)
#pragma warning restore CA1031
        {
            Safe.Add(Telemetry.TasksDropped, tags);
            Safe.Fail(span);
            Log.FailureNotReported(logger, lease.TaskId, ReasonOf(error));
        }
    }

    private static string ReasonOf(Exception error) => error switch
    {
        SagatException known => known.Code.ToWireName(),
        OperationCanceledException => "CANCELLED",
        _ => error.GetType().Name,
    };

    /// <summary>
    /// At most <see cref="MaxMessage"/> Unicode code points (SPEC section 6.5), never inside a surrogate pair; a lone surrogate becomes U+FFFD,
    /// so that what is sent is always valid UTF-8.
    /// </summary>
    private static string Cut(string message)
    {
        var builder = new StringBuilder(Math.Min(message.Length, MaxMessage * 2));
        var count = 0;
        foreach (var rune in message.EnumerateRunes())
        {
            if (count++ == MaxMessage)
            {
                break;
            }

            builder.Append(rune.ToString());
        }

        return builder.ToString();
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

    /// <summary>
    /// Resolves the secrets of one attempt once, on first use (a failed resolution is not remembered), and wipes them when the handler returns:
    /// after that nothing can be asked for.
    /// </summary>
    private sealed class SecretsLoader(WorkerHost host, TaskLease lease, Session session) : IDisposable
    {
        private readonly SemaphoreSlim gate = new(1, 1);
        private ActivitySecrets? loaded;
        private bool closed;

        internal async Task<ActivitySecrets> GetAsync(CancellationToken cancellationToken)
        {
            await gate.WaitAsync(cancellationToken).ConfigureAwait(false);
            try
            {
                return Volatile.Read(ref closed)
                    ? throw SagatException.Invalid("the secrets of an attempt are not available once its handler has returned")
                    : loaded ??= await host.ResolveSecretsAsync(session, lease, cancellationToken).ConfigureAwait(false);
            }
            finally
            {
                gate.Release();
            }
        }

        /// <summary>Wipes what was resolved and refuses any later request.</summary>
        internal void Close()
        {
            Volatile.Write(ref closed, true);
            loaded?.Dispose();
        }

        // The gate holds no handle that needs releasing, and the context of a finished task may still ask (and be refused).
        public void Dispose() => Close();
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
