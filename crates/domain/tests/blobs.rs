//! Blobs: large payloads kept outside the database (FR-002-008, ADR-0023).
#![allow(clippy::expect_used)]

use sagat_domain::blobs::{BlobDescriptor, BlobError, BlobId, BlobState, ContentType};
use sagat_domain::workflow::Digest;

fn digest() -> Digest {
    Digest::new([7; 32])
}

#[test]
fn a_content_type_is_a_normalized_media_type_without_parameters() {
    for (given, stored) in [
        ("application/json", "application/json"),
        ("Application/JSON", "application/json"),
        ("application/json; charset=utf-8", "application/json"),
        ("  text/plain ", "text/plain"),
        ("application/vnd.api+json", "application/vnd.api+json"),
        ("image/svg+xml", "image/svg+xml"),
    ] {
        let parsed = ContentType::parse(given).unwrap_or_else(|e| panic!("{given:?}: {e}"));
        assert_eq!(parsed.as_str(), stored, "{given:?}");
    }
}

#[test]
fn anything_that_is_not_a_media_type_is_refused() {
    let too_long = format!("application/{}", "x".repeat(130));
    for bad in [
        "",
        "json",
        "application/",
        "/json",
        "application/json/extra",
        "application / json",
        "application/jsön",
        "application/json\r\nX-Injected: 1",
        "text/*",
        too_long.as_str(),
    ] {
        assert_eq!(
            ContentType::parse(bad),
            Err(BlobError::InvalidContentType),
            "{bad:?}"
        );
    }
}

#[test]
fn the_default_content_type_is_opaque_bytes() {
    assert_eq!(
        ContentType::octet_stream().as_str(),
        "application/octet-stream"
    );
}

#[test]
fn a_descriptor_keeps_what_identifies_the_content() {
    let id = BlobId::generate();
    let descriptor = BlobDescriptor::new(
        id,
        1_048_576,
        digest(),
        ContentType::parse("application/json").expect("valid"),
    );

    assert_eq!(descriptor.id(), id);
    assert_eq!(descriptor.size(), 1_048_576);
    assert_eq!(descriptor.digest(), &digest());
    assert_eq!(descriptor.content_type().as_str(), "application/json");
}

#[test]
fn a_blob_id_round_trips_through_its_text_form() {
    let id = BlobId::generate();
    let text = id.to_string();

    assert_eq!(text.parse::<BlobId>().expect("parses"), id);
    assert!("not-a-uuid".parse::<BlobId>().is_err());
}

#[test]
fn a_blob_goes_forward_only_and_only_an_uploaded_one_can_be_committed() {
    assert_eq!(BlobState::Uploading.uploaded(), Ok(BlobState::Uploaded));
    assert_eq!(BlobState::Uploaded.committed(), Ok(BlobState::Committed));

    // Not before the bytes are safely stored, and never twice.
    assert_eq!(BlobState::Uploading.committed(), Err(BlobError::NotUsable));
    assert_eq!(BlobState::Uploaded.uploaded(), Err(BlobError::NotUsable));
    assert_eq!(BlobState::Committed.committed(), Err(BlobError::NotUsable));
    assert_eq!(BlobState::Committed.uploaded(), Err(BlobError::NotUsable));
    assert_eq!(BlobState::Deleting.committed(), Err(BlobError::NotUsable));
    assert_eq!(BlobState::Deleting.uploaded(), Err(BlobError::NotUsable));
}

#[test]
fn the_stored_form_of_a_state_is_stable() {
    for state in [
        BlobState::Uploading,
        BlobState::Uploaded,
        BlobState::Committed,
        BlobState::Deleting,
    ] {
        assert_eq!(BlobState::parse_stored(state.as_str()), Some(state));
    }
    assert_eq!(BlobState::Uploading.as_str(), "UPLOADING");
    assert_eq!(BlobState::Uploaded.as_str(), "UPLOADED");
    assert_eq!(BlobState::Committed.as_str(), "COMMITTED");
    assert_eq!(BlobState::Deleting.as_str(), "DELETING");
    assert_eq!(BlobState::parse_stored("uploading"), None);
}

#[test]
fn only_a_committed_blob_is_referenced_by_the_system() {
    assert!(!BlobState::Uploading.is_referenced());
    assert!(!BlobState::Uploaded.is_referenced());
    assert!(!BlobState::Deleting.is_referenced());
    assert!(BlobState::Committed.is_referenced());
}
