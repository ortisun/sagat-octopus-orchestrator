//! The settings of the lake profile of the runner (feature 035, T035-035, D035-123, slice A).
//!
//! A `lake` section of the runner's file configuration. **Absent means the lake profile is off**: the lake activities are
//! not offered and no directory, socket or relay is touched. Present, it is read once and validated whole, and a value
//! outside its bounds is an invalid configuration that stops the runner at start, not a default that is used.
//!
//! What a deployment must say, with no default because a default would be a decision about its security:
//!
//! - where the work volume is mounted (`workRoot`) and where the sandbox runner's socket is (`sandboxSocket`), and the user
//!   that process runs as (`sandboxUid`): the kernel says who is at the other end of the socket, and a socket file that a
//!   process of another user put there is not believed (ADR-0079);
//! - how many containers' users there are (`slots`): the lake runner picks the slot of an attempt, and its user is
//!   `40000 + slot`;
//! - which addresses the relay may connect to (`relayAllowCidrs`). **Empty reaches nothing**, and outside development empty
//!   is refused as a configuration that could not work. The relay itself refuses loopback, link-local, metadata
//!   and other special-use addresses, **and no private network**: not the private ranges, not the pod and service networks, not the platform's own state
//!   (PostgreSQL, the broker, the secret store). A tenant declares the catalog and the storage endpoints of its connection, so
//!   an endpoint that resolves into the platform would otherwise be reached by a compromised host. Those ranges are the
//!   operator's to list in `relayDenyCidrs`, **required and non-empty outside development**, as `denyCidrs` is for the SQL
//!   runner; a deny range wins over an allow range. `0.0.0.0/0` and `::/0` in `relayAllowCidrs` are a valid, explicit answer
//!   ("the internet, less what is denied") and not a default.
//!
//! The numbers (bytes and records of one attempt, the host's ceilings, the time to wait) have defaults that match the
//! contract's fixtures and are bounded on both sides.

use std::path::PathBuf;
use std::time::Duration;

use ipnet::IpNet;
use sagat_infra_lake::sandbox_api::{FIRST_UID, MAX_SLOTS};
use serde::Deserialize;

/// The host's own bound on commit retries (`Limits::new` refuses more).
pub const MAX_COMMIT_RETRIES: u32 = 16;
/// The most addresses ranges one runner is told about.
const MAX_CIDRS: usize = 256;
/// The relay's socket is `<workRoot>/<36-character attempt id>/relay.sock`, and a Unix socket path is at most 104 bytes
/// (macOS; 108 on Linux) with its terminator. Longer and every attempt would fail to bind, which is retryable and so would
/// retry for ever: the section is refused instead.
const MAX_WORK_ROOT_BYTES: usize = 103 - (1 + 36 + 1 + "relay.sock".len());
/// The user the kernel reports for an unmapped one: a process in a user namespace that is not this one looks like it.
const OVERFLOW_UID: u32 = 65_534;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Why the section is not valid. It names a part of the section, never a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error("`{0}` of the lake section is missing, out of its bounds, or not what it must be")]
    Setting(&'static str),
}

fn default_max_bytes() -> u64 {
    GIB
}
fn default_max_records() -> u64 {
    10_000_000
}
fn default_commit_retries() -> u32 {
    4
}
fn default_decoded_bytes() -> u64 {
    16 * GIB
}
fn default_metadata_bytes() -> u64 {
    64 * MIB
}
fn default_host_start_secs() -> u64 {
    30
}
fn default_kill_limit_secs() -> u64 {
    20
}

/// The section as the file has it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LakeSettings {
    pub work_root: PathBuf,
    pub sandbox_socket: PathBuf,
    pub sandbox_uid: u32,
    pub slots: u32,
    pub relay_allow_cidrs: Vec<String>,
    pub relay_deny_cidrs: Vec<String>,
    #[serde(default = "default_max_bytes")]
    pub max_bytes: u64,
    #[serde(default = "default_max_records")]
    pub max_records: u64,
    #[serde(default = "default_commit_retries")]
    pub max_commit_retries: u32,
    #[serde(default = "default_decoded_bytes")]
    pub max_decoded_bytes: u64,
    #[serde(default = "default_metadata_bytes")]
    pub max_metadata_bytes: u64,
    #[serde(default = "default_host_start_secs")]
    pub host_start_secs: u64,
    #[serde(default = "default_kill_limit_secs")]
    pub kill_limit_secs: u64,
    /// An attempt whose declared input is above this many bytes asks for the `large` class, when the node has one.
    #[serde(default)]
    pub large_above_bytes: Option<u64>,
}

/// The section, validated and parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub work_root: PathBuf,
    pub sandbox_socket: PathBuf,
    pub sandbox_uid: u32,
    pub slots: u32,
    pub relay_allow: Vec<IpNet>,
    pub relay_deny: Vec<IpNet>,
    pub max_bytes: u64,
    pub max_records: u64,
    pub max_commit_retries: u32,
    pub max_decoded_bytes: u64,
    pub max_metadata_bytes: u64,
    pub host_start: Duration,
    pub kill_limit: Duration,
    pub large_above_bytes: Option<u64>,
}

fn absolute(path: &std::path::Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|part| !matches!(part, std::path::Component::ParentDir))
}

impl LakeSettings {
    /// # Errors
    /// [`SettingsError`] naming the first setting that is not valid. `development` allows an empty `relayAllowCidrs` (the
    /// fixtures of the tests run on the loopback interface, which `development` opens in the relay).
    pub fn validated(&self, development: bool) -> Result<Settings, SettingsError> {
        let bad = SettingsError::Setting;
        if !absolute(&self.work_root) || self.work_root.as_os_str().len() > MAX_WORK_ROOT_BYTES {
            return Err(bad("workRoot"));
        }
        if !absolute(&self.sandbox_socket) {
            return Err(bad("sandboxSocket"));
        }
        // the sandbox runner is a process of its own: not root, and not one of the users of the containers
        if self.sandbox_uid == 0
            || (FIRST_UID..FIRST_UID + MAX_SLOTS).contains(&self.sandbox_uid)
            || self.sandbox_uid == OVERFLOW_UID
            || self.sandbox_uid == u32::MAX
        {
            return Err(bad("sandboxUid"));
        }
        if !(1..=MAX_SLOTS).contains(&self.slots) {
            return Err(bad("slots"));
        }
        if self.relay_allow_cidrs.len() > MAX_CIDRS
            || (self.relay_allow_cidrs.is_empty() && !development)
        {
            return Err(bad("relayAllowCidrs"));
        }
        let relay_allow = self
            .relay_allow_cidrs
            .iter()
            .map(|text| text.parse::<IpNet>().map_err(|_| bad("relayAllowCidrs")))
            .collect::<Result<Vec<_>, _>>()?;
        if self.relay_deny_cidrs.len() > MAX_CIDRS
            || (self.relay_deny_cidrs.is_empty() && !development)
        {
            return Err(bad("relayDenyCidrs"));
        }
        let relay_deny = self
            .relay_deny_cidrs
            .iter()
            .map(|text| text.parse::<IpNet>().map_err(|_| bad("relayDenyCidrs")))
            .collect::<Result<Vec<_>, _>>()?;
        let ceiling = u64::try_from(i64::MAX).map_err(|_| bad("maxDecodedBytes"))?;
        if !(MIB..=64 * GIB).contains(&self.max_bytes) {
            return Err(bad("maxBytes"));
        }
        if !(1..=100_000_000).contains(&self.max_records) {
            return Err(bad("maxRecords"));
        }
        if self.max_commit_retries > MAX_COMMIT_RETRIES {
            return Err(bad("maxCommitRetries"));
        }
        if !(MIB..=ceiling).contains(&self.max_decoded_bytes) {
            return Err(bad("maxDecodedBytes"));
        }
        if !(MIB..=ceiling).contains(&self.max_metadata_bytes) {
            return Err(bad("maxMetadataBytes"));
        }
        if !(1..=600).contains(&self.host_start_secs) {
            return Err(bad("hostStartSecs"));
        }
        if !(1..=300).contains(&self.kill_limit_secs) {
            return Err(bad("killLimitSecs"));
        }
        if self
            .large_above_bytes
            .is_some_and(|above| above == 0 || above > self.max_bytes)
        {
            return Err(bad("largeAboveBytes"));
        }
        Ok(Settings {
            work_root: self.work_root.clone(),
            sandbox_socket: self.sandbox_socket.clone(),
            sandbox_uid: self.sandbox_uid,
            slots: self.slots,
            relay_allow,
            relay_deny,
            max_bytes: self.max_bytes,
            max_records: self.max_records,
            max_commit_retries: self.max_commit_retries,
            max_decoded_bytes: self.max_decoded_bytes,
            max_metadata_bytes: self.max_metadata_bytes,
            host_start: Duration::from_secs(self.host_start_secs),
            kill_limit: Duration::from_secs(self.kill_limit_secs),
            large_above_bytes: self.large_above_bytes,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn section() -> Value {
        json!({
            "workRoot": "/work",
            "sandboxSocket": "/run/sagat/lake-sandbox.sock",
            "sandboxUid": 10002,
            "slots": 8,
            "relayAllowCidrs": ["0.0.0.0/0", "::/0"],
            "relayDenyCidrs": ["10.0.0.0/8", "fd00::/8"]
        })
    }

    fn read(document: &Value) -> Result<LakeSettings, serde_json::Error> {
        serde_json::from_value(document.clone())
    }

    fn with(edit: impl FnOnce(&mut Value)) -> Value {
        let mut document = section();
        edit(&mut document);
        document
    }

    fn refused(document: &Value, development: bool) -> Option<&'static str> {
        match read(document).map(|section| section.validated(development)) {
            Ok(Ok(_)) => None,
            Ok(Err(SettingsError::Setting(name))) => Some(name),
            Err(_) => Some("document"),
        }
    }

    #[test]
    fn a_section_with_what_a_deployment_must_say_gets_the_defaults_of_the_contract() {
        let settings = read(&section()).unwrap().validated(false).unwrap();
        assert_eq!(settings.max_bytes, GIB);
        assert_eq!(settings.max_records, 10_000_000);
        assert_eq!(settings.max_commit_retries, 4);
        assert_eq!(settings.max_decoded_bytes, 17_179_869_184);
        assert_eq!(settings.max_metadata_bytes, 67_108_864);
        assert_eq!(settings.host_start, Duration::from_secs(30));
        assert_eq!(settings.kill_limit, Duration::from_secs(20));
        assert_eq!(settings.relay_allow.len(), 2);
        assert_eq!(settings.large_above_bytes, None);
    }

    #[test]
    fn a_member_nobody_knows_and_a_missing_one_are_refused() {
        assert_eq!(
            refused(&with(|d| d["image"] = json!("evil")), false),
            Some("document")
        );
        for missing in [
            "workRoot",
            "sandboxSocket",
            "sandboxUid",
            "slots",
            "relayAllowCidrs",
            "relayDenyCidrs",
        ] {
            let document = with(|d| {
                d.as_object_mut().unwrap().remove(missing);
            });
            assert_eq!(refused(&document, false), Some("document"), "{missing}");
        }
    }

    #[test]
    fn paths_must_be_absolute_and_may_not_climb() {
        for (member, value) in [
            ("workRoot", "work"),
            ("workRoot", "/work/../etc"),
            ("sandboxSocket", "run/lake.sock"),
            ("sandboxSocket", "/run/../lake.sock"),
        ] {
            let document = with(|d| d[member] = json!(value));
            assert_eq!(refused(&document, false), Some(member), "{member} {value}");
        }
    }

    #[test]
    fn the_sandbox_runner_is_not_root_and_not_a_user_of_the_containers() {
        for uid in [0_u64, 40_000, 40_000 + 4095, 65_534, u64::from(u32::MAX)] {
            let document = with(|d| d["sandboxUid"] = json!(uid));
            assert_eq!(refused(&document, false), Some("sandboxUid"), "{uid}");
        }
        for uid in [1_u64, 39_999, 44_096] {
            let document = with(|d| d["sandboxUid"] = json!(uid));
            assert_eq!(refused(&document, false), None, "{uid}");
        }
    }

    #[test]
    fn the_slots_are_one_to_the_most_the_users_allow() {
        for slots in [0_u64, 4097] {
            let document = with(|d| d["slots"] = json!(slots));
            assert_eq!(refused(&document, false), Some("slots"), "{slots}");
        }
        for slots in [1_u64, 4096] {
            let document = with(|d| d["slots"] = json!(slots));
            assert_eq!(refused(&document, false), None, "{slots}");
        }
    }

    #[test]
    fn an_empty_allowance_reaches_nothing_and_is_refused_unless_it_is_development() {
        let document = with(|d| d["relayAllowCidrs"] = json!([]));
        assert_eq!(refused(&document, false), Some("relayAllowCidrs"));
        assert_eq!(refused(&document, true), None);
    }

    #[test]
    fn the_platforms_own_ranges_must_be_denied_and_an_empty_list_is_refused_unless_it_is_development()
     {
        let document = with(|d| d["relayDenyCidrs"] = json!([]));
        assert_eq!(refused(&document, false), Some("relayDenyCidrs"));
        assert_eq!(refused(&document, true), None);
        for bad in ["10.0.0.0", "not-a-range", "10.0.0.0/33", ""] {
            let document = with(|d| d["relayDenyCidrs"] = json!([bad]));
            assert_eq!(refused(&document, false), Some("relayDenyCidrs"), "{bad:?}");
        }
        let many: Vec<String> = (0..257)
            .map(|n| format!("10.{}.{}.0/24", n / 256, n % 256))
            .collect();
        let document = with(|d| d["relayDenyCidrs"] = json!(many));
        assert_eq!(refused(&document, false), Some("relayDenyCidrs"));
        let parsed = read(&section()).unwrap().validated(false).unwrap();
        assert_eq!(parsed.relay_deny.len(), 2);
    }

    #[test]
    fn the_work_root_leaves_room_for_the_relays_socket_path() {
        // <root>/<36>/relay.sock must fit in 103 bytes
        let longest = format!("/{}", "r".repeat(MAX_WORK_ROOT_BYTES - 1));
        assert_eq!(longest.len(), MAX_WORK_ROOT_BYTES);
        let document = with(|d| d["workRoot"] = json!(longest));
        assert_eq!(refused(&document, false), None);
        let socket = format!("{longest}/{}/relay.sock", "a".repeat(36));
        assert!(socket.len() < 104, "{}", socket.len());

        let one_more = format!("{longest}r");
        let document = with(|d| d["workRoot"] = json!(one_more));
        assert_eq!(refused(&document, false), Some("workRoot"));
    }

    #[test]
    fn an_allowance_that_is_not_an_address_range_is_refused() {
        for bad in ["10.0.0.0", "not-a-range", "10.0.0.0/33", "::/129", ""] {
            let document = with(|d| d["relayAllowCidrs"] = json!([bad]));
            assert_eq!(
                refused(&document, false),
                Some("relayAllowCidrs"),
                "{bad:?}"
            );
        }
        let many: Vec<String> = (0..257)
            .map(|n| format!("10.{}.{}.0/24", n / 256, n % 256))
            .collect();
        let document = with(|d| d["relayAllowCidrs"] = json!(many));
        assert_eq!(refused(&document, false), Some("relayAllowCidrs"));
        let most: Vec<String> = many.into_iter().take(256).collect();
        let document = with(|d| d["relayAllowCidrs"] = json!(most));
        assert_eq!(refused(&document, false), None);
    }

    #[test]
    fn every_number_is_bounded_on_both_sides_and_the_edges_are_taken() {
        let cases: [(&str, u64, u64); 7] = [
            ("maxBytes", MIB, 64 * GIB),
            ("maxRecords", 1, 100_000_000),
            ("maxDecodedBytes", MIB, i64::MAX as u64),
            ("maxMetadataBytes", MIB, i64::MAX as u64),
            ("hostStartSecs", 1, 600),
            ("killLimitSecs", 1, 300),
            ("maxCommitRetries", 0, 16),
        ];
        for (member, low, high) in cases {
            for value in [low, high] {
                let document = with(|d| d[member] = json!(value));
                assert_eq!(refused(&document, false), None, "{member}={value}");
            }
            for value in [low.checked_sub(1), high.checked_add(1)]
                .into_iter()
                .flatten()
            {
                let document = with(|d| d[member] = json!(value));
                assert_eq!(refused(&document, false), Some(member), "{member}={value}");
            }
        }
    }

    #[test]
    fn the_large_threshold_is_positive_and_not_above_the_attempt_ceiling() {
        for above in [0_u64, GIB + 1] {
            let document = with(|d| d["largeAboveBytes"] = json!(above));
            assert_eq!(
                refused(&document, false),
                Some("largeAboveBytes"),
                "{above}"
            );
        }
        for above in [1_u64, GIB] {
            let document = with(|d| d["largeAboveBytes"] = json!(above));
            assert_eq!(refused(&document, false), None, "{above}");
        }
    }
}
