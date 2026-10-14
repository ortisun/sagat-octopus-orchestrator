using System.Net;
using System.Net.Http.Headers;
using System.Security.Cryptography;
using SagatOctopusOrchestrator.Sdk.Internal;
using SagatOctopusOrchestrator.Sdk.Workers;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>Optional server-side integrity preconditions for a staged run input.</summary>
public sealed record RunInputBlobOptions(string ContentType = "application/octet-stream", long? ExpectedSize = null, string? ExpectedSha256 = null);

public sealed partial class SagatClient
{
    /// <inheritdoc/>
    public async Task<BlobDescriptor> UploadRunInputAsync(Stream source, RunInputBlobOptions? options = null, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(source);
        options ??= new RunInputBlobOptions();
        if (options.ExpectedSize is < 0 || (options.ExpectedSha256 is not null && !System.Text.RegularExpressions.Regex.IsMatch(options.ExpectedSha256, "^[0-9a-f]{64}$")))
        {
            throw SagatException.Invalid("blob preconditions are malformed");
        }

        var content = new MeasuredBlobContent(source, options.ContentType);
        var headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        if (options.ExpectedSize is { } size)
        {
            headers["X-Octopus-Blob-Size"] = size.ToString(System.Globalization.CultureInfo.InvariantCulture);
        }
        if (options.ExpectedSha256 is { } sha256)
        {
            headers["X-Octopus-Blob-Sha256"] = sha256;
        }
        var answer = await transport.SendAsync(
            new HttpCall("upload-run-input", HttpMethod.Post, "/run-input-blobs", "/run-input-blobs", [], null, null, null,
                SafeToRepeat: false, RawContent: content, Headers: headers), cancellationToken).ConfigureAwait(false);
        var raw = Object(answer.Body, "a blob descriptor");
        var descriptor = new BlobDescriptor(Str(raw, "id"), raw.GetProperty("size").GetInt64(), Str(raw, "sha256"), Str(raw, "contentType"));
        Blobs.Validate(descriptor);
        if (!content.SourceEof || !UuidPattern().IsMatch(descriptor.Id) || descriptor.Size != content.Size || descriptor.Sha256 != content.Sha256)
        {
            throw new SagatException(SagatErrorCode.Protocol, "the run-input descriptor does not match the uploaded bytes");
        }
        return descriptor;
    }

    /// <inheritdoc/>
    public async Task<Run> StartWithBlobAsync(string workflow, BlobDescriptor descriptor, StartOptions? options = null, CancellationToken cancellationToken = default)
    {
        if (descriptor is null || string.IsNullOrEmpty(descriptor.Id) || !UuidPattern().IsMatch(descriptor.Id) ||
            descriptor.Size < 0 || string.IsNullOrEmpty(descriptor.Sha256) || descriptor.Sha256.Length != 64 ||
            descriptor.Sha256.Any(ch => ch is not (>= '0' and <= '9') and not (>= 'a' and <= 'f')) ||
            string.IsNullOrWhiteSpace(descriptor.ContentType))
        {
            throw SagatException.Invalid("a run-input blob descriptor is malformed");
        }
        options ??= new StartOptions();
        var traceparent = TraceOf(options.Traceparent);
        var key = KeyOf(options.IdempotencyKey);
        var workflowId = await ResolveWorkflowAsync(workflow, cancellationToken).ConfigureAwait(false);
        var deadline = options.DeadlineIn is { } span ? DateTimeOffset.UtcNow + span : options.Deadline;
        var body = Wire.Write(writer =>
        {
            writer.WriteStartObject();
            writer.WriteString("workflowId", workflowId);
            writer.WriteString("inputBlobRef", descriptor.Id);
            if (options.VersionId is { Length: > 0 } versionId)
            {
                writer.WriteString("versionId", versionId);
            }
            if (options.CorrelationId is not null)
            {
                writer.WriteString("correlationId", options.CorrelationId);
            }
            if (deadline is { } at)
            {
                writer.WriteString("deadline", Wire.Instant(at));
            }
            writer.WriteEndObject();
        });
        var answer = await transport.SendAsync(new HttpCall("start", HttpMethod.Post, "/runs", "/runs", [], body, key, traceparent, true), cancellationToken).ConfigureAwait(false);
        return ToRun(answer.Body);
    }
}

internal sealed class MeasuredBlobContent : HttpContent
{
    private readonly Stream source;
    private readonly IncrementalHash digest = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
    internal long Size { get; private set; }
    internal string Sha256 { get; private set; } = string.Empty;
    internal bool SourceEof { get; private set; }

    internal MeasuredBlobContent(Stream source, string contentType)
    {
        this.source = source;
        Headers.ContentType = new MediaTypeHeaderValue(string.IsNullOrWhiteSpace(contentType) ? "application/octet-stream" : contentType);
    }

    protected override bool TryComputeLength(out long length) { length = 0; return false; }

    protected override async Task SerializeToStreamAsync(Stream target, TransportContext? context)
        => await CopyAsync(target, CancellationToken.None).ConfigureAwait(false);

    protected override async Task SerializeToStreamAsync(Stream target, TransportContext? context, CancellationToken cancellationToken)
        => await CopyAsync(target, cancellationToken).ConfigureAwait(false);

    private async Task CopyAsync(Stream target, CancellationToken cancellationToken)
    {
        var buffer = new byte[64 * 1024];
        while (true)
        {
            var read = await source.ReadAsync(buffer, cancellationToken).ConfigureAwait(false);
            if (read == 0)
            {
                SourceEof = true;
                break;
            }
            await target.WriteAsync(buffer.AsMemory(0, read), cancellationToken).ConfigureAwait(false);
            digest.AppendData(buffer, 0, read);
            Size += read;
        }
        Sha256 = Convert.ToHexString(digest.GetHashAndReset()).ToLowerInvariant();
    }
}
