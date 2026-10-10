using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization.Metadata;
using SagatOctopusOrchestrator.Sdk.Conformance;

// The conformance runner of the .NET SDK (tests/contract/sdk-conformance/README.md). It uses only the public API of the
// SDK: the builders in `build` mode, the client and the worker host in `run` mode. It fails loudly: a scenario that does not
// finish exits non-zero with the reason on stderr, and the output file is written only on success.
try
{
    if (args.Length < 2)
    {
        throw new ArgumentException("usage: runner build <scenario> | run <scenario> <config> <out> | aspnet <config> <out>");
    }

    if (args[0] == "aspnet")
    {
        // An optional mode (README, "Optional modes"): the acceptance scenario of US01 against the real platform.
        if (args.Length < 3)
        {
            throw new ArgumentException("usage: runner aspnet <config> <out>");
        }

        using var guard = new CancellationTokenSource(TimeSpan.FromSeconds(120));
        var result = await AspNetMode.PlayAsync(JsonNode.Parse(File.ReadAllText(args[1]))!.AsObject(), guard.Token).ConfigureAwait(false);
        File.WriteAllText(args[2], result.ToJsonString(new JsonSerializerOptions { WriteIndented = true, TypeInfoResolver = new DefaultJsonTypeInfoResolver() }));
        return 0;
    }

    var scenario = JsonNode.Parse(File.ReadAllText(args[1]))!.AsObject();
    switch (args[0])
    {
        case "build":
            Console.Out.Write(BuildMode.Build(scenario).ToJsonString());
            await Console.Out.FlushAsync().ConfigureAwait(false);
            return 0;
        case "run" when args.Length >= 4:
            {
                var config = JsonNode.Parse(File.ReadAllText(args[2]))!.AsObject();

                // The harness gives a scenario five minutes; a runner that is stuck says so before it is killed.
                using var watchdog = new CancellationTokenSource(TimeSpan.FromSeconds(270));
                await using var run = new RunMode(scenario, config);
                var output = await run.PlayAsync(watchdog.Token).ConfigureAwait(false);
                File.WriteAllText(args[3], output.ToJsonString(new JsonSerializerOptions { WriteIndented = true, TypeInfoResolver = new DefaultJsonTypeInfoResolver() }));
                return 0;
            }

        default:
            throw new ArgumentException($"unknown mode {args[0]}");
    }
}
catch (Exception error)
{
    await Console.Error.WriteLineAsync(error.ToString()).ConfigureAwait(false);
    return 1;
}
