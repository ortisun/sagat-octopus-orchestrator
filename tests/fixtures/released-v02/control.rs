//! Private bounded phase transport; never persist or debug its authentication-bearing frames.
#![allow(dead_code, clippy::expect_used, clippy::panic)]
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;

const MAX_FRAME: usize = 16 * 1024;
pub const OUTPUT: &str = r#"{"consumer":"v0.2.0"}"#;
pub const APPLIED_REPORT: &str = r#"{"effect":"applied","output":{"hold":7}}"#;
pub const APPLIED_RESULT: &str = r#"{"hold":7}"#;

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProfileMode {
    Forward,
    AppliedLookup,
}
impl ProfileMode {
    pub const fn is_applied(self) -> bool {
        match self {
            Self::Forward => false,
            Self::AppliedLookup => true,
        }
    }
    pub const fn scenario(self) -> &'static str {
        match self {
            Self::Forward => "inflight-worker",
            Self::AppliedLookup => "applied-lookup",
        }
    }
    pub const fn old_case(self) -> &'static str {
        match self {
            Self::Forward => "report_old_lease_to_current_server",
            Self::AppliedLookup => "report_old_applied_lookup_to_current_server",
        }
    }
    pub const fn lease_node(self) -> &'static str {
        match self {
            Self::Forward => "charge",
            Self::AppliedLookup => "charge~r",
        }
    }
    pub const fn request_output(self) -> &'static str {
        match self {
            Self::Forward => OUTPUT,
            Self::AppliedLookup => APPLIED_REPORT,
        }
    }
    pub const fn retained_output(self) -> &'static str {
        match self {
            Self::Forward => OUTPUT,
            Self::AppliedLookup => APPLIED_RESULT,
        }
    }
}

pub fn inherited_stream() -> UnixStream {
    use std::os::fd::AsFd as _;
    let inherited = std::io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .expect("inherited anonymous socket");
    let socket = std::os::unix::net::UnixStream::from(inherited);
    socket
        .set_nonblocking(true)
        .expect("nonblocking inherited socket");
    UnixStream::from_std(socket).expect("async inherited socket")
}
