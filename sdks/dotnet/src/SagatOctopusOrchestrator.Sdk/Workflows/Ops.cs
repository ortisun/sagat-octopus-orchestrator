using System.Text.Json.Nodes;

namespace SagatOctopusOrchestrator.Sdk.Workflows;

/// <summary>
/// One stage of a collection's operator chain (design 042): an object with exactly one key, the operator's name, whose value
/// is that operator's parameters. Built with <see cref="Ops"/>; the server validates the chain.
/// </summary>
public sealed class OperatorStage
{
    private readonly JsonObject parameters;

    internal OperatorStage(string name, JsonObject parameters)
    {
        Name = name;
        this.parameters = parameters;
    }

    /// <summary>The operator's name as the IR writes it: filter, map, flatMap, skip, take, distinct or window.</summary>
    public string Name { get; }

    /// <summary>The stage as the IR writes it, <c>{name: parameters}</c>; a fresh copy on every call.</summary>
    public JsonObject ToJsonObject() => new() { [Name] = parameters.DeepClone() };

    /// <summary>The canonical text of the stage.</summary>
    public string ToJson() => CanonicalJson.Write(ToJsonObject());

    /// <inheritdoc/>
    public override string ToString() => ToJson();
}

/// <summary>
/// The operator factory of a collection (design 042, FR-042-028). Pass the stages as <see cref="CollectionOptions.Operators"/>.
/// The factory only assembles a stage; the server validates the chain (its length, the ranges of the counts and the variables an
/// expression reads). Stage expressions read <c>element</c>, <c>position</c>, <c>input</c>, <c>steps</c>, <c>run</c> and <c>state</c>.
/// </summary>
public static class Ops
{
    /// <summary>Keeps the elements for which <paramref name="where"/> is true.</summary>
    public static OperatorStage Filter(string where) => new("filter", new JsonObject { ["where"] = where });

    /// <summary>Replaces each element by the value of <paramref name="select"/>.</summary>
    public static OperatorStage Map(string select) => new("map", new JsonObject { ["select"] = select });

    /// <summary><paramref name="select"/> yields a list for each element; the lists are concatenated in order, one level deep.</summary>
    public static OperatorStage FlatMap(string select) => new("flatMap", new JsonObject { ["select"] = select });

    /// <summary>Drops the first <paramref name="count"/> elements.</summary>
    public static OperatorStage Skip(int count) => new("skip", new JsonObject { ["count"] = count });

    /// <summary>Keeps the first <paramref name="count"/> elements.</summary>
    public static OperatorStage Take(int count) => new("take", new JsonObject { ["count"] = count });

    /// <summary>Keeps the first element of each value of <paramref name="by"/>, or of the element itself when it is null.</summary>
    public static OperatorStage Distinct(string? by = null) =>
        new("distinct", by is null ? new JsonObject() : new JsonObject { ["by"] = by });

    /// <summary>Replaces the list by consecutive lists of <paramref name="count"/> elements, the last one possibly shorter.</summary>
    public static OperatorStage Window(int count) => new("window", new JsonObject { ["count"] = count });
}
