using System.Text;

namespace SagatOctopusOrchestrator.Sdk.Workers;

/// <summary>
/// The secrets a step declares, resolved once per attempt (SPEC section 6.7). The values are credentials: the SDK never logs them,
/// never includes them in an error and clears them when the handler returns. Do not log them either.
/// </summary>
public sealed class ActivitySecrets : IDisposable
{
    private readonly Dictionary<string, byte[]> values;

    internal ActivitySecrets(Dictionary<string, byte[]> values) => this.values = values;

    /// <summary>The names of the secrets the step declares.</summary>
    public IReadOnlyCollection<string> Names => values.Keys;

    /// <summary>
    /// A copy of the bytes of a secret (the caller owns it and should clear it; the SDK's own buffer is never handed out). Raises
    /// <see cref="SagatErrorCode.NotFound"/> for a name the step does not declare, or after the handler has returned.
    /// </summary>
    public byte[] Get(string name) => (byte[])Find(name).Clone();

    /// <summary>A secret as UTF-8 text.</summary>
    public string GetText(string name) => Encoding.UTF8.GetString(Find(name));

    /// <summary>Clears the values.</summary>
    public void Dispose()
    {
        foreach (var value in values.Values)
        {
            Array.Clear(value);
        }

        values.Clear();
    }

    private byte[] Find(string name) =>
        values.TryGetValue(name, out var value)
            ? value
            : throw new SagatException(SagatErrorCode.NotFound, $"the step declares no secret named {name}");
}
