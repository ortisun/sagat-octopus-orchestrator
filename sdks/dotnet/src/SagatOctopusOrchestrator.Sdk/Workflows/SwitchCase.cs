namespace SagatOctopusOrchestrator.Sdk.Workflows;

/// <summary>One ordered branch of a switch decision.</summary>
public sealed record SwitchCase(string Id, string When, string Target);
