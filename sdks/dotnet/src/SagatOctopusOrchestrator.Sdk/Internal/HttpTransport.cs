using System.Buffers;
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
    bool SafeToRepeat,
    HttpContent? RawContent = null,
    IReadOnlyDictionary<string, string>? Headers = null);

internal sealed record HttpAnswer(int Status, string TraceId, JsonElement? Body);

/// <summary>The REST transport: one place for headers, error mapping, spans and the retry rules of SPEC section 5.</summary>
internal sealed class HttpTransport : IDisposable
{
    private static readonly HashSet<string> RestCodes =
    [
        "BAD_REQUEST", "UNAUTHENTICATED", "FORBIDDEN", "NOT_FOUND", "CONFLICT", "VALIDATION_FAILED",
        "RATE_LIMITED", "QUOTA_EXCEEDED", "UNAVAILABLE", "INTERNAL",
    ];

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
            : new HttpClient(Tls.WithServerName(Tls.CreateHandler(options.Tls, options.Timeout), options.Tls), disposeHandler: true);
        http.Timeout = Timeout.InfiniteTimeSpan;
        if (Tls.IsPlainTextOutsideLoopback(baseUri))
        {
            // The host and nothing else: a URL can carry a credential in its user-info or its query.
            Log.InsecureTransport(logger, baseUri.Host);
        }
    }

    public void Dispose() => http.Dispose();

    internal async Task<HttpAnswer> SendAsync(HttpCall call, CancellationToken cancellationToken)
    {
        var activity = StartSpan(call);
        try
        {
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
                        Safe.Tag(activity, "http.response.status_code", answer.Status);
                        Safe.Tag(activity, "sagat.attempts", n + 1);
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
                Safe.Fail(activity);
                Safe.Tag(activity, "error.type", error.Code.ToWireName());
                if (error.HttpStatus is { } status)
                {
                    Safe.Tag(activity, "http.response.status_code", status);
                }

                throw;
            }
            catch (OperationCanceledException)
            {
                Safe.Fail(activity, "cancelled");
                throw;
            }
        }
        finally
        {
            Safe.Stop(activity);
        }
    }

    private Activity? StartSpan(HttpCall call)
    {
        var name = $"sagat {call.Operation}";
        var activity = call.Traceparent is { } explicitParent && ActivityContext.TryParse(explicitParent, null, out var parent)
            ? Safe.Start(name, ActivityKind.Client, parent)
            : Safe.Start(name, ActivityKind.Client);
        Safe.Tag(activity, "sagat.operation", call.Operation);
        Safe.Tag(activity, "sagat.namespace", options.Namespace);
        Safe.Tag(activity, "http.request.method", call.Method.Method);
        Safe.Tag(activity, "url.template", call.PathTemplate);
        return activity;
    }

    private async Task<HttpAnswer> OnceAsync(HttpCall call, string? traceparent, CancellationToken cancellationToken)
    {
        var token = await options.ResolveTokenAsync(cancellationToken).ConfigureAwait(false);

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
        else if (call.RawContent is not null)
        {
            request.Content = call.RawContent;
        }

        if (call.Headers is not null)
        {
            foreach (var (name, value) in call.Headers)
            {
                request.Headers.TryAddWithoutValidation(name, value);
            }
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
            var headerTrace = response.Headers.NonValidated.TryGetValues("X-Trace-Id", out var traces) ? Limits.TraceId(traces.ToString()) : string.Empty;
            var status = (int)response.StatusCode;
            string text;
            try
            {
                text = await ReadBodyAsync(response, status, headerTrace, deadline.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
                throw new SagatException(SagatErrorCode.Transport, "the answer was cut off", httpStatus: status, traceId: headerTrace);
            }
            catch (Exception error) when (error is HttpRequestException or IOException or InvalidOperationException)
            {
                throw new SagatException(SagatErrorCode.Transport, "the answer was cut off", httpStatus: status, traceId: headerTrace);
            }

            if (status is >= 200 and < 300)
            {
                if (text.Length == 0)
                {
                    return new HttpAnswer(status, headerTrace, null);
                }

                try
                {
                    using var document = JsonDocument.Parse(text);
                    return new HttpAnswer(status, headerTrace, document.RootElement.Clone());
                }
                catch (JsonException)
                {
                    throw new SagatException(SagatErrorCode.Protocol, "the answer is not JSON", httpStatus: status, traceId: headerTrace);
                }
            }

            if (status is >= 300 and < 400)
            {
                // Never followed, and not an answer to the call either: the URL is wrong (or the server is not the one it should be).
                throw new SagatException(SagatErrorCode.Protocol, "the server redirected the call", httpStatus: status, traceId: headerTrace);
            }

            throw ErrorOf(response, status, text, headerTrace);
        }
    }

    /// <summary>Reads the body as the server streams it, and stops at <see cref="Limits.MaxBodyBytes"/>: a server cannot make the client hold what it likes.</summary>
    private static async Task<string> ReadBodyAsync(HttpResponseMessage response, int status, string traceId, CancellationToken cancellationToken)
    {
        if (response.Content.Headers.ContentLength is { } declared && declared > Limits.MaxBodyBytes)
        {
            throw TooLarge(status, traceId);
        }

        var stream = await response.Content.ReadAsStreamAsync(cancellationToken).ConfigureAwait(false);
        await using (stream.ConfigureAwait(false))
        {
            using var collected = new MemoryStream();
            var chunk = ArrayPool<byte>.Shared.Rent(80 * 1024);
            try
            {
                int read;
                while ((read = await stream.ReadAsync(chunk, cancellationToken).ConfigureAwait(false)) > 0)
                {
                    if (collected.Length + read > Limits.MaxBodyBytes)
                    {
                        throw TooLarge(status, traceId);
                    }

                    collected.Write(chunk, 0, read);
                }
            }
            finally
            {
                ArrayPool<byte>.Shared.Return(chunk);
            }

            return Encoding.UTF8.GetString(collected.GetBuffer(), 0, (int)collected.Length);
        }
    }

    private static SagatException TooLarge(int status, string traceId) =>
        new(SagatErrorCode.Protocol, "the answer is larger than the SDK reads", httpStatus: status, traceId: traceId);

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

    private static SagatException ErrorOf(HttpResponseMessage response, int status, string text, string headerTrace)
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

        string? reason = null;
        if (envelope is { } found && found.TryGetProperty("details", out var details) && details.ValueKind == JsonValueKind.Object)
        {
            reason = StringOf(details, "reason");
        }

        // The envelope's trace id when it is one, else the header's, else none: a trace id that is not 32 hex characters is not repeated.
        var envelopeTrace = envelope is { } traced ? Limits.TraceId(StringOf(traced, "traceId")) : string.Empty;
        var traceId = envelopeTrace.Length > 0 ? envelopeTrace : headerTrace;

        // A call the server does not have (an older server): a 501; a 404 that says ROUTE_NOT_FOUND or is not the platform's envelope at all;
        // a 405 that says METHOD_NOT_ALLOWED. A 404 of the envelope for a run that does not exist is a plain NOT_FOUND.
        if (status == 501
            || (status == 404 && (given is null || reason == "ROUTE_NOT_FOUND"))
            || (status == 405 && reason == "METHOD_NOT_ALLOWED"))
        {
            return new SagatException(
                SagatErrorCode.NotFound,
                "the server does not have this call",
                httpStatus: status,
                traceId: traceId,
                reason: "UNSUPPORTED_BY_SERVER",
                retryAfter: retryAfter);
        }

        var code = given ?? SagatErrorCodes.FromHttpStatus(status);
        var message = given is not null && envelope is { } withMessage && StringOf(withMessage, "message") is { } serverMessage
            ? serverMessage
            : $"the server answered {status.ToString(CultureInfo.InvariantCulture)}";
        bool? retryable = given is not null && envelope is { } flagged && flagged.TryGetProperty("retryable", out var flag) && flag.ValueKind is JsonValueKind.True or JsonValueKind.False
            ? flag.GetBoolean()
            : null;
        return new SagatException(
            code,
            message,
            httpStatus: status,
            retryable: retryable,
            traceId: traceId,
            reason: reason,
            retryAfter: retryAfter);
    }

    /// <summary><c>retry-after-ms</c> or <c>Retry-After</c> in seconds, nothing else (an HTTP date is ignored), capped at a minute.</summary>
    private static TimeSpan? RetryAfterOf(HttpResponseMessage response)
    {
        if (response.Headers.NonValidated.TryGetValues("retry-after-ms", out var millis) && Limits.MillisecondsOf(millis.ToString()) is { } fromMillis)
        {
            return fromMillis;
        }

        return response.Headers.NonValidated.TryGetValues("Retry-After", out var seconds) ? Limits.SecondsOf(seconds.ToString()) : null;
    }

    private static string? StringOf(JsonElement element, string name) =>
        element.ValueKind == JsonValueKind.Object && element.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;
}
