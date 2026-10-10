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
            // The name verified, and the port of the URL (the connection still goes where the URL says).
            var uri = request.RequestUri;
            request.Headers.Host = uri is null || uri.IsDefaultPort ? serverName : $"{serverName}:{uri.Port}";
            return base.SendAsync(request, cancellationToken);
        }
    }

    internal static void Validate(SagatTlsOptions? tls)
    {
        if (tls?.TrustedRoots is { Count: 0 })
        {
            throw SagatException.Invalid("Tls.TrustedRoots is empty: leave it null for the system roots, or give the roots to trust");
        }

        if (tls?.ServerName is { } name)
        {
            // A host name or an address and nothing else (no port, no user-info, no path, no space, no line break): what the request will name.
            if (string.IsNullOrWhiteSpace(name))
            {
                throw SagatException.Invalid("Tls.ServerName is empty: leave it null for the host of the URL");
            }

            if (Uri.CheckHostName(name) == UriHostNameType.Unknown)
            {
                throw SagatException.Invalid("Tls.ServerName is not a host name or an address (no port, no user-info, no path)");
            }
        }
    }

    /// <summary>A server name is verified against the certificate of a TLS connection: with a plain-text URL there is nothing to verify, and it would only rename the host of the request.</summary>
    internal static void RequireTlsForServerName(SagatTlsOptions? tls, Uri url)
    {
        if (tls?.ServerName is not null && url.Scheme != Uri.UriSchemeHttps)
        {
            throw SagatException.Invalid("Tls.ServerName needs an https URL: there is no certificate to verify over http");
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
