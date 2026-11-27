//! `Sha256CommandDigester`, the implementation of the domain's `CommandDigester` port (design 044, Technical Plan
//! 6.2): SHA-256 with the crate's `sha2`, and `d(k)` as RFC 8785 bytes.

use sagat_domain::execution::code::CommandDigester;
use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::canonical::rfc8785;

/// SHA-256 and the RFC 8785 `d(k)` of the command chain.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sha256CommandDigester;

impl CommandDigester for Sha256CommandDigester {
    fn sha256(&self, bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }

    fn command_digest_input(&self, seq: u32, kind: &str, target: &str, body: [u8; 32]) -> Vec<u8> {
        let mut body_hex = String::with_capacity(64);
        for byte in body {
            body_hex.push(char::from(HEX[usize::from(byte >> 4)]));
            body_hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        // Every member is a string but `seq`, a u32 well inside I-JSON, so the total serializer is exact here.
        rfc8785(&json!({"seq": seq, "kind": kind, "target": target, "body": body_hex}))
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";
