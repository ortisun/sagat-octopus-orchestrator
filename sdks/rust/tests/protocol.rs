#[test]
fn packaged_protocol_matches_the_canonical_wire_contract() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let packaged = std::fs::read(root.join("proto/worker.proto"));
    let canonical =
        std::fs::read(root.join("../../specs/001-platform-foundation/contracts/worker.proto"));
    assert!(
        packaged.is_ok(),
        "the standalone package must embed its protocol"
    );
    assert_eq!(packaged.ok(), canonical.ok());
}
