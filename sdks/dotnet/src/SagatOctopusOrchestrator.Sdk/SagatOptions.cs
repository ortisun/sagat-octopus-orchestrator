using System.Diagnostics;
using System.Security.Cryptography.X509Certificates;
using System.Text.Json;
using System.Text.Json.Serialization;
using Grpc.Core.Interceptors;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>
/// The configuration of the SDK (SPEC section 1): one shape for the client and for the worker host, so that a
/// service that is both configures its endpoint, namespace and credentials once. It binds from configuration
/// (<c>services.Configure&lt;SagatOptions&gt;(configuration.GetSection("Sagat"))</c>).
/// </summary>
[DebuggerDisplay("{ToString(),nq}")]
public sealed class SagatOptions
{
    /// <summary>Base URL of the REST API including <c>/v1</c>, e.g. <c>https://api.example/v1</c>. Required for a client.</summary>
    [JsonIgnore]
    public string? ApiUrl { get; set; }

    /// <summary>URL of the worker service, <c>https://host:port</c> (<c>http://</c> only for loopback or development). Required for a worker.</summary>
    [JsonIgnore]
    public string? GrpcUrl { get; set; }

    /// <summary>Slug of the namespace every call is made in. Required. The tenant is never configured: it is the token's.</summary>
    public string? Namespace { get; set; }

    /// <summary>A bearer token. Either this or <see cref="TokenProvider"/> is required. Never printed, serialized or shown by the debugger.</summary>
    [JsonIgnore]
    [DebuggerBrowsable(DebuggerBrowsableState.Never)]
    public string? Token { get; set; }

    /// <summary>
    /// A provider called before every request that returns a fresh token, so that a rotated one is used at once. When it throws, the call fails
    /// as <see cref="SagatErrorCode.Unauthenticated"/> with the reason <c>TOKEN_PROVIDER_FAILED</c> (and is repeated like a transport failure);
    /// the provider's own exception is not kept.
    /// </summary>
    [JsonIgnore]
    [DebuggerBrowsable(DebuggerBrowsableState.Never)]
    public Func<CancellationToken, ValueTask<string>>? TokenProvider { get; set; }

    /// <summary>Deadline of one REST call or one unary RPC. 30 s by default.</summary>
    public TimeSpan Timeout { get; set; } = TimeSpan.FromSeconds(30);

    /// <summary>Attempts and backoff of the calls that are safe to repeat.</summary>
    public RetryPolicy Retry { get; set; } = new();

    /// <summary>Appended to <c>sagat-sdk-csharp/&lt;version&gt;</c> in the <c>User-Agent</c>.</summary>
    public string? UserAgent { get; set; }

    /// <summary>Trust roots, the server name and the client certificate (mTLS). The system roots when absent.</summary>
    [JsonIgnore]
    [DebuggerBrowsable(DebuggerBrowsableState.Never)]
    public SagatTlsOptions? Tls { get; set; }

    /// <summary>
    /// How the typed payloads (run input, signal payload, event data, activity input and output) are read and written.
    /// Camel-case, case-insensitive on read, by default.
    /// </summary>
    [JsonIgnore]
    public JsonSerializerOptions? Json { get; set; }

    /// <summary>
    /// The HTTP transport of the client, for a proxy, a test or fault injection. The caller owns it (the SDK does not dispose it).
    /// The SDK creates and owns a pooled <see cref="SocketsHttpHandler"/> (redirects off) when this is null. A redirect is never followed
    /// (it could carry the token to another host; a 3xx is <see cref="SagatErrorCode.Protocol"/>): a handler you supply must have
    /// <c>AllowAutoRedirect = false</c>, and the client refuses a <see cref="HttpClientHandler"/> or <see cref="SocketsHttpHandler"/>
    /// (also inside <see cref="DelegatingHandler"/>s) that follows them.
    /// </summary>
    [JsonIgnore]
    [DebuggerBrowsable(DebuggerBrowsableState.Never)]
    public HttpMessageHandler? HttpMessageHandler { get; set; }

    /// <summary>The worker host settings; used only by <see cref="Workers.WorkerHost"/>.</summary>
    public WorkerOptions Worker { get; } = new();

    /// <summary>The settings that are not a credential; a URL is shown by its host only (it can carry a credential in its user-info or query).</summary>
    public override string ToString() =>
        $"SagatOptions {{ ApiUrl = {HostOf(ApiUrl)}, GrpcUrl = {HostOf(GrpcUrl)}, Namespace = {Namespace}, Credential = {(Token is not null || TokenProvider is not null ? "set" : "none")}, Worker = {Worker.Name} }}";

    /// <summary>host[:port] of a URL and nothing else (user-info, path and query can carry a credential).</summary>
    internal static string HostOf(string? url) =>
        url is not null && Uri.TryCreate(url, UriKind.Absolute, out var uri) ? (uri.IsDefaultPort ? uri.Host : $"{uri.Host}:{uri.Port}") : "(none)";

    internal SagatOptions Snapshot()
    {
        var copy = new SagatOptions
        {
            ApiUrl = ApiUrl,
            GrpcUrl = GrpcUrl,
            Namespace = Namespace,
            Token = Token,
            TokenProvider = TokenProvider,
            Timeout = Timeout,
            Retry = Retry.Clone(),
            UserAgent = UserAgent,
            Tls = Tls,
            Json = Json,
            HttpMessageHandler = HttpMessageHandler,
        };
        copy.Worker.CopyFrom(Worker);
        return copy;
    }

    internal async ValueTask<string> ResolveTokenAsync(CancellationToken cancellationToken)
    {
        if (TokenProvider is { } provider)
        {
            string? token;
#pragma warning disable CA1031 // The provider is the caller's code: whatever it raises is "the provider failed", and its exception is not kept.
            try
            {
                token = await provider(cancellationToken).ConfigureAwait(false);
            }
            catch (Exception error) when (error is not OutOfMemoryException)
            {
                cancellationToken.ThrowIfCancellationRequested();
                throw SagatException.TokenProviderFailure("the token provider failed");
            }
#pragma warning restore CA1031

            return string.IsNullOrEmpty(token)
                ? throw SagatException.TokenProviderFailure("the token provider returned no token")
                : token;
        }

        return Token ?? throw new SagatException(SagatErrorCode.Unauthenticated, "no token is configured");
    }

    internal void ValidateCommon()
    {
        if (string.IsNullOrWhiteSpace(Namespace))
        {
            throw SagatException.Invalid("Namespace is required");
        }

        if (string.IsNullOrEmpty(Token) && TokenProvider is null)
        {
            throw SagatException.Invalid("a Token or a TokenProvider is required");
        }

        if (Timeout <= TimeSpan.Zero)
        {
            throw SagatException.Invalid("Timeout must be positive");
        }

        if (Retry is null || Retry.MaxAttempts < 1)
        {
            throw SagatException.Invalid("Retry.MaxAttempts must be at least 1");
        }

        if (Retry.InitialDelay < TimeSpan.Zero || Retry.MaxDelay < TimeSpan.Zero)
        {
            throw SagatException.Invalid("Retry.InitialDelay and Retry.MaxDelay cannot be negative");
        }

        Internal.Tls.Validate(Tls);
    }
}

/// <summary>The worker host settings (<see cref="SagatOptions.Worker"/>).</summary>
public sealed class WorkerOptions
{
    /// <summary>The worker's name; one live session per name and identity: registering again replaces it. Required.</summary>
    public string? Name { get; set; }

    /// <summary>Tasks in flight at once, 1 to 1000. 10 by default.</summary>
    public int MaxInflight { get; set; } = 10;

    /// <summary>The <c>languages</c> capability. <c>csharp</c> by default.</summary>
    public IList<string> Languages { get; } = [SdkInfo.Language];

    /// <summary>The <c>pools</c> capability.</summary>
    public IList<string> Pools { get; } = [];

    /// <summary>How long a stop waits for the tasks in flight before it cancels them with the reason <c>shutdown</c>. 30 s by default.</summary>
    public TimeSpan ShutdownTimeout { get; set; } = TimeSpan.FromSeconds(30);

    /// <summary>Overrides the newest protocol minor the host offers (tests use it to play an older or a newer SDK).</summary>
    public string? ProtocolVersion { get; set; }

    /// <summary>Overrides the heartbeat interval (otherwise a third of the lease, 500 ms to 10 s).</summary>
    public TimeSpan? HeartbeatInterval { get; set; }

    /// <summary>Interceptors of the gRPC channel: the hook of tests and of fault injection.</summary>
    [JsonIgnore]
    public IList<Interceptor> Interceptors { get; } = [];

    internal void CopyFrom(WorkerOptions other)
    {
        Name = other.Name;
        MaxInflight = other.MaxInflight;
        ReplaceAll(Languages, other.Languages);
        ReplaceAll(Pools, other.Pools);
        ShutdownTimeout = other.ShutdownTimeout;
        ProtocolVersion = other.ProtocolVersion;
        HeartbeatInterval = other.HeartbeatInterval;
        ReplaceAll(Interceptors, other.Interceptors);
    }

    private static void ReplaceAll<T>(IList<T> target, IList<T> source)
    {
        target.Clear();
        foreach (var item in source)
        {
            target.Add(item);
        }
    }
}

/// <summary>TLS settings of the REST client and of the worker channel.</summary>
[DebuggerDisplay("{ToString(),nq}")]
public sealed class SagatTlsOptions
{
    /// <summary>
    /// Trust roots used instead of the system roots to validate the server's certificate. The chain, the validity dates and the host
    /// name are still verified; there is no way to skip them. An empty collection is refused (it is not "the system roots": leave this null for those).
    /// </summary>
    [JsonIgnore]
    public X509Certificate2Collection? TrustedRoots { get; set; }

    /// <summary>The client certificate (with its private key) presented to the server for mTLS.</summary>
    [JsonIgnore]
    [DebuggerBrowsable(DebuggerBrowsableState.Never)]
    public X509Certificate2? ClientCertificate { get; set; }

    /// <summary>
    /// The name the server's certificate is verified against (and sent as the server name indication), when it is not the host of the URL.
    /// It is verified, never skipped: a certificate that does not carry the name is refused.
    /// </summary>
    public string? ServerName { get; set; }

    /// <summary>The settings, without the certificates themselves.</summary>
    public override string ToString() =>
        $"SagatTlsOptions {{ TrustedRoots = {TrustedRoots?.Count ?? 0}, ClientCertificate = {(ClientCertificate is null ? "none" : "set")}, ServerName = {ServerName ?? "(the host of the URL)"} }}";
}
