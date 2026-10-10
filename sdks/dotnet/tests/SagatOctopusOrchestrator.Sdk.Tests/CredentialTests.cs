using System.Text.Json;
using System.Text.Json.Nodes;
using Google.Protobuf;
using SagatOctopusOrchestrator.Protocol.V1;
using SagatOctopusOrchestrator.Sdk.Tests.Support;
using SagatOctopusOrchestrator.Sdk.Workers;
using Xunit;

namespace SagatOctopusOrchestrator.Sdk.Tests;

/// <summary>SPEC section 10: a config, a client or a host never prints a credential; the secrets of a step are handed out as copies and wiped; the options are checked.</summary>
public sealed class CredentialTests
{
    private const string Canary = "CANARY-BEARER-TOKEN-9f3a";
    private const string Run = """{"runId":"0190aaaa-0000-7000-8000-000000000001","workflowId":"w","versionId":"v","status":"RUNNING","createdAt":"2026-10-10T10:00:00Z"}""";

    // --------------------------------------------------------------------------- 13. what an object shows of itself

    [Fact]
    public void TheStringFormTheJsonAndTheDebuggerFormOfTheOptionsShowNoCredential()
    {
        var certificate = TestCertificate.CreateClient();
        var options = new SagatOptions
        {
            ApiUrl = "https://api.example.test/v1",
            GrpcUrl = "https://worker.example.test",
            Namespace = "payments",
            Token = Canary,
            TokenProvider = _ => ValueTask.FromResult(Canary),
            Tls = new SagatTlsOptions { ClientCertificate = certificate, ServerName = "worker.example.test" },
        };
        options.Worker.Name = "w";

        var json = JsonSerializer.Serialize(options);
        Assert.DoesNotContain(Canary, json, StringComparison.Ordinal);
        Assert.DoesNotContain("PrivateKey", json, StringComparison.Ordinal);
        Assert.DoesNotContain(Canary, options.ToString(), StringComparison.Ordinal);
        Assert.DoesNotContain(Canary, options.Tls.ToString()!, StringComparison.Ordinal);
        Assert.DoesNotContain("PRIVATE KEY", options.Tls.ToString()!, StringComparison.Ordinal);
        Assert.DoesNotContain(Canary, JsonSerializer.Serialize(options.Tls), StringComparison.Ordinal);
        Assert.Contains("api.example.test", options.ToString(), StringComparison.Ordinal);

        using var client = new SagatClient(options);
        Assert.DoesNotContain(Canary, client.ToString()!, StringComparison.Ordinal);
        Assert.DoesNotContain(Canary, JsonSerializer.Serialize(client), StringComparison.Ordinal);
        var host = new WorkerHost(options);
        Assert.DoesNotContain(Canary, host.ToString()!, StringComparison.Ordinal);
        Assert.DoesNotContain(Canary, JsonSerializer.Serialize(host), StringComparison.Ordinal);
    }

    [Fact]
    public void TheMembersThatHoldACredentialAreHiddenFromTheDebugger()
    {
        foreach (var (type, member) in new[] { (typeof(SagatOptions), nameof(SagatOptions.Token)), (typeof(SagatOptions), nameof(SagatOptions.TokenProvider)) })
        {
            var attribute = type.GetProperty(member)!.GetCustomAttributes(typeof(System.Diagnostics.DebuggerBrowsableAttribute), inherit: false).Cast<System.Diagnostics.DebuggerBrowsableAttribute>().SingleOrDefault();
            Assert.True(attribute is { State: System.Diagnostics.DebuggerBrowsableState.Never }, $"{type.Name}.{member}");
        }

        foreach (var type in new[] { typeof(SagatOptions), typeof(SagatClient), typeof(WorkerHost), typeof(SagatTlsOptions) })
        {
            Assert.NotNull(type.GetCustomAttributes(typeof(System.Diagnostics.DebuggerDisplayAttribute), inherit: false).SingleOrDefault());
        }
    }

    // ---------------------------------------------------------------------------------- 13. the secrets of a step

    private static async Task<(FakeWorkerServer Fake, WorkerHost Host)> StartAsync(Action<SagatOptions>? change = null)
    {
        var fake = await FakeWorkerServer.StartAsync();
        fake.State.Secrets.Add(new ResolvedSecret { Name = "k", Value = ByteString.CopyFromUtf8("topsecret"), Version = 1 });
        var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "p", Token = "t" };
        options.Worker.Name = "creds-" + Guid.NewGuid().ToString("N");
        options.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(50);
        change?.Invoke(options);
        return (fake, new WorkerHost(options));
    }

    [Fact]
    public async Task TheSecretsAreHandedOutAsCopiesAndNeverTheBufferOfTheSdk()
    {
        var (fake, host) = await StartAsync();
        await using (fake)
        await using (host)
        {
            byte[]? first = null;
            byte[]? second = null;
            string? text = null;
            host.RegisterActivity<JsonNode?, int>("pay", async (_, context, ct) =>
            {
                var secrets = await context.GetSecretsAsync(ct);
                first = secrets.Get("k");
                Array.Clear(first);
                second = secrets.Get("k");
                text = secrets.GetText("k");
                Assert.NotSame(first, second);
                return 1;
            });
            fake.State.Lease(Leases.Lease(l => l.ActivityName = "pay"));
            await host.StartAsync();
            await Eventually.UntilAsync(() => fake.State.Of("CompleteStep").Count == 1, "the completion");
            Assert.Equal("topsecret", text);
            Assert.Equal("topsecret"u8.ToArray(), second);
        }
    }

    [Fact]
    public async Task TheSecretsAreWipedWhenTheHandlerReturnsBeforeTheResultIsReportedAndALaterRequestRaises()
    {
        var (fake, host) = await StartAsync();
        await using (fake)
        await using (host)
        {
            ActivitySecrets? kept = null;
            ActivityExecutionContext? context = null;
            var namesWhenReported = -1;
            fake.State.OnComplete = () => namesWhenReported = kept!.Names.Count;
            host.RegisterActivity<JsonNode?, int>("pay", async (_, c, ct) =>
            {
                context = c;
                kept = await c.GetSecretsAsync(ct);
                Assert.Equal(["k"], kept.Names);
                return 1;
            });
            fake.State.Lease(Leases.Lease(l => l.ActivityName = "pay"));
            await host.StartAsync();
            await Eventually.UntilAsync(() => fake.State.Of("CompleteStep").Count == 1, "the completion");
            Assert.Equal(0, namesWhenReported);
            Assert.Empty(kept!.Names);
            Assert.Throws<SagatException>(() => kept.Get("k"));
            var error = await Assert.ThrowsAsync<SagatException>(() => context!.GetSecretsAsync());
            Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
            Assert.Single(fake.State.Of("ResolveSecrets"));
        }
    }

    // ---------------------------------------------------------------------------------------- 14. the options

    [Theory]
    [InlineData(0)]
    [InlineData(-1)]
    [InlineData(-1000)]
    public async Task AHeartbeatIntervalOfZeroOrLessIsRefused(int milliseconds)
    {
        var options = new SagatOptions { GrpcUrl = "http://127.0.0.1:1", Namespace = "p", Token = "t" };
        options.Worker.Name = "n";
        options.Worker.HeartbeatInterval = TimeSpan.FromMilliseconds(milliseconds);
        await using var host = new WorkerHost(options);
        host.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(1));
        var error = await Assert.ThrowsAsync<SagatException>(() => host.StartAsync());
        Assert.Equal(SagatErrorCode.ValidationFailed, error.Code);
        Assert.Contains("HeartbeatInterval", error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public async Task AnEmptyCollectionOfTrustedRootsIsRefusedAndNotTakenForTheSystemRoots()
    {
        var empty = new SagatTlsOptions { TrustedRoots = [] };
        var client = Assert.Throws<SagatException>(() => new SagatClient(new SagatOptions { ApiUrl = "https://api.example.test/v1", Namespace = "p", Token = "t", Tls = empty }));
        Assert.Equal(SagatErrorCode.ValidationFailed, client.Code);
        Assert.Contains("TrustedRoots", client.Message, StringComparison.Ordinal);

        var options = new SagatOptions { GrpcUrl = "https://worker.example.test", Namespace = "p", Token = "t", Tls = empty };
        options.Worker.Name = "n";
        await using var host = new WorkerHost(options);
        host.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(1));
        var error = await Assert.ThrowsAsync<SagatException>(() => host.StartAsync());
        Assert.Contains("TrustedRoots", error.Message, StringComparison.Ordinal);
    }

    // ------------------------------------------------------------------------------------- 15. the server name

    private static SagatTlsOptions Trusting(TestCertificate certificate, string? serverName) =>
        new() { TrustedRoots = [certificate.Root], ServerName = serverName };

    [Fact]
    public async Task TheRestClientVerifiesTheCertificateAgainstTheServerNameItIsGiven()
    {
        var certificate = TestCertificate.CreateServer("by-name", forOtherHost: true);
        await using var server = await FakeRestServer.StartAsync(certificate);
        server.Handler = _ => new FakeAnswer(200, Run);

        // The URL is an address the certificate does not name; the name that it does name is given and verified.
        using (var named = new SagatClient(new SagatOptions { ApiUrl = server.BaseUrl, Namespace = "p", Token = "t", Retry = { MaxAttempts = 1 }, Tls = Trusting(certificate, "other-host.example.test") }))
        {
            Assert.Equal("RUNNING", (await named.GetAsync("0190aaaa-0000-7000-8000-000000000001")).Status);
        }

        // A name the certificate does not carry is refused: the option is verified, not skipped.
        using (var wrong = new SagatClient(new SagatOptions { ApiUrl = server.BaseUrl, Namespace = "p", Token = "t", Retry = { MaxAttempts = 1 }, Tls = Trusting(certificate, "wrong.example.test") }))
        {
            var error = await Assert.ThrowsAsync<SagatException>(() => wrong.GetAsync("0190aaaa-0000-7000-8000-000000000001"));
            Assert.Equal(SagatErrorCode.Transport, error.Code);
        }

        // Without it the address of the URL is checked, and the certificate does not name it.
        using (var without = new SagatClient(new SagatOptions { ApiUrl = server.BaseUrl, Namespace = "p", Token = "t", Retry = { MaxAttempts = 1 }, Tls = Trusting(certificate, null) }))
        {
            await Assert.ThrowsAsync<SagatException>(() => without.GetAsync("0190aaaa-0000-7000-8000-000000000001"));
        }

        Assert.Single(server.Requests);
    }

    [Fact]
    public async Task TheWorkerChannelVerifiesTheCertificateAgainstTheServerNameItIsGiven()
    {
        var certificate = TestCertificate.CreateServer("by-name", forOtherHost: true);
        await using var fake = await FakeWorkerServer.StartAsync(certificate);

        WorkerHost Host(string? serverName)
        {
            var options = new SagatOptions { GrpcUrl = fake.Url, Namespace = "p", Token = "t", Retry = { MaxAttempts = 1 }, Tls = Trusting(certificate, serverName) };
            options.Worker.Name = "named-" + Guid.NewGuid().ToString("N");
            var host = new WorkerHost(options);
            host.RegisterActivity<JsonNode?, int>("a", (_, _, _) => Task.FromResult(1));
            return host;
        }

        await using (var named = Host("other-host.example.test"))
        {
            await named.StartAsync();
            Assert.Equal("1.2", named.ProtocolVersion);
            await named.StopAsync();
        }

        await using (var wrong = Host("wrong.example.test"))
        {
            Assert.Equal(SagatErrorCode.Transport, (await Assert.ThrowsAsync<SagatException>(() => wrong.StartAsync())).Code);
        }

        await using (var without = Host(null))
        {
            Assert.Equal(SagatErrorCode.Transport, (await Assert.ThrowsAsync<SagatException>(() => without.StartAsync())).Code);
        }

        Assert.Equal(1, fake.State.Registrations);
    }

    [Fact]
    public void AServerNameIsNotAnExcuseToSkipVerificationInTheSource()
    {
        var text = string.Concat(Directory.GetFiles(Repository.Path("sdks", "dotnet", "src"), "*.cs", SearchOption.AllDirectories)
            .Where(path => !path.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
            .Select(File.ReadAllText));
        Assert.DoesNotContain("ServerCertificateValidationCallback", text, StringComparison.Ordinal);
        Assert.DoesNotContain("RemoteCertificateValidationCallback", text, StringComparison.Ordinal);
        Assert.Equal(typeof(string), typeof(SagatTlsOptions).GetProperty(nameof(SagatTlsOptions.ServerName))!.PropertyType);
    }
}
