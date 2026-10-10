using System.Text.Json.Nodes;
using Microsoft.Extensions.Logging;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workers;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>
/// SPEC section 1, TLS: trust roots, and a client certificate for mTLS, for the REST client (HttpClient) and for the worker channel
/// (gRPC), each against an in-process TLS server with throw-away certificates.
/// </summary>
public sealed class TlsTests
{
    private const string Run = """{"runId":"r","workflowId":"w","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T10:00:00Z"}""";

    private static SagatOptions RestOptions(FakeRestServer server, SagatTlsOptions? tls) => new()
    {
        ApiUrl = server.BaseUrl,
        Namespace = "p",
        Token = "t",
        Retry = { MaxAttempts = 1 },
        Timeout = TimeSpan.FromSeconds(10),
        Tls = tls,
    };

    private static WorkerHost WorkerOf(FakeWorkerServer server, SagatTlsOptions? tls, ILoggerFactory? logs = null)
    {
        var options = new SagatOptions { GrpcUrl = server.Url, Namespace = "p", Token = "t", Retry = { MaxAttempts = 1 }, Timeout = TimeSpan.FromSeconds(10), Tls = tls };
        options.Worker.Name = "tls-" + Guid.NewGuid().ToString("N");
        var host = new WorkerHost(options, logs);
        host.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(1));
        return host;
    }

    private static async Task AssertTransportAsync(Func<Task> call)
    {
        var error = await Assert.ThrowsAsync<SagatException>(call);
        Assert.Equal(SagatErrorCode.Transport, error.Code);
        Assert.NotEmpty(error.Remediation);
    }

    [Fact]
    public async Task TheRestClientRefusesAnUntrustedServerTrustsTheRootsItIsGivenAndNoOtherAndPresentsItsClientCertificate()
    {
        var certificate = TestCertificate.CreateServer();
        var other = TestCertificate.CreateServer("some-other-server");
        var client = TestCertificate.CreateClient();
        await using var server = await FakeRestServer.StartAsync(certificate, requireClientCertificate: true);
        server.Handler = _ => new FakeAnswer(200, Run);
        Assert.StartsWith("https://", server.BaseUrl, StringComparison.Ordinal);

        // A self-signed certificate is not in the system roots: refused, as a transport error.
        using (var system = new SagatClient(RestOptions(server, new SagatTlsOptions { ClientCertificate = client })))
        {
            await AssertTransportAsync(() => system.GetAsync("r"));
        }

        using (var noTls = new SagatClient(RestOptions(server, null)))
        {
            await AssertTransportAsync(() => noTls.GetAsync("r"));
        }

        // The root of another server does not make this one trusted.
        using (var wrongRoot = new SagatClient(RestOptions(server, new SagatTlsOptions { TrustedRoots = [other.Root], ClientCertificate = client })))
        {
            await AssertTransportAsync(() => wrongRoot.GetAsync("r"));
        }

        Assert.Empty(server.Requests);

        // The configured CA makes the same server work, and the server sees the client certificate.
        using (var good = new SagatClient(RestOptions(server, new SagatTlsOptions { TrustedRoots = [certificate.Root], ClientCertificate = client })))
        {
            Assert.Equal("r", (await good.GetAsync("r")).RunId);
        }

        Assert.Equal(client.Thumbprint, Assert.Single(server.Requests).ClientCertificate);
    }

    [Fact]
    public async Task ARestServerThatRequiresAClientCertificateRefusesAClientWithoutOne()
    {
        var certificate = TestCertificate.CreateServer();
        await using var server = await FakeRestServer.StartAsync(certificate, requireClientCertificate: true);
        server.Handler = _ => new FakeAnswer(200, Run);
        using var client = new SagatClient(RestOptions(server, new SagatTlsOptions { TrustedRoots = [certificate.Root] }));
        await AssertTransportAsync(() => client.GetAsync("r"));
        Assert.Empty(server.Requests);
    }

    [Fact]
    public async Task ATrustedRootDoesNotExcuseAnExpiredCertificateOrOneForAnotherHost()
    {
        var expired = TestCertificate.CreateServer("expired", expired: true);
        await using (var server = await FakeRestServer.StartAsync(expired))
        {
            server.Handler = _ => new FakeAnswer(200, Run);
            using var client = new SagatClient(RestOptions(server, new SagatTlsOptions { TrustedRoots = [expired.Root] }));
            await AssertTransportAsync(() => client.GetAsync("r"));
            Assert.Empty(server.Requests);
        }

        var elsewhere = TestCertificate.CreateServer("elsewhere", forOtherHost: true);
        await using (var server = await FakeRestServer.StartAsync(elsewhere))
        {
            server.Handler = _ => new FakeAnswer(200, Run);
            using var client = new SagatClient(RestOptions(server, new SagatTlsOptions { TrustedRoots = [elsewhere.Root] }));
            await AssertTransportAsync(() => client.GetAsync("r"));
            Assert.Empty(server.Requests);
        }
    }

    [Fact]
    public async Task TheWorkerChannelRefusesAnUntrustedServerTrustsTheRootsItIsGivenAndNoOtherAndPresentsItsClientCertificate()
    {
        var certificate = TestCertificate.CreateServer();
        var client = TestCertificate.CreateClient();
        await using var fake = await FakeWorkerServer.StartAsync(certificate, requireClientCertificate: true);
        Assert.StartsWith("https://", fake.Url, StringComparison.Ordinal);

        foreach (var tls in new SagatTlsOptions?[]
        {
            null,
            new SagatTlsOptions { ClientCertificate = client },
            new SagatTlsOptions { TrustedRoots = [TestCertificate.CreateServer("some-other-server").Root], ClientCertificate = client },
        })
        {
            await using var refused = WorkerOf(fake, tls);
            await AssertTransportAsync(() => refused.StartAsync());
        }

        Assert.Equal(0, fake.State.Registrations);

        await using var trusting = WorkerOf(fake, new SagatTlsOptions { TrustedRoots = [certificate.Root], ClientCertificate = client });
        await trusting.StartAsync();
        Assert.Equal("1.2", trusting.ProtocolVersion);
        await trusting.StopAsync();

        Assert.Equal(client.Thumbprint, fake.State.Of("RegisterWorker")[0].ClientCertificate);
    }

    [Fact]
    public async Task AWorkerServerThatRequiresAClientCertificateRefusesAWorkerWithoutOne()
    {
        var certificate = TestCertificate.CreateServer();
        await using var fake = await FakeWorkerServer.StartAsync(certificate, requireClientCertificate: true);
        await using var worker = WorkerOf(fake, new SagatTlsOptions { TrustedRoots = [certificate.Root] });
        await AssertTransportAsync(() => worker.StartAsync());
        Assert.Equal(0, fake.State.Registrations);
    }

    [Fact]
    public async Task TheWorkerChannelWorksWithoutClientCertificateWhenTheServerAsksForNone()
    {
        var certificate = TestCertificate.CreateServer();
        await using var fake = await FakeWorkerServer.StartAsync(certificate);
        await using var worker = WorkerOf(fake, new SagatTlsOptions { TrustedRoots = [certificate.Root] });
        await worker.StartAsync();
        Assert.Null(fake.State.Of("RegisterWorker")[0].ClientCertificate);
    }

    [Fact]
    public async Task NoPlainTextWarningIsLoggedForTlsAndNoCallEverAcceptsAnyServerCertificate()
    {
        var logs = new CapturingLoggerProvider();
        using var factory = LoggerFactory.Create(builder => builder.AddProvider(logs));
        var certificate = TestCertificate.CreateServer();
        await using var rest = await FakeRestServer.StartAsync(certificate);
        rest.Handler = _ => new FakeAnswer(200, Run);
        var options = RestOptions(rest, new SagatTlsOptions { TrustedRoots = [certificate.Root] });
        using var client = new SagatClient(options, factory.CreateLogger<SagatClient>());
        await client.GetAsync("r");

        await using var fake = await FakeWorkerServer.StartAsync(certificate);
        await using var worker = WorkerOf(fake, new SagatTlsOptions { TrustedRoots = [certificate.Root] }, factory);
        await worker.StartAsync();

        Assert.DoesNotContain(logs.Lines, line => line.Contains("plain text", StringComparison.Ordinal));
        Assert.Contains(logs.Lines, line => line.Contains("registered at protocol", StringComparison.Ordinal));
    }

    [Fact]
    public void TheSourceOfTheSdkHasNoCodeThatSwitchesCertificateOrHostNameVerificationOff()
    {
        var files = Directory.GetFiles(Repository.Path("sdks", "dotnet", "src"), "*.cs", SearchOption.AllDirectories)
            .Where(path => !path.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
            .ToList();
        Assert.NotEmpty(files);

        // Trust roots go in as a chain policy of the platform's own validation: no callback of any kind decides, no host name is
        // overridden (SslClientAuthenticationOptions.TargetHost), and nothing accepts "any" certificate.
        string[] forbidden =
        [
            "DangerousAcceptAnyServerCertificateValidator",
            "ServerCertificateCustomValidationCallback",
            "ServerCertificateValidationCallback",
            "CertificateValidationCallback",
            "TargetHost",
            "SslPolicyErrors",
            "AllowAnyClientCertificate",
            "CheckCertificateRevocationList = false",
        ];
        foreach (var file in files)
        {
            var text = File.ReadAllText(file);
            foreach (var token in forbidden)
            {
                Assert.False(text.Contains(token, StringComparison.Ordinal), $"{Path.GetFileName(file)} mentions {token}");
            }
        }
    }

    [Theory]
    [InlineData(301, "GET")]
    [InlineData(302, "GET")]
    [InlineData(303, "POST")]
    [InlineData(307, "POST")]
    [InlineData(308, "POST")]
    public async Task ARedirectIsNeverFollowedAndIsAProtocolErrorThatNeverReachesTheOtherServer(int status, string method)
    {
        await using var origin = await FakeRestServer.StartAsync();
        await using var elsewhere = await FakeRestServer.StartAsync();
        elsewhere.Handler = _ => new FakeAnswer(200, Run);
        origin.Handler = _ => new FakeAnswer(status, null, new Dictionary<string, string> { ["location"] = elsewhere.BaseUrl + "/runs/r" });
        using var client = new SagatClient(RestOptions(origin, null));

        var error = await Assert.ThrowsAsync<SagatException>(() => method == "GET"
            ? client.GetAsync("r")
            : client.StartAsync("0190aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee", new { }, new StartOptions { IdempotencyKey = "key-12345678" }));

        Assert.Equal(SagatErrorCode.Protocol, error.Code);
        Assert.Equal(status, error.HttpStatus);
        Assert.Equal("the server redirected the call", error.Message);
        Assert.False(error.Retryable);
        Assert.Single(origin.Requests);
        Assert.Empty(elsewhere.Requests);
    }

    [Fact]
    public async Task ARedirectIsNotRetriedEvenWhenTheCallCouldBe()
    {
        await using var origin = await FakeRestServer.StartAsync();
        origin.Handler = _ => new FakeAnswer(302, null, new Dictionary<string, string> { ["location"] = "http://127.0.0.1:9/elsewhere" });
        var options = RestOptions(origin, null);
        options.Retry.MaxAttempts = 4;
        using var client = new SagatClient(options);
        await Assert.ThrowsAsync<SagatException>(() => client.GetAsync("r"));
        Assert.Single(origin.Requests);
    }

    [Fact]
    public async Task AHandlerTheCallerSuppliesThatFollowsRedirectsIsRefusedAndOneThatDoesNotIsUsed()
    {
        await using var origin = await FakeRestServer.StartAsync();
        origin.Handler = _ => new FakeAnswer(200, Run);

        foreach (var following in new Func<HttpMessageHandler>[]
        {
            () => new HttpClientHandler(),
            () => new SocketsHttpHandler(),
            () => new PassThrough(new HttpClientHandler { AllowAutoRedirect = true }),
            () => new PassThrough(new PassThrough(new SocketsHttpHandler { AllowAutoRedirect = true })),
        })
        {
            var options = RestOptions(origin, null);
            options.HttpMessageHandler = following();
            var error = Assert.Throws<SagatException>(() => new SagatClient(options));
            Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
            Assert.Contains("AllowAutoRedirect", error.Message, StringComparison.Ordinal);
        }

        foreach (var staying in new Func<HttpMessageHandler>[]
        {
            () => new HttpClientHandler { AllowAutoRedirect = false },
            () => new SocketsHttpHandler { AllowAutoRedirect = false },
            () => new PassThrough(new SocketsHttpHandler { AllowAutoRedirect = false }),
        })
        {
            var options = RestOptions(origin, null);
            options.HttpMessageHandler = staying();
            using var client = new SagatClient(options);
            Assert.Equal("r", (await client.GetAsync("r")).RunId);
        }
    }

    private sealed class PassThrough(HttpMessageHandler inner) : DelegatingHandler(inner);
}
