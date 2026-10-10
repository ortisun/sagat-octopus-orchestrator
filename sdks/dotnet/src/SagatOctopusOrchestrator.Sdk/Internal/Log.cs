using Microsoft.Extensions.Logging;

namespace SagatOctopusOrchestrator.Sdk.Internal;

/// <summary>
/// Every log line of the SDK. Fixed templates with identifiers only (method, path template, code, run, task, node, attempt):
/// never a token, a lease token, a session token, a secret or a payload.
/// </summary>
internal static partial class Log
{
    [LoggerMessage(EventId = 1, Level = LogLevel.Warning, Message = "Retrying {Method} {Path}: {Code}, attempt {Attempt}, waiting {WaitMs} ms")]
    internal static partial void RetryingRequest(ILogger logger, string method, string path, string code, int attempt, long waitMs);

    [LoggerMessage(EventId = 2, Level = LogLevel.Warning, Message = "The connection to {Host} is plain text outside loopback: the token and the payloads travel unencrypted")]
    internal static partial void InsecureTransport(ILogger logger, string host);

    [LoggerMessage(EventId = 29, Level = LogLevel.Warning, Message = "The poll answered {Got} tasks but the host asked for {Asked}: running the first {Asked} and leaving the {Ignored} others to lapse")]
    internal static partial void TooManyTasks(ILogger logger, int got, int asked, int ignored);

    [LoggerMessage(EventId = 10, Level = LogLevel.Information, Message = "Worker {Worker} registered at protocol {Protocol}")]
    internal static partial void WorkerRegistered(ILogger logger, string worker, string protocol);

    [LoggerMessage(EventId = 11, Level = LogLevel.Warning, Message = "The worker session is gone: registering again")]
    internal static partial void SessionGone(ILogger logger);

    [LoggerMessage(EventId = 12, Level = LogLevel.Error, Message = "Registering again failed: {Code}")]
    internal static partial void RegisteringAgainFailed(ILogger logger, string code);

    [LoggerMessage(EventId = 13, Level = LogLevel.Error, Message = "The poll loop failed: {ErrorType}")]
    internal static partial void PollLoopFailed(ILogger logger, string errorType);

    [LoggerMessage(EventId = 14, Level = LogLevel.Warning, Message = "A poll failed ({Code}): backing off {WaitMs} ms")]
    internal static partial void PollFailed(ILogger logger, string code, long waitMs);

    [LoggerMessage(EventId = 20, Level = LogLevel.Warning, Message = "The lease of task {TaskId} was lost: the result is dropped")]
    internal static partial void LeaseLost(ILogger logger, string taskId);

    [LoggerMessage(EventId = 21, Level = LogLevel.Warning, Message = "A heartbeat of task {TaskId} failed ({Code}): trying again on the next beat")]
    internal static partial void HeartbeatFailed(ILogger logger, string taskId, string code);

    [LoggerMessage(EventId = 22, Level = LogLevel.Error, Message = "The completion of task {TaskId} could not be reported: {Code}")]
    internal static partial void CompletionNotReported(ILogger logger, string taskId, string code);

    [LoggerMessage(EventId = 23, Level = LogLevel.Error, Message = "The failure of task {TaskId} could not be reported: {Code}")]
    internal static partial void FailureNotReported(ILogger logger, string taskId, string code);

    [LoggerMessage(EventId = 24, Level = LogLevel.Debug, Message = "Task {TaskId} taken: activity {Activity}, attempt {Attempt}")]
    internal static partial void TaskTaken(ILogger logger, string taskId, string activity, int attempt);

    [LoggerMessage(EventId = 25, Level = LogLevel.Debug, Message = "Task {TaskId} reported as {Outcome}")]
    internal static partial void TaskReported(ILogger logger, string taskId, string outcome);

    [LoggerMessage(EventId = 28, Level = LogLevel.Error, Message = "Task {TaskId} failed unexpectedly in the host: {ErrorType}")]
    internal static partial void TaskCrashed(ILogger logger, string taskId, string errorType);

    [LoggerMessage(EventId = 27, Level = LogLevel.Warning, Message = "Task {TaskId} stopped for the shutdown: nothing is reported and its lease will lapse")]
    internal static partial void ShutdownAbandoned(ILogger logger, string taskId);

    [LoggerMessage(EventId = 26, Level = LogLevel.Warning, Message = "The tasks still in flight were cancelled with the reason shutdown")]
    internal static partial void ShutdownCancelled(ILogger logger);
}
