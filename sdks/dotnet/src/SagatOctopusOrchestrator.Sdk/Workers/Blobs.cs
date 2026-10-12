using System.Runtime.CompilerServices;
using System.Security.Cryptography;
using System.Text.RegularExpressions;
using SagatOctopusOrchestrator.Protocol.V1;

namespace SagatOctopusOrchestrator.Sdk.Workers;

/// <summary>The verified identity of an immutable worker-protocol blob.</summary>
public sealed record BlobDescriptor(string Id, long Size, string Sha256, string ContentType);

/// <summary>Optional preconditions for a one-pass upload. Zero and empty values omit them.</summary>
public sealed record BlobWriteOptions(string ContentType = "", long ExpectedSize = 0, string ExpectedSha256 = "");

/// <summary>Explicitly selects a blob as an activity result. An ordinary object with these fields stays JSON.</summary>
public sealed record BlobOutput(BlobDescriptor Descriptor);

internal static partial class Blobs
{
    internal const int ChunkSize = 256 * 1024;

    [GeneratedRegex("^[0-9a-f]{64}$", RegexOptions.CultureInvariant)]
    private static partial Regex Sha256Pattern();

    internal static void Validate(BlobDescriptor descriptor)
    {
        if (descriptor is null || string.IsNullOrWhiteSpace(descriptor.Id) || descriptor.Size < 0 ||
            string.IsNullOrEmpty(descriptor.Sha256) || !Sha256Pattern().IsMatch(descriptor.Sha256) ||
            string.IsNullOrWhiteSpace(descriptor.ContentType))
        {
            throw new SagatException(SagatErrorCode.Protocol, "the blob descriptor is malformed");
        }
    }

    internal static async Task<BlobDescriptor> WriteAsync(
        WorkerRpc rpc,
        TaskLease lease,
        Stream source,
        BlobWriteOptions options,
        CancellationToken cancellationToken)
    {
        if (options.ExpectedSize < 0 || (!string.IsNullOrEmpty(options.ExpectedSha256) && !Sha256Pattern().IsMatch(options.ExpectedSha256)))
        {
            throw SagatException.Invalid("blob preconditions are malformed");
        }

        var header = new WriteBlobHeader
        {
            TaskId = lease.TaskId, RunId = lease.RunId, StepId = lease.StepId, Attempt = lease.Attempt,
            Generation = lease.Generation, LeaseToken = lease.LeaseToken, ContentType = options.ContentType,
            ExpectedSize = options.ExpectedSize, ExpectedSha256 = options.ExpectedSha256,
        };
        var (response, size, sha256) = await rpc.WriteBlobAsync(header, source, lease.Traceparent, cancellationToken).ConfigureAwait(false);
        var descriptor = new BlobDescriptor(response.BlobId, response.Size, response.Sha256, response.ContentType);
        Validate(descriptor);
        if (descriptor.Size != size || descriptor.Sha256 != sha256)
        {
            throw new SagatException(SagatErrorCode.Protocol, "the blob service descriptor does not match the uploaded bytes");
        }

        return descriptor;
    }

    internal static async IAsyncEnumerable<ReadOnlyMemory<byte>> ReadAsync(
        WorkerRpc rpc,
        TaskLease lease,
        BlobDescriptor descriptor,
        [EnumeratorCancellation] CancellationToken cancellationToken)
    {
        Validate(descriptor);
        using var digest = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
        long size = 0;
        var headerSeen = false;
        var request = new ReadBlobRequest
        {
            TaskId = lease.TaskId, RunId = lease.RunId, StepId = lease.StepId, Attempt = lease.Attempt,
            Generation = lease.Generation, LeaseToken = lease.LeaseToken, BlobId = descriptor.Id,
        };
        await foreach (var part in rpc.ReadBlobAsync(request, lease.Traceparent, cancellationToken).ConfigureAwait(false))
        {
            if (part.PartCase == ReadBlobResponse.PartOneofCase.Header)
            {
                var header = part.Header;
                if (headerSeen || size != 0 || header.BlobId != descriptor.Id || header.Size != descriptor.Size ||
                    header.Sha256 != descriptor.Sha256 || header.ContentType != descriptor.ContentType)
                {
                    throw new SagatException(SagatErrorCode.Protocol, "the blob stream header does not match its descriptor");
                }

                headerSeen = true;
                continue;
            }

            if (part.PartCase != ReadBlobResponse.PartOneofCase.Chunk || !headerSeen || part.Chunk.Length > ChunkSize)
            {
                throw new SagatException(SagatErrorCode.Protocol, "the blob stream has an invalid part order or chunk size");
            }

            var chunk = part.Chunk.Memory;
            size += chunk.Length;
            if (size > descriptor.Size)
            {
                throw new SagatException(SagatErrorCode.Protocol, "the blob stream is larger than its descriptor");
            }

            digest.AppendData(chunk.Span);
            yield return chunk;
        }

        var actual = Convert.ToHexString(digest.GetHashAndReset()).ToLowerInvariant();
        if (!headerSeen || size != descriptor.Size || actual != descriptor.Sha256)
        {
            throw new SagatException(SagatErrorCode.Protocol, "the blob stream ended with mismatched integrity metadata");
        }
    }
}
