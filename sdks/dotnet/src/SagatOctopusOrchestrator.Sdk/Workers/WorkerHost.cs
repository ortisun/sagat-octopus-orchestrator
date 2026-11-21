using System.Collections.Concurrent;
using System.Diagnostics;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json;
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
    private const int MaxStreamInputBytes = 8 * 1024 * 1024;
    private const int ReportAttempts = 5;
    private const int MaxMessage = 512;

    // At most this many hand-backs are on their way at once; the others wait their turn in the order received (SPEC section 6.2,
    // decision D006-026): a server that answers a poll with thousands of leases does not start thousands of reports.
    private const int MaxHandBacks = 8;

    // The first protocol minor on which the host hands a lease back: its server reserves TASK_NOT_STARTED, and an older one would record it
    // as an ordinary failure that spends the attempt (decision D006-080).
    private const int HandBackMinor = 8;

    // Why a lease the host received is not run (SPEC section 6.2): it arrived after the stop began, it is beyond the credit or the
    // free slots, it is in the rest of a frame that broke the receive contract, or the host could not start it.
    // The step failure code of a lease the host received and hands back without starting it, from protocol 1.8; it is never raised to a
    // caller.
    private const string TaskNotStarted = "TASK_NOT_STARTED";

    private const string HandBackStopping = "stopping";
    private const string HandBackCredit = "credit";
    private const string HandBackMalformed = "malformed";
    private const string HandBackNotStarted = "not_started";

    [DebuggerBrowsable(DebuggerBrowsableState.Never)]
    private readonly SagatOptions options;
    private readonly ILoggerFactory loggerFactory;
    private readonly ILogger logger;
    private readonly JsonSerializerOptions json;
    private readonly int maxInflight;
    private readonly Dictionary<string, ActivityRegistration> handlers = new(StringComparer.Ordinal);

    // The tasks in flight by their in-flight key (InflightKey): the task id, which sibling iterations of a collection share, and from
    // protocol 1.5 the instance path.
    private readonly ConcurrentDictionary<string, InFlight> inflight = new(StringComparer.Ordinal);

    // The hand-backs on their way, queued ones included, by the in-flight key of their lease: stop waits for them as for the tasks in
    // flight, and a lease under one of these keys is a duplicate (decision D006-027), but they hold no slot.
    private readonly ConcurrentDictionary<string, TaskCompletionSource> handingBack = new(StringComparer.Ordinal);

    // The hand-backs waiting for one of the MaxHandBacks workers, in the order they were received, and how many workers drain them.
    private readonly ConcurrentQueue<HandBackItem> handBackQueue = new();

    // How a task is started: the thread pool. The tests replace it by reflection with a runtime that refuses a task, the one path of
    // Dispatch that nothing else can reach.
    private readonly Func<Func<Task>, Task> startTask = static work => Task.Run(work);

    // The bound of the hand-back workers, and what a worker does once it has found the queue empty and before it gives up its place. The
    // tests lower the one and hold the other by reflection, to queue a hand-back exactly while a worker leaves.
    private readonly int handBackBound = MaxHandBacks;
    private readonly Func<Task> beforeHandBackWorkerLeaves = static () => Task.CompletedTask;
    private int handBackWorkers;
    private readonly SemaphoreSlim lifecycle = new(1, 1);
    private readonly object wakeGate = new();
    private readonly TagList tags;

    private Session? current;
    private string workerId = string.Empty;
    [DebuggerBrowsable(DebuggerBrowsableState.Never)]
    private string sessionToken = string.Empty;
    private string accepted = string.Empty;
    private int minor;
    private bool streamUnavailable;
    private volatile bool running;
    private volatile bool stopping;
    private Task loop = Task.CompletedTask;
    private CancellationTokenSource? loopCancel;
    private TaskCompletionSource? wake;
    private TaskCompletionSource stopSignal = new(TaskCreationOptions.RunContinuationsAsynchronously);
    private bool wakePending;

    // The clock of the host: the time a lease is taken at, the heartbeat interval, the pause after a failed beat and the pauses of the
    // loop are counted on it. It is the
    // system clock; the unit tests replace it (by reflection, before the host starts: they compile the protocol types themselves, so they
    // cannot be given the internals) to read the pacing exactly instead of by the wall clock.
    private readonly TimeProvider time = TimeProvider.System;

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
            loopCancel?.Cancel();

            // The receive call in progress ends and no other starts; a poll answer or a stream frame the loop already holds is handed back
            // whole from protocol 1.8 (SPEC section 6.2). Cancelling the token ends the wait for that call at once (the host is going away), never the code
            // that reads what was received.
            using (cancellationToken.Register(static state => ((CancellationTokenSource)state!).Cancel(), loopCancel))
            {
                await loop.ConfigureAwait(false);
            }

            // The loop has ended: every task and every hand-back it started is in one of the two tables. The hand-backs are waited for within
            // the same grace, so that the channel is still open for them.
            var pending = Task.WhenAll(inflight.Values.Select(task => task.Completion.Task).Concat(handingBack.Values.Select(handBack => handBack.Task)));
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
                streamUnavailable = false;
                RequireStreamingProtocol();
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
            var delay = Task.Delay(Limits.Capped(wait), time, timer.Token);
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

    private void RequireStreamingProtocol()
    {
        if (options.Worker.Transport == WorkerTransport.Stream && minor < 4)
        {
            throw new SagatException(SagatErrorCode.UnsupportedProtocol, "stream transport requires worker protocol 1.4");
        }
    }

    private async Task RunLoopAsync(Session session, CancellationToken cancellationToken)
    {
        var failures = 0;
        var refused = 0;
        var channel = session.Rpc;
        while (!stopping)
        {
            var meaningful = false;
            try
            {
                var free = maxInflight - inflight.Count;
                if (free <= 0)
                {
                    await IdleAsync(TimeSpan.FromMilliseconds(50)).ConfigureAwait(false);
                    continue;
                }

                var asked = Math.Min(free, MaxPoll);
                RequireStreamingProtocol();
                if (minor >= 4 && options.Worker.Transport != WorkerTransport.Poll && !streamUnavailable)
                {
                    try
                    {
                        var delivered = 0;
                        var inputBytes = 0;
                        var hintedWait = 0L;
                        var window = new HashSet<string>(StringComparer.Ordinal);
                        var windowMinor = minor;
                        await foreach (var frame in channel.StreamTasksAsync(
                            new PollTasksRequest { WorkerId = workerId, SessionToken = sessionToken, MaxTasks = asked },
                            cancellationToken).ConfigureAwait(false))
                        {
                            if (stopping)
                            {
                                // A host that is stopping starts nothing new: the frame it read is handed back whole (from protocol 1.8).
                                foreach (var lease in frame.Tasks)
                                {
                                    Take(window, lease, HandBackStopping, session, windowMinor);
                                }

                                return;
                            }

                            if (frame.Tasks.Count == 0)
                            {
                                hintedWait = frame.RetryAfterMs;
                                continue;
                            }

                            // The host runs at most the credit it asked for and has slots for, within the input budget of the window: the
                            // lease that breaks either and the rest of the frame after it are not run, and the stream is given up.
                            var allowed = Math.Min(asked - delivered, maxInflight - inflight.Count);
                            var taken = 0;
                            string? why = null;
                            SagatException? broken = null;
                            foreach (var lease in frame.Tasks)
                            {
                                var bytes = Encoding.UTF8.GetByteCount(lease.InputJson);
                                if (why is null && taken >= allowed)
                                {
                                    why = HandBackCredit;
                                    broken = SagatException.Invalid("a task stream exceeded its receive credit");
                                }
                                else if (why is null && bytes > MaxStreamInputBytes - inputBytes)
                                {
                                    why = HandBackMalformed;
                                    broken = SagatException.Invalid("a task stream exceeded its input budget");
                                }

                                if (Take(window, lease, why, session, windowMinor))
                                {
                                    inputBytes += bytes;
                                    delivered++;
                                    taken++;
                                    meaningful = true;
                                }
                            }

                            if (broken is not null)
                            {
                                throw broken;
                            }
                        }
                        failures = 0;
                        refused = 0;
                        if (delivered < asked)
                        {
                            var wait = Math.Max(50, hintedWait > 0 ? hintedWait : 1000);
                            await IdleAsync(Limits.Capped(Jitter(TimeSpan.FromMilliseconds(wait)))).ConfigureAwait(false);
                        }
                        continue;
                    }
                    catch (StreamTasksUnimplementedException)
                    {
                        if (options.Worker.Transport == WorkerTransport.Stream)
                        {
                            throw new SagatException(SagatErrorCode.UnsupportedProtocol, "StreamTasks is not implemented by the server");
                        }
                        streamUnavailable = true;
                        await IdleAsync(TimeSpan.FromMilliseconds(100), endsWithTask: false).ConfigureAwait(false);
                        continue;
                    }
                }
                var answerMinor = minor;
                var response = await channel.PollAsync(
                    new PollTasksRequest { WorkerId = workerId, SessionToken = sessionToken, MaxTasks = asked },
                    cancellationToken).ConfigureAwait(false);
                failures = 0;
                refused = 0;
                Safe.Add(Telemetry.Polls, tags);

                // The host runs at most as many tasks as it asked for, and nothing once it is stopping: a server that sends more is not
                // obeyed, and the rest are not run (SPEC section 6.2).
                var got = response.Tasks.Count;
                if (got > asked)
                {
                    Log.TooManyTasks(logger, got, asked, got - asked);
                }

                var answer = new HashSet<string>(StringComparer.Ordinal);
                var dispatched = 0;
                var stoppingNow = stopping;
                foreach (var lease in response.Tasks)
                {
                    var why = stoppingNow ? HandBackStopping : dispatched >= asked ? HandBackCredit : null;
                    if (Take(answer, lease, why, session, answerMinor))
                    {
                        dispatched++;
                    }
                }

                if (stoppingNow)
                {
                    return;
                }

                if (got == 0)
                {
                    var wait = Math.Max(50, response.RetryAfterMs > 0 ? response.RetryAfterMs : 1000);
                    await IdleAsync(Limits.Capped(Jitter(TimeSpan.FromMilliseconds(wait)))).ConfigureAwait(false);
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

                if (meaningful)
                {
                    failures = 0;
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

    /// <summary>
    /// What identifies a task while it runs: the legacy task id is <c>run/node/generation</c>, which sibling iterations of a collection share, so
    /// from protocol 1.5 the instance path is part of it.
    /// </summary>
    private static string InflightKey(TaskLease lease, int protocolMinor)
    {
        if (protocolMinor < 5 || lease.Instance is null || lease.Instance.InstancePath.Count == 0)
        {
            return lease.TaskId;
        }

        var key = new StringBuilder(lease.TaskId);
        foreach (var segment in lease.Instance.InstancePath)
        {
            key.Append('/').Append(segment.ScopeNodeId).Append('[').Append(segment.IterationIndex.ToString(System.Globalization.CultureInfo.InvariantCulture)).Append(']');
        }

        return key.ToString();
    }

    /// <summary>
    /// Dispatches a lease of a poll answer or of a stream receive window, or drops it when <paramref name="why"/> is not null (handing it back
    /// from protocol 1.8), and says whether it took a slot of the credit (it was dispatched, or the host tried to start it). A lease whose in-flight key is in flight, or
    /// was already seen in the same answer or window (<paramref name="seen"/>), is a duplicate: it is ignored and never reported, since its fence
    /// may be the one of the running handler (SPEC section 6.2).
    /// </summary>
    private bool Take(HashSet<string> seen, TaskLease lease, string? why, Session session, int protocolMinor)
    {
        var key = InflightKey(lease, protocolMinor);
        if (inflight.ContainsKey(key) || handingBack.ContainsKey(key) || !seen.Add(key))
        {
            Log.DuplicateIgnored(logger, lease.TaskId);
            return false;
        }

        if (why is not null)
        {
            HandBack(lease, key, session, protocolMinor, why);
            return false;
        }

        Dispatch(lease, key, session, protocolMinor);
        return true;
    }

    /// <summary>
    /// Reports a lease the host received and will not run: <c>FailStep</c> <c>TASK_NOT_STARTED</c>, retryable, before any handler of it runs
    /// (SPEC section 6.2), on a session of protocol 1.8 or later. The server gives the step back without consuming the attempt when it can.
    /// Below 1.8 the lease is only logged and left to lapse. It is the failure report of every task, with its retries, made by one of at most <see cref="MaxHandBacks"/> workers in the
    /// order received; nothing here waits, so the receive loop never blocks on the bound. Stop waits for it like a task in flight, so the
    /// channel is still open.
    /// </summary>
    private void HandBack(TaskLease lease, string key, Session session, int protocolMinor, string why)
    {
        if (protocolMinor < HandBackMinor)
        {
            Log.DroppingLapses(logger, lease.TaskId, why);
            return;
        }

        var done = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        // Only the receive loop hands back, and Take looked for the same key in this table just before: a lease is handed back at most once.
        var added = handingBack.TryAdd(key, done);
        Debug.Assert(added, "a lease was handed back twice");

        Log.HandingBack(logger, lease.TaskId, why);
        handBackQueue.Enqueue(new HandBackItem(lease, key, session, protocolMinor, done));
        if (!TakeHandBackWorker())
        {
            // Every worker is busy: the one that finishes first takes this hand-back up.
            return;
        }

        try
        {
            _ = Task.Run(DrainHandBacksAsync);
        }
#pragma warning disable CA1031 // A worker that cannot be started is logged: the queued hand-backs wait for another, or lapse when stop gives up.
        catch (Exception error)
#pragma warning restore CA1031
        {
            Interlocked.Decrement(ref handBackWorkers);
            Log.HandBackNotStarted(logger, lease.TaskId, error.GetType().Name);
        }
    }

    /// <summary>Takes one of the <see cref="MaxHandBacks"/> places of a hand-back worker, if one is free.</summary>
    private bool TakeHandBackWorker()
    {
        while (true)
        {
            var workers = Volatile.Read(ref handBackWorkers);
            if (workers >= handBackBound)
            {
                return false;
            }

            if (Interlocked.CompareExchange(ref handBackWorkers, workers + 1, workers) == workers)
            {
                return true;
            }
        }
    }

    private async Task DrainHandBacksAsync()
    {
        while (true)
        {
            while (handBackQueue.TryDequeue(out var item))
            {
                try
                {
                    if (item.Session.ReportToken.IsCancellationRequested)
                    {
                        // The channel closed when stop gave up: the lease lapses.
                        Safe.Add(Telemetry.TasksDropped, tags);
                        Log.FailureNotReported(logger, item.Lease.TaskId, "CANCELLED");
                    }
                    else
                    {
                        await FailAsync(item.Session, item.Lease, item.ProtocolMinor, TaskNotStarted, "the host did not start this task", retryable: true, started: null, span: null, handBack: true).ConfigureAwait(false);
                    }
                }
                finally
                {
                    handingBack.TryRemove(new KeyValuePair<string, TaskCompletionSource>(item.Key, item.Done));
                    item.Done.TrySetResult();
                }
            }

            await beforeHandBackWorkerLeaves().ConfigureAwait(false);
            Interlocked.Decrement(ref handBackWorkers);

            // A hand-back queued while this worker was leaving may have found every place taken: it is taken up again here.
            if (handBackQueue.IsEmpty || !TakeHandBackWorker())
            {
                return;
            }
        }
    }

    private void Dispatch(TaskLease lease, string key, Session session, int protocolMinor)
    {
        InFlight? entry = null;
        try
        {
            entry = new InFlight(new TaskRuntime(lease.ExpiresAtEpochMs, time.GetUtcNow()));
            // Only the receive loop adds to the table, and Take looked for the same key in it just before: a lease of a running task is
            // never started again.
            var added = inflight.TryAdd(key, entry);
            Debug.Assert(added, "a running task was leased again");

            var started = entry;
            _ = startTask(async () =>
            {
                try
                {
                    await ExecuteAsync(lease, started.Runtime, session, protocolMinor).ConfigureAwait(false);
                }
#pragma warning disable CA1031 // A bug in one task must not take the host down.
                catch (Exception error)
#pragma warning restore CA1031
                {
                    Log.TaskCrashed(logger, lease.TaskId, error.GetType().Name);
                }
                finally
                {
                    // Its own entry only: a lease under the same key that is taken after this one ended is another task.
                    inflight.TryRemove(new KeyValuePair<string, InFlight>(key, started));
                    started.Completion.TrySetResult();
                    Wake();
                }
            });
        }
#pragma warning disable CA1031 // The task could not be started (its runtime refused it): it is not in flight and is dropped.
        catch (Exception error)
#pragma warning restore CA1031
        {
            if (entry is not null)
            {
                inflight.TryRemove(new KeyValuePair<string, InFlight>(key, entry));
                entry.Completion.TrySetResult();
            }

            Log.TaskCrashed(logger, lease.TaskId, error.GetType().Name);
            HandBack(lease, key, session, protocolMinor, HandBackNotStarted);
            return;
        }

        // Counted once the task is started, outside the block above, so that nothing after the start can hand back a started task: a lease
        // the host could not start is handed back and was never taken.
        Safe.Add(Telemetry.TasksTaken, tags);
        Log.TaskTaken(logger, lease.TaskId, lease.ActivityName, lease.Attempt);
    }

    private async Task ExecuteAsync(TaskLease lease, TaskRuntime runtime, Session session, int protocolMinor)
    {
        var started = Stopwatch.GetTimestamp();
        ActivityRegistration? registered;
        lock (handlers)
        {
            handlers.TryGetValue(lease.ActivityName, out registered);
        }

        // `task_kind` came with 1.2: against an older server every task is a forward one, whatever the field says.
        var kind = protocolMinor < 2
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
            await ExecuteInAsync(lease, runtime, session, protocolMinor, kind, started, registered, span).ConfigureAwait(false);
        }
        finally
        {
            Safe.Stop(span);
        }
    }

    private async Task ExecuteInAsync(TaskLease lease, TaskRuntime runtime, Session session, int protocolMinor, TaskKind kind, long started, ActivityRegistration? registered, Activity? span)
    {
        if (registered is null)
        {
            await FailAsync(session, lease, protocolMinor, "NO_HANDLER", "no handler is registered for the activity", retryable: false, started, span).ConfigureAwait(false);
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
                await FailAsync(session, lease, protocolMinor, "INPUT_INVALID", "the step document is not JSON", retryable: false, started, span).ConfigureAwait(false);
                return;
            }

            var root = document is { RootElement.ValueKind: JsonValueKind.Object } ? document.RootElement : default;
            var hasRoot = root.ValueKind == JsonValueKind.Object;
            JsonElement? input = hasRoot && root.TryGetProperty("input", out var inputElement) ? inputElement : null;
            var node = hasRoot && root.TryGetProperty("node", out var nodeElement) && nodeElement.ValueKind == JsonValueKind.String && nodeElement.GetString() is { Length: > 0 } nodeId
                ? nodeId
                : lease.StepId;
            JsonElement? item = null;
            uint? index = null;
            if (protocolMinor >= 5 && lease.Instance is not null && lease.Instance.InstancePath.Count > 0)
            {
                var itemElement = default(JsonElement);
                var indexElement = default(JsonElement);
                var hasItem = hasRoot && root.TryGetProperty("item", out itemElement);
                var hasIndex = hasRoot && root.TryGetProperty("index", out indexElement);
                if (hasItem || hasIndex)
                {
                    if (!hasItem || !hasIndex || !indexElement.TryGetUInt32(out var parsedIndex))
                    {
                        await FailAsync(session, lease, protocolMinor, "INPUT_INVALID", "the collection item context is malformed", retryable: false, started, span).ConfigureAwait(false);
                        return;
                    }
                    item = itemElement.Clone();
                    index = parsedIndex;
                }
            }

            object? typed;
            try
            {
                typed = registered.ParseInput(input, json);
            }
#pragma warning disable CA1031 // Whatever the deserializer raises, the input does not have the type of the handler.
            catch (Exception)
#pragma warning restore CA1031
            {
                await FailAsync(session, lease, protocolMinor, "INPUT_INVALID", "the input does not have the type of the handler", retryable: false, started, span).ConfigureAwait(false);
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

            using var secretsLoader = new SecretsLoader(lease, session, protocolMinor);
            var context = new ActivityExecutionContext(
                runtime,
                lease.ActivityName,
                lease.TaskId,
                lease.RunId,
                node,
                lease.StepId,
                lease.Attempt,
                lease.Generation,
                protocolMinor >= 5 && lease.Instance is not null
                    ? new global::SagatOctopusOrchestrator.Sdk.Workers.ExecutionInstanceIdentity(
                        lease.Instance.InstancePath.Select(segment => new ExecutionScope(segment.ScopeNodeId, segment.IterationIndex)).ToArray(),
                        lease.Instance.DefinitionNodeId)
                    : null,
                item,
                index,
                lease.IdempotencyKey,
                lease.Traceparent,
                kind,
                protocolMinor >= 2 ? lease.TargetNode : string.Empty,
                protocolMinor >= 2 ? lease.TargetIdempotencyKey : string.Empty,
                steps,
                ContextLogger(lease.ActivityName),
                secretsLoader.GetAsync,
                (source, blobOptions, cancellationToken) => WriteBlobAsync(session, lease, runtime, protocolMinor, source, blobOptions, cancellationToken),
                (descriptor, cancellationToken) => ReadBlobAsync(session, lease, runtime, protocolMinor, descriptor, cancellationToken));

            await RunAndReportAsync(session, lease, runtime, protocolMinor, registered, typed, context, secretsLoader, started, span).ConfigureAwait(false);
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
        int protocolMinor,
        ActivityRegistration registered,
        object? typed,
        ActivityExecutionContext context,
        SecretsLoader secretsLoader,
        long started,
        Activity? span)
    {
        using var stopBeat = new CancellationTokenSource();
        var heartbeat = BeatAsync(session, lease, runtime, protocolMinor, stopBeat.Token);
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
                await FailAsync(session, lease, protocolMinor, failure.Code, failure.Message, failure.IsRetryable, started, span).ConfigureAwait(false);
                return;
            }

            if (result is VerdictResult { Verdict: null })
            {
                await FailAsync(session, lease, protocolMinor, "RECONCILIATION_ANSWER_INVALID", "a reconciler answers applied, notApplied or unknown", retryable: false, started, span).ConfigureAwait(false);
                return;
            }

            var blob = result!.Blob;
            string output;
            if (blob is not null)
            {
                if (protocolMinor < 3)
                {
                    await FailAsync(session, lease, protocolMinor, "UNSUPPORTED_PROTOCOL", "blob output requires worker protocol 1.3", retryable: false, started, span).ConfigureAwait(false);
                    return;
                }

                try
                {
                    Blobs.Validate(blob);
                }
                catch (SagatException)
                {
                    await FailAsync(session, lease, protocolMinor, "OUTPUT_INVALID", "the blob output descriptor is malformed", retryable: false, started, span).ConfigureAwait(false);
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
                    await FailAsync(session, lease, protocolMinor, "OUTPUT_INVALID", error.Message, retryable: false, started, span).ConfigureAwait(false);
                    return;
                }
            }

            if (Encoding.UTF8.GetByteCount(output) > MaxOutputBytes)
            {
                await FailAsync(session, lease, protocolMinor, "PAYLOAD_TOO_LARGE", "the output is over 256 KiB", retryable: false, started, span).ConfigureAwait(false);
                return;
            }

            await CompleteAsync(session, lease, protocolMinor, output, blob, started, span).ConfigureAwait(false);
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

    private static async Task<BlobDescriptor> WriteBlobAsync(
        Session session,
        TaskLease lease,
        TaskRuntime runtime,
        int protocolMinor,
        Stream source,
        BlobWriteOptions blobOptions,
        CancellationToken cancellationToken)
    {
        if (runtime.Finished)
        {
            throw SagatException.Invalid("the attempt is over");
        }

        if (protocolMinor < 3)
        {
            throw new SagatException(SagatErrorCode.UnsupportedProtocol, "blob streaming requires worker protocol 1.3");
        }

        using var linked = CancellationTokenSource.CreateLinkedTokenSource(runtime.Abort.Token, cancellationToken);
        return await Blobs.WriteAsync(session.Rpc, lease, protocolMinor >= 5, source, blobOptions, linked.Token).ConfigureAwait(false);
    }

    private static async IAsyncEnumerable<ReadOnlyMemory<byte>> ReadBlobAsync(
        Session session,
        TaskLease lease,
        TaskRuntime runtime,
        int protocolMinor,
        BlobDescriptor descriptor,
        [EnumeratorCancellation] CancellationToken cancellationToken)
    {
        if (runtime.Finished)
        {
            throw SagatException.Invalid("the attempt is over");
        }

        if (protocolMinor < 3)
        {
            throw new SagatException(SagatErrorCode.UnsupportedProtocol, "blob streaming requires worker protocol 1.3");
        }

        using var linked = CancellationTokenSource.CreateLinkedTokenSource(runtime.Abort.Token, cancellationToken);
        await foreach (var chunk in Blobs.ReadAsync(session.Rpc, lease, protocolMinor >= 5, descriptor, linked.Token).ConfigureAwait(false))
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

    private Task BeatAsync(Session session, TaskLease lease, TaskRuntime runtime, int protocolMinor, CancellationToken stop)
    {
        var channel = session.Rpc;
        var identity = protocolMinor >= 1;

        var interval = options.Worker.HeartbeatInterval ?? runtime.HeartbeatInterval;

        return Task.Run(
            async () =>
            {
                while (!stop.IsCancellationRequested)
                {
                    try
                    {
                        await Task.Delay(interval, time, stop).ConfigureAwait(false);
                        var request = new HeartbeatRequest
                        {
                            TaskId = lease.TaskId,
                            WorkerId = workerId,
                            Generation = lease.Generation,
                            LeaseToken = lease.LeaseToken,

                            // The step identity came with 1.1.
                            RunId = identity ? lease.RunId : string.Empty,
                            StepId = identity ? lease.StepId : string.Empty,
                            Attempt = identity ? lease.Attempt : 0,
                        };
                        if (protocolMinor >= 5 && lease.Instance is not null)
                        {
                            request.Instance = lease.Instance;
                        }

                        var answer = await channel.HeartbeatAsync(
                            request,
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
    private async Task Breathe(CancellationToken stop)
    {
        try
        {
            await Task.Delay(TimeSpan.FromMilliseconds(100), time, stop).ConfigureAwait(false);
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

    private async Task CompleteAsync(Session session, TaskLease lease, int protocolMinor, string output, BlobDescriptor? blob, long started, Activity? span)
    {
        var channel = session.Rpc;
        try
        {
            var request = new CompleteStepRequest
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
            };
            if (protocolMinor >= 5 && lease.Instance is not null)
            {
                request.Instance = lease.Instance;
            }

            var reported = await WithRetriesAsync(session, token => channel.CompleteStepAsync(
                request,
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

    /// <summary>Reports a failure; <paramref name="started"/> is null for a lease that is handed back, which is not an attempt and has no duration.</summary>
    /// <remarks>
    /// <see cref="TaskNotStarted"/> is reserved for the host (SPEC section 6.2): only a hand-back (<paramref name="handBack"/>) sends it, and
    /// any other failure that carries it, a handler's above all, is reported as <c>WORKER_ERROR</c> with its own message and retryable flag.
    /// </remarks>
    private async Task FailAsync(Session session, TaskLease lease, int protocolMinor, string code, string message, bool retryable, long? started, Activity? span, bool handBack = false)
    {
        var channel = session.Rpc;
        var safeCode = handBack ? TaskNotStarted : CodePattern().IsMatch(code) && code != TaskNotStarted ? code : "WORKER_ERROR";
        try
        {
            var request = new FailStepRequest
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
            };
            if (protocolMinor >= 5 && lease.Instance is not null)
            {
                request.Instance = lease.Instance;
            }

            var reported = await WithRetriesAsync(session, token => channel.FailStepAsync(
                request,
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
            if (started is { } since)
            {
                Safe.Record(Telemetry.ActivityDuration, Stopwatch.GetElapsedTime(since).TotalSeconds, tags);
            }
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

    private static async Task<ActivitySecrets> ResolveSecretsAsync(Session session, TaskLease lease, int protocolMinor, CancellationToken cancellationToken)
    {
        if (protocolMinor < 2)
        {
            throw new SagatException(SagatErrorCode.UnsupportedProtocol, "the server speaks a protocol older than 1.2, which has no ResolveSecrets");
        }

        var request = new ResolveSecretsRequest
        {
            TaskId = lease.TaskId,
            RunId = lease.RunId,
            StepId = lease.StepId,
            Attempt = lease.Attempt,
            Generation = lease.Generation,
            LeaseToken = lease.LeaseToken,
            Traceparent = lease.Traceparent,
        };
        if (protocolMinor >= 5 && lease.Instance is not null)
        {
            request.Instance = lease.Instance;
        }

        var answer = await session.Rpc.ResolveSecretsAsync(
            request,
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
    private sealed class SecretsLoader(TaskLease lease, Session session, int protocolMinor) : IDisposable
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
                    : loaded ??= await ResolveSecretsAsync(session, lease, protocolMinor, cancellationToken).ConfigureAwait(false);
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

    private sealed record HandBackItem(TaskLease Lease, string Key, Session Session, int ProtocolMinor, TaskCompletionSource Done);

    private sealed class InFlight(TaskRuntime runtime)
    {
        internal TaskRuntime Runtime { get; } = runtime;

        internal TaskCompletionSource Completion { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
    }
}
