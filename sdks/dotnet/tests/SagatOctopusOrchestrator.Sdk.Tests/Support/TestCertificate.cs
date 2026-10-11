using System.Net;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;

namespace SagatOctopusOrchestrator.Sdk.Tests.Support;

/// <summary>A self-signed certificate for 127.0.0.1 and localhost: the server presents <see cref="Server"/>, a client that trusts <see cref="Root"/> accepts it.</summary>
internal sealed class TestCertificate
{
    private TestCertificate(X509Certificate2 server, X509Certificate2 root)
    {
        Server = server;
        Root = root;
    }

    /// <summary>With its private key, for Kestrel.</summary>
    internal X509Certificate2 Server { get; }

    /// <summary>The public part, for the trust roots of a client.</summary>
    internal X509Certificate2 Root { get; }

    /// <summary>A server certificate; <paramref name="expired"/> makes it valid only in the past, <paramref name="forOtherHost"/> names a host that is not the loopback.</summary>
    internal static TestCertificate CreateServer(string name = "sagat-test-server", bool expired = false, bool forOtherHost = false) =>
        Create(name, serverAuth: true, expired, forOtherHost);

    internal static X509Certificate2 CreateClient(string name = "sagat-test-client") => Create(name, serverAuth: false, expired: false, forOtherHost: false).Server;

    private static TestCertificate Create(string name, bool serverAuth, bool expired, bool forOtherHost)
    {
        using var key = RSA.Create(2048);
        var request = new CertificateRequest($"CN={name}", key, HashAlgorithmName.SHA256, RSASignaturePadding.Pkcs1);
        var names = new SubjectAlternativeNameBuilder();
        if (forOtherHost)
        {
            names.AddDnsName("other-host.example.test");
        }
        else
        {
            names.AddIpAddress(IPAddress.Loopback);
            names.AddDnsName("localhost");
        }

        request.CertificateExtensions.Add(names.Build());
        request.CertificateExtensions.Add(new X509BasicConstraintsExtension(certificateAuthority: false, hasPathLengthConstraint: false, pathLengthConstraint: 0, critical: true));
        request.CertificateExtensions.Add(new X509KeyUsageExtension(X509KeyUsageFlags.DigitalSignature | X509KeyUsageFlags.KeyEncipherment, critical: true));
        request.CertificateExtensions.Add(new X509EnhancedKeyUsageExtension([new Oid(serverAuth ? "1.3.6.1.5.5.7.3.1" : "1.3.6.1.5.5.7.3.2")], critical: false));
        using var created = expired
            ? request.CreateSelfSigned(DateTimeOffset.UtcNow.AddDays(-3), DateTimeOffset.UtcNow.AddDays(-1))
            : request.CreateSelfSigned(DateTimeOffset.UtcNow.AddDays(-1), DateTimeOffset.UtcNow.AddDays(1));

        // The private key must be persisted for the TLS stack of the platform to use it: round-trip through a PFX.
        var pfx = created.Export(X509ContentType.Pfx, "test");
        var withKey = X509CertificateLoader.LoadPkcs12(pfx, "test", X509KeyStorageFlags.Exportable);
        var publicOnly = X509CertificateLoader.LoadCertificate(created.Export(X509ContentType.Cert));
        return new TestCertificate(withKey, publicOnly);
    }
}
