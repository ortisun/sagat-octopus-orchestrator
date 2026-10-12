//! Compiles the worker protocol contract (`worker.proto`, the single source of truth shared with
//! every SDK) into the server and client stubs. `protox` is a pure-Rust protobuf compiler, so the
//! build needs neither `protoc` nor network access beyond the crates themselves.

use std::error::Error;

const CONTRACTS: &str = "../../specs/001-platform-foundation/contracts";

fn main() -> Result<(), Box<dyn Error>> {
    let proto = format!("{CONTRACTS}/worker.proto");
    println!("cargo:rerun-if-changed={proto}");
    println!("cargo:rerun-if-changed=build.rs");
    let descriptors = protox::compile([proto.as_str()], [CONTRACTS])?;
    // A resolved secret is a credential: the generated `Debug` would print its bytes, so it is
    // skipped here and replaced by a redacting one in `lib.rs`.
    let mut config = tonic_prost_build::Config::new();
    config.skip_debug(["sagat_octopus_orchestrator.v1.ResolvedSecret"]);
    // The chunks of a blob are handed on without being copied.
    config.bytes([
        ".sagat_octopus_orchestrator.v1.WriteBlobRequest.chunk",
        ".sagat_octopus_orchestrator.v1.ReadBlobResponse.chunk",
    ]);
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_fds_with_config(descriptors, config)?;
    Ok(())
}
