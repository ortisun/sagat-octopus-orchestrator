fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../proto/worker.proto");
    let descriptors = protox::compile(["../proto/worker.proto"], ["../proto"])?;
    // A step-log batch carries the text a step's code logged (feature 041, FR-041-007 (d)): its
    // generated `Debug` is skipped and replaced by one that prints counts only.
    let mut config = tonic_prost_build::Config::new();
    config.skip_debug([
        "sagat_octopus_orchestrator.v1.AppendStepLogsRequest",
        "sagat_octopus_orchestrator.v1.StepLogCarry",
        "sagat_octopus_orchestrator.v1.StepLogRecord",
        "sagat_octopus_orchestrator.v1.StepLogField",
        "sagat_octopus_orchestrator.v1.StepLogField.value",
    ]);
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_fds_with_config(descriptors, config)?;
    Ok(())
}
