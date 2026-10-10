using System.Security.Cryptography.X509Certificates;
using System.Text.Json;
using Grpc.Core.Interceptors;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>
/// The configuration of the SDK (SPEC section 1): one shape for the client and for the worker host, so that a
/// service that is both configures its endpoint, namespace and credentials once. It binds from configuration
/// (<c>services.Configure&lt;SagatOptions&gt;(configuration.GetSection("Sagat"))</c>).
/// </summary>
public sealed class SagatOptions
{
    /// <summary>Base URL of the REST API including <c>/v1</c>, e.g. <c>https://api.example/v1</c>. Required for a client.</summary>
    public string? ApiUrl { get; set; }

    /// <summary>URL of the worker service, <c>https://host:port</c> (<c>http://</c> only for loopback or development). Required for a worker.</summary>
    public string? GrpcUrl { get; set; }

    /// <summary>Slug of the namespace every call is made in. Required. The tenant is never configured: it is the token's.</summary>
    public string? Namespace { get; set; }

    /// <summary>A bearer token. Either this or <see cref="TokenProvider"/> is required.</summary>
    public string? Token { get; set; }

    /// <summary>A provider called before every request that returns a fresh token, so that a rotated one is used at once.</summary>
    public Func<CancellationToken, ValueTask<string>>? TokenProvider { get; set; }

    /// <summary>Deadline of one REST call or one unary RPC. 30 s by default.</summary>
    public TimeSpan Timeout { get; set; } = TimeSpan.FromSeconds(30);

    /// <summary>Attempts and backoff of the calls that are safe to repeat.</summary>
    public RetryPolicy Retry { get; set; } = new();

    /// <summary>Appended to <c>sagat-sdk-csharp/&lt;version&gt;</c> in the <c>User-Agent</c>.</summary>
    public string? UserAgent { get; set; }

    /// <summary>Trust roots and the client certificate (mTLS). The system roots when absent.</summary>
    public SagatTlsOptions? Tls { get; set; }

    /// <summary>
    /// How the typed payloads (run input, signal payload, event data, activity input and output) are read and written.
    /// Camel-case, case-insensitive on read, by default.
    /// </summary>
    public JsonSerializerOptions? Json { get; set; }

    /// <summary>
    /// The HTTP transport of the client, for a proxy, a test or fault injection. The caller owns it (the SDK does not dispose it).
    /// The SDK creates and owns a pooled <see cref="SocketsHttpHandler"/> (redirects off) when this is null. A redirect is never followed
    /// (it could carry the token to another host; a 3xx is <see cref="SagatErrorCode.Protocol"/>): a handler you supply must have
    /// <c>AllowAutoRedirect = false</c>, and the client refuses a <see cref="HttpClientHandler"/> or <see cref="SocketsHttpHandler"/>
    /// (also inside <see cref="DelegatingHandler"/>s) that follows them.
    /// </summary>
    public HttpMessageHandler? HttpMessageHandler { get; set; }

    /// <summary>The worker host settings; used only by <see cref="Workers.WorkerHost"/>.</summary>
    public WorkerOptions Worker { get; } = new();

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
            var token = await provider(cancellationToken).ConfigureAwait(false);
            if (string.IsNullOrEmpty(token))
            {
                throw new SagatException(SagatErrorCode.Unauthenticated, "the token provider returned no token");
            }

            return token;
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
public sealed class SagatTlsOptions
{
    /// <summary>
    /// Trust roots used instead of the system roots to validate the server's certificate. The chain, the validity dates and the host
    /// name are still verified; there is no way to skip them.
    /// </summary>
    public X509Certificate2Collection? TrustedRoots { get; set; }

    /// <summary>The client certificate (with its private key) presented to the server for mTLS.</summary>
    public X509Certificate2? ClientCertificate { get; set; }
}
