using System.Diagnostics;

// The debugger forms of the messages that hold step-log records (design 041, Requirement 041-007 (d)): counts only, never a line's text. The
// SDK never prints, logs or traces these messages; their generated ToString (JSON) is never called by it.
namespace SagatOctopusOrchestrator.Protocol.V1;

[DebuggerDisplay("AppendStepLogsRequest TaskId = {TaskId}, Records = {Records.Count}")]
internal sealed partial class AppendStepLogsRequest
{
}

[DebuggerDisplay("StepLogCarry Records = {Records.Count}, HighestSequence = {HighestSequence}")]
internal sealed partial class StepLogCarry
{
}

[DebuggerDisplay("StepLogRecord Sequence = {Sequence}, Fields = {Fields.Count}")]
internal sealed partial class StepLogRecord
{
}

[DebuggerDisplay("StepLogField")]
internal sealed partial class StepLogField
{
}

[DebuggerDisplay("CompleteStepRequest TaskId = {TaskId}, StepLogs = {StepLogs != null}")]
internal sealed partial class CompleteStepRequest
{
}

[DebuggerDisplay("FailStepRequest TaskId = {TaskId}, StepLogs = {StepLogs != null}")]
internal sealed partial class FailStepRequest
{
}
