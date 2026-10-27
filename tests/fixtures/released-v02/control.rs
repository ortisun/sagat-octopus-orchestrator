//! Private bounded phase transport; never persist or debug its authentication-bearing frames.
#![allow(dead_code, clippy::expect_used, clippy::panic)]
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;

const MAX_FRAME: usize = 16 * 1024;
pub const OUTPUT: &str = r#"{"consumer":"v0.2.0"}"#;
pub const APPLIED_REPORT: &str = r#"{"effect":"applied","output":{"hold":7}}"#;
pub const APPLIED_RESULT: &str = r#"{"hold":7}"#;
pub const QUOTA_RESULT_BYTES: usize = 200_000;
pub const QUOTA_RESERVE_BYTES: usize = 250_000;
pub const QUOTA_HISTORY_BYTES: usize = 210_000;
static QUOTA_REPORT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        r#"{{"effect":"applied","output":"{}"}}"#,
        "q".repeat(QUOTA_RESULT_BYTES - 2)
    )
});

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
    AppliedOutputQuota,
}
impl ProfileMode {
    pub const fn is_applied(self) -> bool {
        match self {
            Self::Forward => false,
            Self::AppliedLookup | Self::AppliedOutputQuota => true,
        }
    }
    pub const fn scenario(self) -> &'static str {
        match self {
            Self::Forward => "inflight-worker",
            Self::AppliedLookup => "applied-lookup",
            Self::AppliedOutputQuota => "applied-output-quota",
        }
    }
    pub const fn old_case(self) -> &'static str {
        match self {
            Self::Forward => "report_old_lease_to_current_server",
            Self::AppliedLookup => "report_old_applied_lookup_to_current_server",
            Self::AppliedOutputQuota => "report_old_applied_lookup_crossing_the_run_output_quota",
        }
    }
    pub const fn lease_node(self) -> &'static str {
        match self {
            Self::Forward => "charge",
            Self::AppliedLookup | Self::AppliedOutputQuota => "charge~r",
        }
    }
    pub fn request_output(self) -> &'static str {
        match self {
            Self::Forward => OUTPUT,
            Self::AppliedLookup => APPLIED_REPORT,
            Self::AppliedOutputQuota => QUOTA_REPORT.as_str(),
        }
    }
    pub const fn retained_output(self) -> Option<&'static str> {
        match self {
            Self::Forward => Some(OUTPUT),
            Self::AppliedLookup => Some(APPLIED_RESULT),
            Self::AppliedOutputQuota => None,
        }
    }
    pub const fn refuses_output(self) -> bool {
        matches!(self, Self::AppliedOutputQuota)
    }
    pub const fn report_status(self) -> &'static str {
        if self.refuses_output() {
            "CANCELLING"
        } else {
            "RUNNING"
        }
    }
    pub const fn terminal_status(self) -> &'static str {
        if self.refuses_output() {
            "COMPENSATED"
        } else {
            "SUCCEEDED"
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

/// Metadata-only identity of the nineteen already retained quota-fixture outputs.
pub async fn quota_sources(pool: &sqlx::PgPool, tenant: uuid::Uuid, run: uuid::Uuid) -> Value {
    let rows:Value=sqlx::query_scalar("SELECT jsonb_agg(jsonb_build_object('node',s.node,'path',s.instance_path_nodes,'indices',s.instance_path_indices,'digest',encode(s.output_digest,'hex'),'bytes',octet_length(p.content)) ORDER BY s.instance_path_nodes,s.instance_path_indices,s.node) FROM steps s LEFT JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest WHERE s.tenant_id=$1 AND s.run_id=$2 AND ((s.instance_path_nodes='{}' AND s.node LIKE 'history%') OR (s.node='historyLoad' AND cardinality(s.instance_path_nodes)=1 AND s.instance_path_nodes[1] LIKE 'history%' AND s.instance_path_indices=ARRAY[0]::int[]) OR (s.node='reserve' AND s.instance_path_nodes=ARRAY['lines']::text[] AND s.instance_path_indices=ARRAY[0]::int[]))")
        .bind(tenant).bind(run).fetch_one(pool).await.expect("bounded prior output metadata");
    let values = rows.as_array().expect("source metadata array");
    assert_eq!(values.len(), 19);
    assert!(values.iter().all(|value| {
        value["digest"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64)
            && value["bytes"].as_i64().is_some_and(|bytes| bytes > 0)
    }));
    rows
}
