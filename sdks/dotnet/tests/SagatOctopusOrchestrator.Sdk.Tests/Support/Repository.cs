namespace SagatOctopusOrchestrator.Sdk.Tests.Support;

internal static class Repository
{
    /// <summary>The root of the repository: the first directory above the test binaries that holds the shared contracts.</summary>
    internal static string Root { get; } = Find();

    internal static string Path(params string[] parts) => System.IO.Path.Combine([Root, .. parts]);

    private static string Find()
    {
        for (var directory = new DirectoryInfo(AppContext.BaseDirectory); directory is not null; directory = directory.Parent)
        {
            if (File.Exists(System.IO.Path.Combine(directory.FullName, "sdks", "error-catalog.json"))
                && File.Exists(System.IO.Path.Combine(directory.FullName, "specs", "001-platform-foundation", "contracts", "worker.proto")))
            {
                return directory.FullName;
            }
        }

        throw new InvalidOperationException("The repository (sdks/error-catalog.json) was not found above the test binaries.");
    }
}
