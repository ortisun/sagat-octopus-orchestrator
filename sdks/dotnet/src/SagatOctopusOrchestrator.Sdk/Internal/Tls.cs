using System.Net;
using System.Net.Security;
using System.Security.Cryptography.X509Certificates;

namespace SagatOctopusOrchestrator.Sdk.Internal;

/// <summary>TLS settings for the handlers the SDK creates, and the plain-text warning.</summary>
internal static class Tls
{
    internal static SocketsHttpHandler CreateHandler(SagatTlsOptions? tls, TimeSpan connectTimeout)
    {
        var handler = new SocketsHttpHandler
        {
            AllowAutoRedirect = false,
            PooledConnectionLifetime = TimeSpan.FromMinutes(5),
            ConnectTimeout = connectTimeout,
            EnableMultipleHttp2Connections = true,
        };
        Configure(handler.SslOptions, tls);
        return handler;
    }

    /// <summary>
    /// Trust roots are a chain policy handed to the platform's own validation, which also checks the host name and the validity dates:
    /// the SDK has no certificate callback and no way to turn verification off.
    /// </summary>
    /// <summary>
    /// With a server name the request names that host: the platform's TLS stack takes its host name from there for the server name indication and
    /// for verifying the certificate, so the certificate must carry the name (it is verified, never skipped). The connection still goes to the URL.
    /// </summary>
    internal static HttpMessageHandler WithServerName(HttpMessageHandler inner, SagatTlsOptions? tls) =>
        tls?.ServerName is { } serverName ? new ServerNameHandler(serverName, inner) : inner;

    private sealed class ServerNameHandler(string serverName, HttpMessageHandler inner) : DelegatingHandler(inner)
    {
        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
        {
            request.Headers.Host = serverName;
            return base.SendAsync(request, cancellationToken);
        }
    }

    internal static void Validate(SagatTlsOptions? tls)
    {
        if (tls?.TrustedRoots is { Count: 0 })
        {
            throw SagatException.Invalid("Tls.TrustedRoots is empty: leave it null for the system roots, or give the roots to trust");
        }

        if (tls?.ServerName is { } name && string.IsNullOrWhiteSpace(name))
        {
            throw SagatException.Invalid("Tls.ServerName is empty: leave it null for the host of the URL");
        }
    }

    internal static void Configure(SslClientAuthenticationOptions ssl, SagatTlsOptions? tls)
    {
        Validate(tls);

        if (tls?.ClientCertificate is { } certificate)
        {
            ssl.ClientCertificates = [certificate];
        }

        if (tls?.TrustedRoots is { Count: > 0 } roots)
        {
            var policy = new X509ChainPolicy
            {
                TrustMode = X509ChainTrustMode.CustomRootTrust,
                RevocationMode = X509RevocationMode.NoCheck,
            };
            policy.CustomTrustStore.AddRange(roots);
            ssl.CertificateChainPolicy = policy;
        }
    }

    /// <summary>
    /// A redirect is never followed (it could carry the token to another host). The handler the SDK creates does not follow one; a
    /// handler the caller supplies must not either, and is refused here when it is a known handler that does.
    /// </summary>
    internal static void RejectRedirects(HttpMessageHandler? handler)
    {
        for (var current = handler; current is not null; current = (current as DelegatingHandler)?.InnerHandler)
        {
            if (current is HttpClientHandler { AllowAutoRedirect: true } or SocketsHttpHandler { AllowAutoRedirect: true })
            {
                throw SagatException.Invalid("the HttpMessageHandler follows redirects: set AllowAutoRedirect = false (a redirect is never followed)");
            }
        }
    }

    internal static bool IsPlainTextOutsideLoopback(Uri uri) =>
        uri.Scheme == Uri.UriSchemeHttp && !IsLoopback(uri);

    internal static bool IsLoopback(Uri uri)
    {
        if (uri.IsLoopback)
        {
            return true;
        }

        return uri.Host.Equals("localhost", StringComparison.OrdinalIgnoreCase)
            || uri.Host.EndsWith(".localhost", StringComparison.OrdinalIgnoreCase)
            || (IPAddress.TryParse(uri.Host, out var address) && IPAddress.IsLoopback(address));
    }
}
