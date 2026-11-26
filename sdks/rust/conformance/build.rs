fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../proto/worker.proto");
    let descriptors = protox::compile(["../proto/worker.proto"], ["../proto"])?;
    // A step-log batch carries the text a step's code logged (feature 041, FR-041-007 (d)): its
    // generated `Debug` is skipped and replaced by one that prints counts only.
    let mut config = tonic_prost_build::Config::new();
    config.skip_debug([
        "sagat_octopus_orchestrator.v1.AppendStepLogsRequest",
        "sagat_octopus_orchestrator.v1.StepLogRecord",
        "sagat_octopus_orchestrator.v1.StepLogField",
        "sagat_octopus_orchestrator.v1.StepLogField.value",
        // every message with a field ending in `_token` (delivery acknowledgement, Task 006-043): counts and fences only
        "sagat_octopus_orchestrator.v1.RegisterWorkerResponse",
        "sagat_octopus_orchestrator.v1.PollTasksRequest",
        "sagat_octopus_orchestrator.v1.TaskLease",
        "sagat_octopus_orchestrator.v1.HeartbeatRequest",
        "sagat_octopus_orchestrator.v1.CompleteStepRequest",
        "sagat_octopus_orchestrator.v1.FailStepRequest",
        "sagat_octopus_orchestrator.v1.ResolveSecretsRequest",
        "sagat_octopus_orchestrator.v1.WriteBlobHeader",
        "sagat_octopus_orchestrator.v1.ReadBlobRequest",
        "sagat_octopus_orchestrator.v1.TaskAcknowledgement",
    ]);
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_fds_with_config(descriptors, config)?;
    Ok(())
}
