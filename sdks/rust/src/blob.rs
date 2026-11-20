use crate::BlobDescriptor;
use serde::{Serialize, Serializer, ser::SerializeMap as _};
/// Only the worker upload capability constructs this output marker.
#[derive(Clone)]
pub struct BlobOutput(pub(crate) BlobDescriptor);
impl BlobOutput {
    pub fn descriptor(&self) -> &BlobDescriptor {
        &self.0
    }
}
impl std::fmt::Debug for BlobOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BlobOutput").field(&self.0).finish()
    }
}
impl Serialize for BlobOutput {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(1))?;
        map.serialize_entry("$blob", &self.0)?;
        map.end()
    }
}
#[derive(Clone, Debug)]
pub struct WriteBlobOptions {
    pub content_type: String,
    pub expected_size: Option<u64>,
    pub expected_sha256: Option<String>,
}
impl Default for WriteBlobOptions {
    fn default() -> Self {
        Self {
            content_type: "application/octet-stream".into(),
            expected_size: None,
            expected_sha256: None,
        }
    }
}
