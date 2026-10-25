using System.Reflection;

namespace SagatOctopusOrchestrator.Sdk;

/// <summary>Constants that identify this SDK on the wire and in telemetry.</summary>
public static class SdkInfo
{
    /// <summary>The version of the SDK, sent in <c>User-Agent</c> (<c>sagat-sdk-csharp/&lt;version&gt;</c>).</summary>
    public const string Version = "0.2.0";

    /// <summary>The newest minor version of worker protocol 1 this SDK speaks (SPEC section 6.1).</summary>
    public const string ProtocolVersion = "1.5";

    /// <summary>The <c>languages</c> capability a worker host announces.</summary>
    public const string Language = "csharp";

    /// <summary>The name of the <see cref="System.Diagnostics.ActivitySource"/> this SDK emits spans from.</summary>
    public const string ActivitySourceName = "SagatOctopusOrchestrator.Sdk";

    /// <summary>The name of the <see cref="System.Diagnostics.Metrics.Meter"/> this SDK emits metrics from.</summary>
    public const string MeterName = "SagatOctopusOrchestrator.Sdk";

    internal static readonly Assembly Assembly = typeof(SdkInfo).Assembly;

    internal static string UserAgent(string? suffix) =>
        string.IsNullOrWhiteSpace(suffix) ? $"sagat-sdk-csharp/{Version}" : $"sagat-sdk-csharp/{Version} {suffix}";
}
