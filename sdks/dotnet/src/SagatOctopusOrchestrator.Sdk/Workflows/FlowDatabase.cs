namespace SagatOctopusOrchestrator.Sdk.Workflows;

public abstract partial class FlowBase<TSelf>
    where TSelf : FlowBase<TSelf>
{
    /// <summary>Uses a connection managed by Sagat; ordinary SDK workers never receive its credentials.</summary>
    public TSelf Database(string id, DatabaseOptions options) => Push((options ?? throw DatabaseOptions.Invalid()).ToNode(id));
}
