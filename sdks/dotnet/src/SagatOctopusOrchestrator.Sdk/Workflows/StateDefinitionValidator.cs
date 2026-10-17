using System.Globalization;
using System.Text;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;

namespace SagatOctopusOrchestrator.Sdk.Workflows;

internal static partial class StateDefinitionValidator
{
    [GeneratedRegex("^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}\\z", RegexOptions.CultureInvariant)]
    private static partial Regex OperationNamePattern();

    internal static JsonObject Build(StateDefinition definition)
    {
        ArgumentNullException.ThrowIfNull(definition);
        var state = new JsonObject
        {
            ["schema"] = ResultSchema(definition.Schema, "state.schema"),
            ["initial"] = Expression(definition.Initial, "state.initial"),
        };
        var queries = definition.Queries ?? new Dictionary<string, QueryDefinition>();
        if (queries.Count > 32)
        {
            throw SagatException.Invalid("state supports at most 32 queries");
        }

        if (queries.Count > 0)
        {
            var declared = new JsonObject();
            foreach (var (name, query) in queries)
            {
                OperationName(name, "query");
                if (query is null)
                {
                    throw SagatException.Invalid($"query {name} is required");
                }
                Disclosure(query.Disclosure, $"query {name}");
                declared[name] = new JsonObject
                {
                    ["argsSchema"] = ArgsSchema(query.ArgsSchema, $"query {name}"),
                    ["resultSchema"] = ResultSchema(query.ResultSchema, $"query {name}.resultSchema"),
                    ["expression"] = Expression(query.Expression, $"query {name}"),
                    ["disclosure"] = query.Disclosure,
                };
            }

            state["queries"] = declared;
        }

        var updates = definition.Updates ?? new Dictionary<string, UpdateDefinition>();
        if (updates.Count > 32)
        {
            throw SagatException.Invalid("state supports at most 32 updates");
        }

        if (updates.Count > 0)
        {
            var declared = new JsonObject();
            foreach (var (name, update) in updates)
            {
                OperationName(name, "update");
                if (update is null)
                {
                    throw SagatException.Invalid($"update {name} is required");
                }
                Disclosure(update.Disclosure, $"update {name}");
                var entry = new JsonObject
                {
                    ["argsSchema"] = ArgsSchema(update.ArgsSchema, $"update {name}"),
                    ["state"] = Expression(update.State, $"update {name}.state"),
                    ["result"] = Expression(update.Result, $"update {name}.result"),
                    ["resultSchema"] = ResultSchema(update.ResultSchema, $"update {name}.resultSchema"),
                    ["disclosure"] = update.Disclosure,
                };
                if (update.Validate is not null)
                {
                    entry["validate"] = Expression(update.Validate, $"update {name}.validate");
                }

                declared[name] = entry;
            }

            state["updates"] = declared;
        }

        return state;
    }

    private static void OperationName(string name, string kind)
    {
        if (!OperationNamePattern().IsMatch(name ?? string.Empty))
        {
            throw SagatException.Invalid($"{kind} name {CanonicalJson.Write(JsonValue.Create(name))} is invalid");
        }
    }

    private static void Disclosure(string? disclosure, string subject)
    {
        if (disclosure != OperationDisclosure.DeclaredBusinessResultV1)
        {
            throw SagatException.Invalid($"{subject} needs disclosure {OperationDisclosure.DeclaredBusinessResultV1}");
        }
    }

    private static string Expression(string? value, string subject)
    {
        if (string.IsNullOrEmpty(value))
        {
            throw SagatException.Invalid($"{subject} needs an expression");
        }

        if (Encoding.UTF8.GetByteCount(value) > 1024)
        {
            throw SagatException.Invalid($"{subject} expression exceeds 1024 UTF-8 bytes");
        }

        return value;
    }

    private static JsonObject ArgsSchema(JsonObject? value, string subject)
    {
        if (value is null)
        {
            throw SagatException.Invalid($"{subject}.argsSchema must be an object");
        }

        return (JsonObject)value.DeepClone();
    }

    private static JsonObject ResultSchema(JsonObject? value, string subject)
    {
        if (value is null)
        {
            throw SagatException.Invalid($"{subject} must be a closed bounded schema");
        }

        if (value["description"] is JsonNode descriptionNode
            && (descriptionNode is not JsonValue descriptionValue
                || !descriptionValue.TryGetValue<string>(out var description)
                || description.EnumerateRunes().Count() > 1024))
        {
            throw SagatException.Invalid($"{subject}.description must be a string of at most 1024 characters");
        }

        var allowed = new HashSet<string>(["type", "description"], StringComparer.Ordinal);
        var kind = (string?)value["type"];
        switch (kind)
        {
            case "object":
                allowed.UnionWith(["properties", "required", "additionalProperties"]);
                if (value["properties"] is not JsonObject properties)
                {
                    throw SagatException.Invalid($"{subject}.properties must be an object");
                }

                if (value["additionalProperties"] is not JsonValue closedValue
                    || !closedValue.TryGetValue<bool>(out var closed) || closed)
                {
                    throw SagatException.Invalid($"{subject}.additionalProperties must be false");
                }

                foreach (var (name, nested) in properties)
                {
                    ResultSchema(nested as JsonObject, $"{subject}.properties.{name}");
                }

                if (value["required"] is JsonNode requiredNode)
                {
                    if (requiredNode is not JsonArray required || required.Count > 10_000)
                    {
                        throw SagatException.Invalid($"{subject}.required must name unique declared properties");
                    }

                    var seen = new HashSet<string>(StringComparer.Ordinal);
                    foreach (var item in required)
                    {
                        if (item is not JsonValue nameValue || !nameValue.TryGetValue<string>(out var name)
                            || !seen.Add(name) || !properties.ContainsKey(name))
                        {
                            throw SagatException.Invalid($"{subject}.required must name unique declared properties");
                        }
                    }
                }

                break;
            case "array":
                allowed.UnionWith(["items", "minItems", "maxItems"]);
                ResultSchema(value["items"] as JsonObject, $"{subject}.items");
                var maxItems = Integer(value["maxItems"], 0, 10_000, $"{subject}.maxItems");
                if (value["minItems"] is JsonNode minItemsNode
                    && Integer(minItemsNode, 0, 10_000, $"{subject}.minItems") > maxItems)
                {
                    throw SagatException.Invalid($"{subject}.minItems must not exceed maxItems");
                }

                break;
            case "string":
                allowed.UnionWith(["minLength", "maxLength"]);
                var maxLength = Integer(value["maxLength"], 0, 262_144, $"{subject}.maxLength");
                if (value["minLength"] is JsonNode minLengthNode
                    && Integer(minLengthNode, 0, 262_144, $"{subject}.minLength") > maxLength)
                {
                    throw SagatException.Invalid($"{subject}.minLength must not exceed maxLength");
                }

                break;
            case "integer":
            case "number":
                allowed.UnionWith(["minimum", "maximum"]);
                var minimum = Number(value["minimum"], subject, "minimum");
                var maximum = Number(value["maximum"], subject, "maximum");
                if (minimum is double min && maximum is double max && min > max)
                {
                    throw SagatException.Invalid($"{subject}.minimum must not exceed maximum");
                }

                break;
            case "boolean":
            case "null":
                break;
            default:
                throw SagatException.Invalid($"{subject}.type is not a supported closed result type");
        }

        foreach (var key in value.Select(pair => pair.Key))
        {
            if (!allowed.Contains(key))
            {
                throw SagatException.Invalid($"{subject}.{key} is not supported in a closed result schema");
            }
        }

        if (Encoding.UTF8.GetByteCount(CanonicalJson.Write(value)) > 65_536)
        {
            throw SagatException.Invalid($"{subject} exceeds 65536 UTF-8 bytes");
        }

        return (JsonObject)value.DeepClone();
    }

    private static long Integer(JsonNode? node, long minimum, long maximum, string subject)
    {
        if (node is not JsonValue value || !long.TryParse(value.ToJsonString(), NumberStyles.Integer, CultureInfo.InvariantCulture, out var parsed)
            || parsed < minimum || parsed > maximum)
        {
            throw SagatException.Invalid($"{subject} must be between {minimum} and {maximum}");
        }

        return parsed;
    }

    private static double? Number(JsonNode? node, string subject, string field)
    {
        if (node is null)
        {
            return null;
        }

        if (!double.TryParse(node.ToJsonString(), NumberStyles.Float, CultureInfo.InvariantCulture, out var parsed)
            || !double.IsFinite(parsed))
        {
            throw SagatException.Invalid($"{subject}.{field} must be a finite number");
        }

        return parsed;
    }
}
