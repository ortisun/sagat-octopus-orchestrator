//! Private bounded phase transport; never persist or debug its authentication-bearing frames.
#![allow(clippy::expect_used, clippy::panic)]
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;

const MAX_FRAME: usize = 16 * 1024;
pub const OUTPUT: &str = r#"{"consumer":"v0.2.0"}"#;

pub async fn send(stream: &mut UnixStream, value: &Value) {
    let bytes = serde_json::to_vec(value).expect("phase frame serialization");
    assert!(bytes.len() <= MAX_FRAME, "phase frame exceeds byte limit");
    stream
        .write_u32(u32::try_from(bytes.len()).expect("bounded length"))
        .await
        .expect("phase header write");
    stream.write_all(&bytes).await.expect("phase frame write");
}

pub async fn receive(stream: &mut UnixStream) -> Value {
    let length =
        usize::try_from(stream.read_u32().await.expect("phase header read")).expect("frame length");
    assert!(length <= MAX_FRAME, "phase frame exceeds byte limit");
    let mut bytes = vec![0; length];
    stream
        .read_exact(&mut bytes)
        .await
        .expect("phase frame read");
    serde_json::from_slice(&bytes).expect("phase frame JSON")
}

pub fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
