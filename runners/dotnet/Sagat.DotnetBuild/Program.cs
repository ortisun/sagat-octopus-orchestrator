using System.Reflection;
using System.Security.Cryptography;
using System.Text.Json;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Sagat.ScriptContract;

internal static class Program
{
    private static readonly JsonSerializerOptions Json = new(JsonSerializerDefaults.Web);

    public static async Task<int> Main()
    {
        try
        {
            var line = await Console.In.ReadLineAsync();
            var request = JsonSerializer.Deserialize<BuildEnvelope>(line ?? string.Empty, Json)
                ?? throw new BuildRejected("the build request is not JSON");
            if (!string.Equals(request.Op, "build", StringComparison.Ordinal))
            {
                throw new BuildRejected("the build request has another operation");
            }

            var catalogPath = Environment.GetEnvironmentVariable("SAGAT_DOTNET_CATALOG")
                ?? "/opt/sagat/catalog/catalog.json";
            var references = Catalog.Load(catalogPath).Approve(request.Lockfile, request.Packages);
            var assembly = Compile(request.Source, references.Select(reference => reference.Path));
            var files = new SortedDictionary<string, string>(StringComparer.Ordinal)
            {
                ["script.dll"] = Convert.ToBase64String(assembly),
            };
            foreach (var reference in references)
            {
                files.Add(reference.File, Convert.ToBase64String(await File.ReadAllBytesAsync(reference.Path)));
            }
            await Say(new { op = "built", entry = "script.dll", files });
            return 0;
        }
        catch (BuildRejected error)
        {
            await Say(new { op = "rejected", message = error.Message });
            return 0;
        }
        catch (Exception error)
        {
            await Say(new { op = "rejected", message = $"the C# builder failed: {error.GetType().Name}" });
            return 0;
        }
    }

    private static byte[] Compile(string source, IEnumerable<string> packageAssemblies)
    {
        if (string.IsNullOrWhiteSpace(source))
        {
            throw new BuildRejected("the C# source is empty");
        }
        var syntax = CSharpSyntaxTree.ParseText(
            source,
            new CSharpParseOptions(LanguageVersion.CSharp14),
            path: "script.cs");
        var defaults = CSharpSyntaxTree.ParseText(
            "global using System; global using System.Threading; global using System.Threading.Tasks;",
            new CSharpParseOptions(LanguageVersion.CSharp14),
            path: "platform.cs");
        var platform = ((string?)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES"))
            ?.Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
            ?? throw new BuildRejected("the .NET reference set is unavailable");
        var paths = platform
            .Append(typeof(IInlineScript).Assembly.Location)
            .Concat(packageAssemblies)
            .Distinct(StringComparer.Ordinal);
        var references = paths.Select(path => MetadataReference.CreateFromFile(path));
        var compilation = CSharpCompilation.Create(
            "Sagat.InlineScript",
            [defaults, syntax],
            references,
            new CSharpCompilationOptions(
                OutputKind.DynamicallyLinkedLibrary,
                optimizationLevel: OptimizationLevel.Release,
                allowUnsafe: false,
                deterministic: true,
                concurrentBuild: false));

        var contract = compilation.GetTypeByMetadataName(typeof(IInlineScript).FullName!)
            ?? throw new BuildRejected("the C# script contract is unavailable");
        var scripts = Types(compilation.Assembly.GlobalNamespace)
            .Where(type => !type.IsAbstract && type.AllInterfaces.Contains(contract, SymbolEqualityComparer.Default))
            .ToArray();
        if (scripts.Length != 1 || !scripts[0].InstanceConstructors.Any(ctor =>
                ctor.DeclaredAccessibility == Accessibility.Public && ctor.Parameters.Length == 0))
        {
            throw new BuildRejected("the source must define one public IInlineScript with a public parameterless constructor");
        }

        using var output = new MemoryStream();
        var result = compilation.Emit(output);
        if (!result.Success)
        {
            var messages = result.Diagnostics
                .Where(diagnostic => diagnostic.Severity == DiagnosticSeverity.Error)
                .Take(6)
                .Select(diagnostic => diagnostic.GetMessage())
                .ToArray();
            throw new BuildRejected(string.Join("\n", messages));
        }
        return output.ToArray();
    }

    private static IEnumerable<INamedTypeSymbol> Types(INamespaceSymbol scope)
    {
        foreach (var type in scope.GetTypeMembers())
        {
            yield return type;
            foreach (var nested in Nested(type))
            {
                yield return nested;
            }
        }
        foreach (var child in scope.GetNamespaceMembers())
        {
            foreach (var type in Types(child))
            {
                yield return type;
            }
        }
    }

    private static IEnumerable<INamedTypeSymbol> Nested(INamedTypeSymbol parent)
    {
        foreach (var child in parent.GetTypeMembers())
        {
            yield return child;
            foreach (var nested in Nested(child))
            {
                yield return nested;
            }
        }
    }

    private static Task Say(object message) => Console.Out.WriteLineAsync(JsonSerializer.Serialize(message, Json));
}

internal sealed record BuildEnvelope(string Op, string Source, string? Lockfile, PackagePin[] Packages);
internal sealed record PackagePin(string Id, string Version, string ContentHash);
internal sealed record ApprovedAssembly(string File, string Path);

internal sealed class BuildRejected(string message) : Exception(message);

internal sealed class Catalog
{
    private const string Newtonsoft = "Newtonsoft.Json";
    private const string Version = "13.0.4";
    private const string ContentHash = "pdgNNMai3zv51W5aq268sujXUyx7SNdE2bj1wZcWjAQrKMFZV260lbqYop1d2GM67JI1huLRwxo9ZqnfF/lC6A==";
    private const string ArchiveSha512 = "bR+v+E/yJ6g7GV2uXw2OrUSjYYfjLkOLC8JD4kCS23msLapnKtdJPBJA75fwHH++ErIffeIqzYITLxAur4KAXA==";
    private const string AssemblySha256 = "a28c251dfe36d881e9e2462e171441b8b0ec156fe3f452602c9149b1b9efe05b";

    private readonly string _directory;
    private readonly CatalogDocument _document;

    private Catalog(string path, CatalogDocument document)
    {
        _directory = Path.GetDirectoryName(Path.GetFullPath(path))!;
        _document = document;
    }

    public static Catalog Load(string path)
    {
        var document = JsonSerializer.Deserialize<CatalogDocument>(File.ReadAllText(path), new JsonSerializerOptions(JsonSerializerDefaults.Web))
            ?? throw new BuildRejected("the immutable package catalog is unreadable");
        if (document.Version != 1 || document.Packages.Length != 1)
        {
            throw new BuildRejected("the immutable package catalog has another shape");
        }
        return new Catalog(path, document);
    }

    public IReadOnlyList<ApprovedAssembly> Approve(string? lockfile, PackagePin[] supplied)
    {
        if (string.IsNullOrWhiteSpace(lockfile))
        {
            if (supplied.Length != 0)
            {
                throw new BuildRejected("approved packages were supplied without a lockfile");
            }
            return [];
        }
        using var parsed = JsonDocument.Parse(lockfile);
        EnsureUnique(parsed.RootElement);
        var root = parsed.RootElement;
        ExactProperties(root, "version", "dependencies");
        if (root.GetProperty("version").GetInt32() != 1)
        {
            throw new BuildRejected("the C# lockfile must be version 1");
        }
        var groups = root.GetProperty("dependencies");
        ExactProperties(groups, "net10.0");
        var dependencies = groups.GetProperty("net10.0");
        if (!dependencies.EnumerateObject().Any())
        {
            if (supplied.Length != 0)
            {
                throw new BuildRejected("approved packages do not match the lockfile");
            }
            return [];
        }
        ExactProperties(dependencies, Newtonsoft);
        var package = dependencies.GetProperty(Newtonsoft);
        ExactProperties(package, "type", "requested", "resolved", "contentHash");
        if (package.GetProperty("type").GetString() != "Direct"
            || package.GetProperty("requested").GetString() != "[13.0.4, 13.0.4]"
            || package.GetProperty("resolved").GetString() != Version
            || package.GetProperty("contentHash").GetString() != ContentHash
            || supplied is not [{ Id: Newtonsoft, Version: Version, ContentHash: ContentHash }])
        {
            throw new BuildRejected("the lockfile does not match the immutable package catalog");
        }
        var catalogPackage = _document.Packages.Single();
        if (catalogPackage.Id != Newtonsoft || catalogPackage.Version != Version
            || catalogPackage.ContentHash != ContentHash || catalogPackage.ArchiveSha512 != ArchiveSha512
            || catalogPackage.Dependencies.Length != 0
            || catalogPackage.Assemblies is not [{ File: "Newtonsoft.Json.dll", Sha256: AssemblySha256 }])
        {
            throw new BuildRejected("the immutable package catalog does not match its policy");
        }
        var approved = new List<ApprovedAssembly>();
        foreach (var assembly in catalogPackage.Assemblies)
        {
            if (Path.GetFileName(assembly.File) != assembly.File || !assembly.File.EndsWith(".dll", StringComparison.OrdinalIgnoreCase))
            {
                throw new BuildRejected("the immutable package catalog names an invalid assembly");
            }
            var path = Path.Combine(_directory, assembly.File);
            var actual = Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(path))).ToLowerInvariant();
            if (!CryptographicOperations.FixedTimeEquals(
                    Convert.FromHexString(actual), Convert.FromHexString(assembly.Sha256)))
            {
                throw new BuildRejected("an assembly in the immutable package catalog has changed");
            }
            approved.Add(new ApprovedAssembly(assembly.File, path));
        }
        return approved;
    }

    private static void ExactProperties(JsonElement element, params string[] names)
    {
        if (element.ValueKind != JsonValueKind.Object
            || element.EnumerateObject().Select(property => property.Name).Order(StringComparer.Ordinal)
                .SequenceEqual(names.Order(StringComparer.Ordinal), StringComparer.Ordinal) is false)
        {
            throw new BuildRejected("the lockfile has an unexpected shape");
        }
    }

    private static void EnsureUnique(JsonElement element)
    {
        if (element.ValueKind == JsonValueKind.Object)
        {
            var names = new HashSet<string>(StringComparer.Ordinal);
            foreach (var property in element.EnumerateObject())
            {
                if (!names.Add(property.Name))
                {
                    throw new BuildRejected("the lockfile repeats a JSON member");
                }
                EnsureUnique(property.Value);
            }
        }
        else if (element.ValueKind == JsonValueKind.Array)
        {
            foreach (var child in element.EnumerateArray())
            {
                EnsureUnique(child);
            }
        }
    }
}

internal sealed record CatalogDocument(int Version, CatalogPackage[] Packages);
internal sealed record CatalogPackage(
    string Id,
    string Version,
    string ContentHash,
    string ArchiveSha512,
    string[] Dependencies,
    CatalogAssembly[] Assemblies);
internal sealed record CatalogAssembly(string File, string Sha256);
