using System.Diagnostics;
using System.Globalization;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using Microsoft.Extensions.Logging;

namespace SagatOctopusOrchestrator.Sdk.Internal;

internal sealed record HttpCall(
    string Operation,
    HttpMethod Method,
    string Path,
    string PathTemplate,
    IReadOnlyList<KeyValuePair<string, string>> Query,
    byte[]? Body,
    string? IdempotencyKey,
    string? Traceparent,
    bool SafeToRepeat);

internal sealed record HttpAnswer(int Status, string TraceId, JsonElement? Body);

/// <summary>The REST transport: one place for headers, error mapping, spans and the retry rules of SPEC section 5.</summary>
internal sealed class HttpTransport : IDisposable
{
    private static readonly HashSet<string> RestCodes =
    [
        "BAD_REQUEST", "UNAUTHENTICATED", "FORBIDDEN", "NOT_FOUND", "CONFLICT", "VALIDATION_FAILED",
        "RATE_LIMITED", "QUOTA_EXCEEDED", "UNAVAILABLE", "INTERNAL",
    ];

    private const int MaxServerMessage = 512;

    private readonly SagatOptions options;
    private readonly ILogger logger;
    private readonly HttpClient http;
    private readonly Uri baseUri;

    internal HttpTransport(SagatOptions options, ILogger logger)
    {
        this.options = options;
        this.logger = logger;
        baseUri = new Uri(options.ApiUrl!.TrimEnd('/'), UriKind.Absolute);
        Tls.RejectRedirects(options.HttpMessageHandler);
        http = options.HttpMessageHandler is { } handler
            ? new HttpClient(handler, disposeHandler: false)
            : new HttpClient(Tls.CreateHandler(options.Tls, options.Timeout), disposeHandler: true);
        http.Timeout = Timeout.InfiniteTimeSpan;
        if (Tls.IsPlainTextOutsideLoopback(baseUri))
        {
            Log.InsecureTransport(logger, baseUri.GetLeftPart(UriPartial.Authority));
        }
    }

    public void Dispose() => http.Dispose();

    internal async Task<HttpAnswer> SendAsync(HttpCall call, CancellationToken cancellationToken)
    {
        using var activity = StartSpan(call);
        // An explicit traceparent wins; else the span of this call (a child of the ambient one); else the ambient one; else none.
        var traceparent = call.Traceparent ?? Telemetry.CurrentTraceparent();
        var attempts = call.SafeToRepeat ? Math.Max(1, options.Retry.MaxAttempts) : 1;
        SagatException? last = null;
        try
        {
            for (var n = 0; n < attempts; n++)
            {
                try
                {
                    var answer = await OnceAsync(call, traceparent, cancellationToken).ConfigureAwait(false);
                    activity?.SetTag("http.response.status_code", answer.Status);
                    activity?.SetTag("sagat.attempts", n + 1);
                    return answer;
                }
                catch (SagatException error)
                {
                    last = error;
                    if (!error.Retryable || n + 1 >= attempts)
                    {
                        throw;
                    }

                    var wait = options.Retry.DelayAfter(n, error);
                    Log.RetryingRequest(logger, call.Method.Method, call.PathTemplate, error.Code.ToWireName(), n + 1, (long)wait.TotalMilliseconds);
                    await Task.Delay(wait, cancellationToken).ConfigureAwait(false);
                }
            }

            throw last ?? new SagatException(SagatErrorCode.Internal, "no attempt was made");
        }
        catch (SagatException error)
        {
            activity?.SetStatus(ActivityStatusCode.Error);
            activity?.SetTag("error.type", error.Code.ToWireName());
            if (error.HttpStatus is { } status)
            {
                activity?.SetTag("http.response.status_code", status);
            }

            throw;
        }
        catch (OperationCanceledException)
        {
            activity?.SetStatus(ActivityStatusCode.Error, "cancelled");
            throw;
        }
    }

    private Activity? StartSpan(HttpCall call)
    {
        var name = $"sagat {call.Operation}";
        Activity? activity;
        if (call.Traceparent is { } explicitParent && ActivityContext.TryParse(explicitParent, null, out var parent))
        {
            activity = Telemetry.Source.StartActivity(name, ActivityKind.Client, parent);
        }
        else
        {
            activity = Telemetry.Source.StartActivity(name, ActivityKind.Client);
        }

        if (activity is not null)
        {
            activity.SetTag("sagat.operation", call.Operation);
            activity.SetTag("sagat.namespace", options.Namespace);
            activity.SetTag("http.request.method", call.Method.Method);
            activity.SetTag("url.template", call.PathTemplate);
        }

        return activity;
    }

    private async Task<HttpAnswer> OnceAsync(HttpCall call, string? traceparent, CancellationToken cancellationToken)
    {
        string token;
        try
        {
            token = await options.ResolveTokenAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (Exception error) when (error is not (OperationCanceledException or SagatException))
        {
            throw new SagatException(SagatErrorCode.Unauthenticated, "the token provider failed");
        }

        using var request = new HttpRequestMessage(call.Method, BuildUri(call));
        request.Headers.TryAddWithoutValidation("Authorization", "Bearer " + token);
        request.Headers.TryAddWithoutValidation("Accept", "application/json");
        request.Headers.TryAddWithoutValidation("X-Octopus-Namespace", options.Namespace);
        request.Headers.TryAddWithoutValidation("User-Agent", SdkInfo.UserAgent(options.UserAgent));
        if (traceparent is not null && Traceparent.IsValid(traceparent))
        {
            request.Headers.TryAddWithoutValidation("traceparent", traceparent);
        }

        if (call.IdempotencyKey is not null)
        {
            request.Headers.TryAddWithoutValidation("Idempotency-Key", call.IdempotencyKey);
        }

        if (call.Body is not null)
        {
            request.Content = new ByteArrayContent(call.Body);
            request.Content.Headers.ContentType = new MediaTypeHeaderValue("application/json");
        }

        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        deadline.CancelAfter(options.Timeout);

        HttpResponseMessage response;
        try
        {
            response = await http.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, deadline.Token).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            throw new SagatException(SagatErrorCode.Transport, "the request timed out");
        }
        catch (Exception error) when (error is HttpRequestException or IOException or InvalidOperationException)
        {
            throw new SagatException(SagatErrorCode.Transport, "no answer from the server");
        }

        using (response)
        {
            var traceId = response.Headers.TryGetValues("X-Trace-Id", out var values) ? values.FirstOrDefault() ?? string.Empty : string.Empty;
            var status = (int)response.StatusCode;
            string text;
            try
            {
                text = await response.Content.ReadAsStringAsync(deadline.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
                throw new SagatException(SagatErrorCode.Transport, "the answer was cut off", httpStatus: status, traceId: traceId);
            }
            catch (Exception error) when (error is HttpRequestException or IOException or InvalidOperationException)
            {
                throw new SagatException(SagatErrorCode.Transport, "the answer was cut off", httpStatus: status, traceId: traceId);
            }

            if (status is >= 200 and < 300)
            {
                if (text.Length == 0)
                {
                    return new HttpAnswer(status, traceId, null);
                }

                try
                {
                    using var document = JsonDocument.Parse(text);
                    return new HttpAnswer(status, traceId, document.RootElement.Clone());
                }
                catch (JsonException)
                {
                    throw new SagatException(SagatErrorCode.Protocol, "the answer is not JSON", httpStatus: status, traceId: traceId);
                }
            }

            if (status is >= 300 and < 400)
            {
                // Never followed, and not an answer to the call either: the URL is wrong (or the server is not the one it should be).
                throw new SagatException(SagatErrorCode.Protocol, "the server redirected the call", httpStatus: status, traceId: traceId);
            }

            throw ErrorOf(response, status, text, traceId);
        }
    }

    private Uri BuildUri(HttpCall call)
    {
        var builder = new StringBuilder(baseUri.AbsoluteUri.TrimEnd('/')).Append(call.Path);
        var separator = '?';
        foreach (var (key, value) in call.Query)
        {
            builder.Append(separator).Append(Uri.EscapeDataString(key)).Append('=').Append(Uri.EscapeDataString(value));
            separator = '&';
        }

        return new Uri(builder.ToString(), UriKind.Absolute);
    }

    private static SagatException ErrorOf(HttpResponseMessage response, int status, string text, string traceId)
    {
        var retryAfter = RetryAfterOf(response);
        JsonElement? envelope = null;
        try
        {
            using var parsed = JsonDocument.Parse(text);
            if (parsed.RootElement.ValueKind == JsonValueKind.Object)
            {
                envelope = parsed.RootElement.Clone();
            }
        }
        catch (JsonException)
        {
            // An intermediary's page: classify by status.
        }

        SagatErrorCode? given = null;
        if (envelope is { } body && StringOf(body, "code") is { } wire && RestCodes.Contains(wire) && SagatErrorCodes.TryParse(wire, out var parsedCode))
        {
            given = parsedCode;
        }

        var code = given ?? SagatErrorCodes.FromHttpStatus(status);
        string? reason = null;
        if (envelope is { } found && found.TryGetProperty("details", out var details) && details.ValueKind == JsonValueKind.Object)
        {
            reason = StringOf(details, "reason");
        }

        // A call the server does not have (an older server): 501, or a 404 that no envelope of ours explains.
        if (status == 501 || (status == 404 && given is null))
        {
            return new SagatException(
                SagatErrorCode.NotFound,
                "the server does not have this call",
                httpStatus: status,
                traceId: traceId,
                reason: "UNSUPPORTED_BY_SERVER",
                retryAfter: retryAfter);
        }

        var message = given is not null && envelope is { } withMessage && StringOf(withMessage, "message") is { } serverMessage
            ? Truncate(serverMessage)
            : $"the server answered {status.ToString(CultureInfo.InvariantCulture)}";
        bool? retryable = given is not null && envelope is { } flagged && flagged.TryGetProperty("retryable", out var flag) && flag.ValueKind is JsonValueKind.True or JsonValueKind.False
            ? flag.GetBoolean()
            : null;
        var envelopeTrace = envelope is { } traced ? StringOf(traced, "traceId") : null;
        return new SagatException(
            code,
            message,
            httpStatus: status,
            retryable: retryable,
            traceId: envelopeTrace ?? traceId,
            reason: reason,
            retryAfter: retryAfter);
    }

    private static TimeSpan? RetryAfterOf(HttpResponseMessage response)
    {
        if (response.Headers.TryGetValues("retry-after-ms", out var millis)
            && long.TryParse(millis.FirstOrDefault(), NumberStyles.None, CultureInfo.InvariantCulture, out var ms))
        {
            return TimeSpan.FromMilliseconds(ms);
        }

        var header = response.Headers.RetryAfter;
        if (header?.Delta is { } delta)
        {
            return delta;
        }

        if (header?.Date is { } date)
        {
            var wait = date - DateTimeOffset.UtcNow;
            return wait > TimeSpan.Zero ? wait : TimeSpan.Zero;
        }

        return null;
    }

    private static string? StringOf(JsonElement element, string name) =>
        element.ValueKind == JsonValueKind.Object && element.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;

    private static string Truncate(string text) => text.Length <= MaxServerMessage ? text : text[..MaxServerMessage];
}
