using System.Text.Json;
using SagatOctopusOrchestrator.Sdk.Internal;
using static SagatOctopusOrchestrator.Sdk.Internal.Answers;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>What to do when an open execution already holds the business identifier a start asks for (<c>businessIdConflictPolicy</c>).</summary>
public static class BusinessIdConflictPolicies
{
    /// <summary>The start is refused with <c>CONFLICT</c> (the default).</summary>
    public const string Fail = "FAIL";

    /// <summary>Nothing is created: the open run is answered, and <see cref="StartedRun.Started"/> is false.</summary>
    public const string UseExisting = "USE_EXISTING";
}

/// <summary>What to do when a closed execution still remembers the business identifier a start asks for (<c>businessIdReusePolicy</c>).</summary>
public static class BusinessIdReusePolicies
{
    /// <summary>The identifier may be used again (the default).</summary>
    public const string AllowDuplicate = "ALLOW_DUPLICATE";

    /// <summary>The identifier may be used again only if the closed execution failed.</summary>
    public const string AllowDuplicateFailedOnly = "ALLOW_DUPLICATE_FAILED_ONLY";

    /// <summary>The identifier may not be used again while the closed execution is remembered.</summary>
    public const string RejectDuplicate = "REJECT_DUPLICATE";
}

/// <summary>How a business identifier stands in a namespace.</summary>
public sealed record BusinessIdBinding
{
    /// <summary>The identifier, as it was given.</summary>
    public required string BusinessId { get; init; }

    /// <summary>The run that holds the identifier, or the initial run of the continuation chain that does.</summary>
    public required string ExecutionId { get; init; }

    /// <summary>The run that carries the work now.</summary>
    public required string CurrentRunId { get; init; }

    /// <summary>Whether the execution is still open.</summary>
    public bool Open { get; init; }

    /// <summary>How the execution ended (a run status), or <c>null</c> while it is open.</summary>
    public string? LastStatus { get; init; }

    /// <summary>The workflow of the execution.</summary>
    public required string WorkflowId { get; init; }

    /// <summary>When the identifier was bound (UTC).</summary>
    public required DateTimeOffset BoundAt { get; init; }

    /// <summary>Until when a closed holder is remembered (UTC): the end of the execution plus the reuse window.</summary>
    public DateTimeOffset? ReusableAfter { get; init; }

    /// <summary>The answer as the server sent it.</summary>
    public JsonElement Raw { get; init; }
}

/// <summary>The business identifiers of a namespace (feature 018).</summary>
[System.Diagnostics.DebuggerDisplay("{ToString(),nq}")]
public sealed class BusinessIdsClient
{
    [System.Diagnostics.DebuggerBrowsable(System.Diagnostics.DebuggerBrowsableState.Never)]
    private readonly HttpTransport transport;

    internal BusinessIdsClient(HttpTransport transport) => this.transport = transport;

    /// <summary>The group; it shows nothing of the client that holds it.</summary>
    public override string ToString() => nameof(BusinessIdsClient);

    /// <summary>
    /// How an identifier stands in a namespace. An identifier nobody holds (or that cannot be one) is <c>NOT_FOUND</c>. It is sent as given, percent-encoded:
    /// no trimming and no case folding; only a value that is not 1 to 200 visible ASCII characters is refused here, before anything is sent.
    /// </summary>
    public async Task<BusinessIdBinding> GetAsync(string namespaceId, string businessId, RequestOptions? options = null, CancellationToken cancellationToken = default)
    {
        var path = "/namespaces/" + Requests.Uuid(namespaceId, "a namespace id") + "/business-ids/" + Requests.BusinessIdSegment(businessId);
        var answer = await transport.SendAsync(
            new HttpCall("get business id", HttpMethod.Get, path, "/namespaces/{namespaceId}/business-ids/{businessId}", [], null, null, SagatClient.TraceOf(options?.Traceparent), SafeToRepeat: true),
            cancellationToken).ConfigureAwait(false);
        var raw = ObjectOf(answer.Body, "a business id binding");
        return new BusinessIdBinding
        {
            BusinessId = Str(raw, "businessId"),
            ExecutionId = Str(raw, "executionId"),
            CurrentRunId = Str(raw, "currentRunId"),
            Open = Flag(raw, "open"),
            LastStatus = OptStr(raw, "lastStatus"),
            WorkflowId = Str(raw, "workflowId"),
            BoundAt = Date(raw, "boundAt"),
            ReusableAfter = OptDate(raw, "reusableAfter"),
            Raw = raw,
        };
    }
}
