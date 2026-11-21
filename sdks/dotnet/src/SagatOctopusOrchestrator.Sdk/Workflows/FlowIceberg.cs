namespace SagatOctopusOrchestrator.Sdk.Workflows;

public abstract partial class FlowBase<TSelf>
    where TSelf : FlowBase<TSelf>
{
    /// <summary>Appends records to an Iceberg table through the isolated lake runner; no catalog or storage credential appears here.</summary>
    public TSelf IcebergWrite(string id, IcebergWriteOptions options) => Push((options ?? throw IcebergWriteOptions.Invalid()).ToNode(id));
}
