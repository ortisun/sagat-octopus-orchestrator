//! Keyed digests of flow-control keys (feature 022, T022-007; D1168, NFR-022-006, FR-022-053).
//!
//! A concurrency or rate-limit key is often personal data (an account, a customer). A plain SHA-256 of it, visible to
//! anyone who reads runs or listings, would be an offline oracle: guess a value, hash it, compare. So a key is stored
//! and shown only as an HMAC-SHA-256 under a 32-byte deployment key, separated by purpose, tenant and namespace, and
//! shown as the 12-hex label of that digest. The key material is configuration of the control plane and the engine,
//! never a row; which version is current is a row (`flow_control_key_versions`, read in each deciding transaction).
//!
//! Nothing here prints a key: [`FlowControlKey`] and [`KeyRing`] redact themselves, and a [`KeyedDigest`] shows only
//! its label.

use std::collections::BTreeMap;
use std::fmt;

use ring::{aead, hmac};
use sagat_domain::execution::KeyLabel;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::workflow::flow_control::MAX_KEY_BYTES;

/// Domain separation: what is digested is a flow-control key of this scheme, and nothing else the same key could sign.
const SCHEME: &[u8] = b"sagat.flow-control.key.v1\0";

/// What a key is the key of: a concurrency key and a rate-limit key of the same value never share a digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPurpose {
    Concurrency,
    RateLimit,
}

impl KeyPurpose {
    const fn tag(self) -> &'static [u8] {
        match self {
            Self::Concurrency => b"concurrency\0",
            Self::RateLimit => b"rate-limit\0",
        }
    }
}

/// Why a key or a key configuration was refused. Never names a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyedDigestError {
    /// The canonical value of a key is longer than [`MAX_KEY_BYTES`] (FR-022-054: a step fails `FLOW_CONTROL_KEY_INVALID`).
    #[error("a flow-control key's canonical value is longer than {MAX_KEY_BYTES} bytes")]
    KeyTooLong,
    /// A configured key is not `version:hex` with a version of at least 1 and 32 bytes of hexadecimal.
    #[error(
        "a flow-control digest key is `version:hex`, the version at least 1 and the key 32 bytes in hexadecimal"
    )]
    MalformedKey,
    /// The same version is configured twice.
    #[error("a flow-control digest key version is configured twice")]
    DuplicateVersion,
    /// The version the database says is current has no key in this process's configuration.
    #[error(
        "the current flow-control digest key version has no key in this process's configuration"
    )]
    UnknownVersion,
}

/// One version of the digest key.
pub struct FlowControlKey {
    version: u16,
    key: hmac::Key,
}

impl FlowControlKey {
    #[must_use]
    pub fn new(version: u16, material: &[u8; 32]) -> Self {
        Self {
            version,
            key: hmac::Key::new(hmac::HMAC_SHA256, material),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// The digest of a key's canonical value (its canonical JSON text) for `purpose` in one namespace of one tenant.
    ///
    /// # Errors
    /// [`KeyedDigestError::KeyTooLong`] above [`MAX_KEY_BYTES`].
    pub fn digest(
        &self,
        purpose: KeyPurpose,
        tenant: TenantId,
        namespace: NamespaceId,
        canonical: &[u8],
    ) -> Result<KeyedDigest, KeyedDigestError> {
        if canonical.len() > MAX_KEY_BYTES {
            return Err(KeyedDigestError::KeyTooLong);
        }
        let mut context = hmac::Context::with_key(&self.key);
        context.update(SCHEME);
        context.update(purpose.tag());
        context.update(tenant.as_uuid().as_bytes());
        context.update(namespace.as_uuid().as_bytes());
        context.update(
            &u16::try_from(canonical.len())
                .unwrap_or(u16::MAX)
                .to_be_bytes(),
        );
        context.update(canonical);
        let tag = context.sign();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(tag.as_ref());
        Ok(KeyedDigest {
            version: self.version,
            bytes,
        })
    }
}

impl fmt::Debug for FlowControlKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FlowControlKey")
            .field("version", &self.version)
            .field("key", &"[redacted]")
            .finish()
    }
}

/// The digest of one key under one key version. It is what a semaphore or window row is filed under; it is shown only
/// as its label.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct KeyedDigest {
    version: u16,
    bytes: [u8; 32],
}

impl KeyedDigest {
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// The 12-hex label listings and the journal show.
    #[must_use]
    pub fn label(&self) -> KeyLabel {
        KeyLabel::of_digest(&self.bytes)
    }
}

impl fmt::Debug for KeyedDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyedDigest(v{}, {})", self.version, self.label())
    }
}

/// The versions of the digest key this process was configured with (the file `SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE` names, a
/// comma-separated list of `version:hex`, D022-057). Which one is current is the database's row, never this list
/// (FR-022-053).
#[derive(Default)]
pub struct KeyRing {
    keys: BTreeMap<u16, FlowControlKey>,
}

impl KeyRing {
    /// Reads `version:hex[,version:hex...]`. An empty text is a ring with no key (a deployment without flow control).
    ///
    /// # Errors
    /// [`KeyedDigestError::MalformedKey`] or [`KeyedDigestError::DuplicateVersion`]; the message names no key.
    pub fn parse(text: &str) -> Result<Self, KeyedDigestError> {
        let mut keys = BTreeMap::new();
        for entry in text
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            let (version, hex) = entry
                .split_once(':')
                .ok_or(KeyedDigestError::MalformedKey)?;
            let version: u16 = version
                .trim()
                .parse()
                .ok()
                .filter(|version| *version >= 1 && *version <= 32_767)
                .ok_or(KeyedDigestError::MalformedKey)?;
            let material = decode_key(hex.trim()).ok_or(KeyedDigestError::MalformedKey)?;
            if keys
                .insert(version, FlowControlKey::new(version, &material))
                .is_some()
            {
                return Err(KeyedDigestError::DuplicateVersion);
            }
        }
        Ok(Self { keys })
    }

    /// The key of the version the database says is current.
    ///
    /// # Errors
    /// [`KeyedDigestError::UnknownVersion`] when this process was not given it: it must not decide a key it cannot
    /// digest as the others do.
    pub fn current(&self, version: u16) -> Result<&FlowControlKey, KeyedDigestError> {
        self.keys
            .get(&version)
            .ok_or(KeyedDigestError::UnknownVersion)
    }

    /// The versions configured, in order.
    pub fn versions(&self) -> impl Iterator<Item = u16> + '_ {
        self.keys.keys().copied()
    }
}

/// The versions of the digest key a deciding transaction reads (`flow_control_key_versions_in_force`, FR-022-053): the
/// active one, and the one a rotation is retiring while rows still use it (FR-022-056).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyVersions {
    pub active: u16,
    pub retiring: Option<u16>,
}

/// A key's digests under the versions in force. The row filed under [`Self::retiring`] wins when it exists, else the key
/// is filed under [`Self::active`]: no key is ever split between two rows during a rotation (FR-022-056), unless the
/// operator forces the retiring version out (ADR-0093 decision 5), which leaves a key at most two rows until the old drains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCandidates {
    pub active: KeyedDigest,
    pub retiring: Option<KeyedDigest>,
}

impl KeyRing {
    /// The digests of a key's canonical value under every version in force.
    ///
    /// # Errors
    /// [`KeyedDigestError::UnknownVersion`] when this process lacks the key of either version: it must not decide a key
    /// it cannot digest as the others do; [`KeyedDigestError::KeyTooLong`] above [`MAX_KEY_BYTES`].
    pub fn candidates(
        &self,
        versions: KeyVersions,
        purpose: KeyPurpose,
        tenant: TenantId,
        namespace: NamespaceId,
        canonical: &[u8],
    ) -> Result<KeyCandidates, KeyedDigestError> {
        let active = self
            .current(versions.active)?
            .digest(purpose, tenant, namespace, canonical)?;
        let retiring = versions
            .retiring
            .map(|version| {
                self.current(version)?
                    .digest(purpose, tenant, namespace, canonical)
            })
            .transpose()?;
        Ok(KeyCandidates { active, retiring })
    }
}

/// Domain separation of the key that seals listing cursors: derived from a digest key version by an HMAC of this text,
/// which no key digest can equal (every digested message starts with [`SCHEME`]).
const CURSOR_SCHEME: &[u8] = b"sagat.flow-control.cursor.v1\0";

/// The prefix of a sealed cursor's text.
const SEALED_PREFIX: &str = "c1.";

impl KeyRing {
    /// Seals `plain`, the position of a listing page, for the listing of one namespace of one tenant: encrypted and
    /// authenticated (ChaCha20-Poly1305) under a key derived from `version`, the version the database has active (which
    /// every process of a rotation holds, D022-059), so a caller learns nothing from the text (not even a key's digest)
    /// and cannot alter it or replay it in another namespace. `None` without that version, above
    /// [`MAX_CURSOR_PLAIN_BYTES`], or without a source of randomness.
    #[must_use]
    pub fn seal_cursor(
        &self,
        version: u16,
        tenant: TenantId,
        namespace: NamespaceId,
        plain: &[u8],
    ) -> Option<String> {
        use ring::rand::SecureRandom as _;
        if plain.len() > MAX_CURSOR_PLAIN_BYTES {
            return None;
        }
        let key = self.keys.get(&version)?;
        let mut nonce = [0u8; aead::NONCE_LEN];
        ring::rand::SystemRandom::new().fill(&mut nonce).ok()?;
        let mut sealed = plain.to_vec();
        cursor_key(key)?
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(cursor_aad(key.version, tenant, namespace).as_slice()),
                &mut sealed,
            )
            .ok()?;
        let mut bytes = Vec::with_capacity(2 + nonce.len() + sealed.len());
        bytes.extend_from_slice(&key.version.to_be_bytes());
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&sealed);
        let mut text = String::with_capacity(SEALED_PREFIX.len() + bytes.len() * 2);
        text.push_str(SEALED_PREFIX);
        for byte in bytes {
            text.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
            text.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
        }
        Some(text)
    }

    /// The digest key version a cursor names in clear (its first two bytes), unauthenticated: only meaningful once
    /// [`Self::open_cursor`] accepted the same text, since the version is part of the cursor's associated data. A caller
    /// checks it against the versions the database has in force, so that a version taken out of force (a forced
    /// retirement of a leaked key, ADR-0093 decision 5) opens no cursor although a process still holds it.
    #[must_use]
    pub fn sealed_version(text: &str) -> Option<u16> {
        let hex = text.strip_prefix(SEALED_PREFIX)?.as_bytes();
        if hex.len() < 4 {
            return None;
        }
        let mut version = 0u16;
        for digit in &hex[..4] {
            version = version << 4 | u16::from(hex_value(*digit)?);
        }
        Some(version)
    }

    /// Opens a cursor [`Self::seal_cursor`] wrote for this namespace of this tenant: `None` for any other text, an
    /// altered one, one written for another namespace, or one sealed under a version this process no longer holds.
    #[must_use]
    pub fn open_cursor(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        text: &str,
    ) -> Option<Vec<u8>> {
        let hex = text.strip_prefix(SEALED_PREFIX)?;
        // a sealed position is at most a few hundred bytes; refuse long text before decoding it
        if hex.len() > 2 * MAX_SEALED_BYTES || !hex.len().is_multiple_of(2) {
            return None;
        }
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        for pair in hex.as_bytes().chunks(2) {
            let high = hex_value(pair[0])?;
            let low = hex_value(pair[1])?;
            bytes.push(high << 4 | low);
        }
        if bytes.len() < 2 + aead::NONCE_LEN + aead::CHACHA20_POLY1305.tag_len() {
            return None;
        }
        let version = u16::from_be_bytes([bytes[0], bytes[1]]);
        let key = self.keys.get(&version)?;
        let nonce: [u8; aead::NONCE_LEN] = bytes[2..2 + aead::NONCE_LEN].try_into().ok()?;
        let mut sealed = bytes[2 + aead::NONCE_LEN..].to_vec();
        let plain = cursor_key(key)?
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(cursor_aad(version, tenant, namespace).as_slice()),
                &mut sealed,
            )
            .ok()?;
        Some(plain.to_vec())
    }
}

/// The longest position a cursor seals. The longest a key listing writes is 364 bytes: `k1.` (3), an instant of at most
/// 19 digits, `.` (1), `DRAFT_TEST` (10), `.` (1), `STEP` (4), `.` (1), a digest in hexadecimal (64), `.` (1) and a group
/// of at most 130 bytes in hexadecimal (260). A group view's place of a group read in part (decision D022-086 item 5) is
/// at most 450: `kg1.n.` (6), the same pool, scope, digest and group (342), and its partial sums, three counts of at most
/// 20 digits and two instants of at most 19, each after a `.` (103). Every position is sealed after the instant of its
/// page, at most 19 digits and a `.` (decision D022-100), so at most 470; 512 leaves room without ever cutting one off.
pub const MAX_CURSOR_PLAIN_BYTES: usize = 512;

/// The longest sealed cursor accepted, in bytes: a version, a nonce, a tag and a position.
const MAX_SEALED_BYTES: usize = 2 + 12 + 16 + MAX_CURSOR_PLAIN_BYTES;

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

const fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    }
}

fn cursor_key(key: &FlowControlKey) -> Option<aead::LessSafeKey> {
    let derived = hmac::sign(&key.key, CURSOR_SCHEME);
    aead::UnboundKey::new(&aead::CHACHA20_POLY1305, derived.as_ref())
        .ok()
        .map(aead::LessSafeKey::new)
}

fn cursor_aad(version: u16, tenant: TenantId, namespace: NamespaceId) -> Vec<u8> {
    let mut aad = Vec::with_capacity(CURSOR_SCHEME.len() + 2 + 16 + 16);
    aad.extend_from_slice(CURSOR_SCHEME);
    aad.extend_from_slice(&version.to_be_bytes());
    aad.extend_from_slice(tenant.as_uuid().as_bytes());
    aad.extend_from_slice(namespace.as_uuid().as_bytes());
    aad
}

impl fmt::Debug for KeyRing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyRing")
            .field("versions", &self.keys.keys().collect::<Vec<_>>())
            .finish()
    }
}

fn decode_key(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        // `from_str_radix` alone would take a sign (`+f`)
        return None;
    }
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let text = std::str::from_utf8(chunk).ok()?;
        out[i] = u8::from_str_radix(text, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    fn ids() -> (TenantId, NamespaceId) {
        (TenantId::generate(), NamespaceId::generate())
    }

    #[test]
    fn the_digest_is_stable_and_separated_by_purpose_tenant_namespace_and_key() {
        let key = FlowControlKey::new(1, &[7; 32]);
        let (tenant, namespace) = ids();
        let value = br#""acct-42""#;
        let a = key
            .digest(KeyPurpose::Concurrency, tenant, namespace, value)
            .expect("digest");
        assert_eq!(
            a,
            key.digest(KeyPurpose::Concurrency, tenant, namespace, value)
                .expect("again"),
            "the same key of the same namespace is the same row"
        );
        let (other_tenant, other_namespace) = ids();
        for other in [
            key.digest(KeyPurpose::RateLimit, tenant, namespace, value),
            key.digest(KeyPurpose::Concurrency, other_tenant, namespace, value),
            key.digest(KeyPurpose::Concurrency, tenant, other_namespace, value),
            key.digest(KeyPurpose::Concurrency, tenant, namespace, br#""acct-43""#),
            FlowControlKey::new(2, &[8; 32]).digest(
                KeyPurpose::Concurrency,
                tenant,
                namespace,
                value,
            ),
        ] {
            assert_ne!(other.expect("digest").as_bytes(), a.as_bytes());
        }
        let plain: [u8; 32] = {
            use sha2::Digest as _;
            sha2::Sha256::digest(value).into()
        };
        assert_ne!(
            a.as_bytes(),
            &plain,
            "not a plain hash anyone could recompute"
        );
        assert_eq!(a.label().as_str().len(), 12);
    }

    #[test]
    fn a_key_is_at_most_its_byte_ceiling() {
        let key = FlowControlKey::new(1, &[7; 32]);
        let (tenant, namespace) = ids();
        assert!(
            key.digest(
                KeyPurpose::RateLimit,
                tenant,
                namespace,
                &[b'a'; MAX_KEY_BYTES]
            )
            .is_ok()
        );
        assert_eq!(
            key.digest(
                KeyPurpose::RateLimit,
                tenant,
                namespace,
                &[b'a'; MAX_KEY_BYTES + 1]
            ),
            Err(KeyedDigestError::KeyTooLong)
        );
    }

    /// The log canary: nothing a process can print of a key, a ring or a digest carries the key material or the value.
    #[test]
    fn nothing_printed_carries_the_key_or_the_value() {
        let ring = KeyRing::parse(&format!("1:{HEX}, 2:{}", "ab".repeat(32))).expect("ring");
        let key = ring.current(1).expect("version 1");
        let (tenant, namespace) = ids();
        let digest = key
            .digest(
                KeyPurpose::Concurrency,
                tenant,
                namespace,
                b"\"canary-value-7731\"",
            )
            .expect("digest");
        let printed = format!(
            "{ring:?} {key:?} {digest:?} {:?}",
            ring.versions().collect::<Vec<_>>()
        );
        for secret in [HEX, &"ab".repeat(32), "canary-value-7731", "0001020304"] {
            assert!(!printed.contains(secret), "{printed}");
        }
        assert!(printed.contains("[redacted]") && printed.contains(digest.label().as_str()));
        let full_hex: String = digest
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert!(
            !printed.contains(&full_hex),
            "the digest is shown only as its label"
        );
    }

    #[test]
    fn a_ring_is_read_strictly_and_names_no_key_when_it_refuses() {
        assert_eq!(KeyRing::parse("").expect("empty").versions().count(), 0);
        let ring = KeyRing::parse(&format!("3:{HEX}")).expect("one");
        assert_eq!(ring.current(3).expect("v3").version(), 3);
        assert!(matches!(
            ring.current(4),
            Err(KeyedDigestError::UnknownVersion)
        ));
        for bad in [
            format!("0:{HEX}"),
            format!("x:{HEX}"),
            "1:abcd".to_owned(),
            format!("1:{}", "zz".repeat(32)),
            format!("1:{}", "+f".repeat(32)),
            HEX.to_owned(),
        ] {
            let refused = KeyRing::parse(&bad).map(|_| ()).unwrap_err();
            assert_eq!(refused, KeyedDigestError::MalformedKey, "{bad}");
            assert!(!refused.to_string().contains(HEX));
        }
        assert_eq!(
            KeyRing::parse(&format!("1:{HEX},1:{HEX}")).map(|_| ()),
            Err(KeyedDigestError::DuplicateVersion)
        );
    }

    /// FR-022-056: during a rotation a decider digests the key under both versions; outside one, under the active only;
    /// a process missing either key refuses to decide.
    #[test]
    fn the_candidates_are_the_digests_under_every_version_in_force() {
        let ring = KeyRing::parse(&format!("1:{HEX},2:{}", "ab".repeat(32))).expect("ring");
        let (tenant, namespace) = ids();
        let value = br#""acct-42""#;
        let quiet = ring
            .candidates(
                KeyVersions {
                    active: 2,
                    retiring: None,
                },
                KeyPurpose::Concurrency,
                tenant,
                namespace,
                value,
            )
            .expect("candidates");
        assert_eq!(quiet.active.version(), 2);
        assert!(quiet.retiring.is_none());
        let rotating = ring
            .candidates(
                KeyVersions {
                    active: 2,
                    retiring: Some(1),
                },
                KeyPurpose::Concurrency,
                tenant,
                namespace,
                value,
            )
            .expect("candidates");
        assert_eq!(rotating.active, quiet.active);
        let old = rotating.retiring.expect("the retiring digest");
        assert_eq!(old.version(), 1);
        assert_eq!(
            old,
            ring.current(1)
                .expect("v1")
                .digest(KeyPurpose::Concurrency, tenant, namespace, value)
                .expect("digest")
        );
        assert_ne!(old.as_bytes(), rotating.active.as_bytes());
        let half = KeyRing::parse(&format!("2:{}", "ab".repeat(32))).expect("ring");
        assert_eq!(
            half.candidates(
                KeyVersions {
                    active: 2,
                    retiring: Some(1)
                },
                KeyPurpose::Concurrency,
                tenant,
                namespace,
                value,
            ),
            Err(KeyedDigestError::UnknownVersion),
            "a process without the retiring key cannot find the old row"
        );
    }

    #[test]
    fn a_sealed_cursor_opens_only_unaltered_in_its_namespace_under_a_held_version() {
        let ring = KeyRing::parse(&format!("1:{HEX},2:{}", "ab".repeat(32))).expect("ring");
        let (tenant, namespace) = ids();
        let plain = b"position of the last key";
        let text = ring
            .seal_cursor(2, tenant, namespace, plain)
            .expect("sealed");
        assert!(text.starts_with("c1."), "{text}");
        // opaque: neither the position nor any part of it is readable in the text
        assert!(
            !text.contains(
                &"position"
                    .bytes()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            )
        );
        assert_eq!(
            ring.open_cursor(tenant, namespace, &text).as_deref(),
            Some(&plain[..])
        );
        // sealed under the version asked for (the active one), with a fresh nonce each time
        assert_eq!(&text[3..7], "0002");
        assert_ne!(
            ring.seal_cursor(2, tenant, namespace, plain),
            Some(text.clone())
        );
        let older = KeyRing::parse(&format!("1:{HEX}")).expect("ring");
        // during a rotation a process that holds both versions opens what one holding the active one sealed
        let under_one = older
            .seal_cursor(1, tenant, namespace, plain)
            .expect("sealed");
        assert_eq!(&under_one[3..7], "0001");
        assert_eq!(
            ring.open_cursor(tenant, namespace, &under_one).as_deref(),
            Some(&plain[..])
        );
        assert_eq!(
            ring.seal_cursor(3, tenant, namespace, plain),
            None,
            "a version not held"
        );
        // the longest position is sealed and opened; a longer one is not sealed
        let longest = vec![b'a'; MAX_CURSOR_PLAIN_BYTES];
        let sealed = ring
            .seal_cursor(2, tenant, namespace, &longest)
            .expect("the longest position");
        assert_eq!(ring.open_cursor(tenant, namespace, &sealed), Some(longest));
        assert_eq!(
            ring.seal_cursor(2, tenant, namespace, &[b'a'; MAX_CURSOR_PLAIN_BYTES + 1]),
            None
        );
        // every single altered byte is refused, the version and nonce included
        for at in 3..text.len() {
            let mut altered = text.clone().into_bytes();
            altered[at] = if altered[at] == b'0' { b'1' } else { b'0' };
            let altered = String::from_utf8(altered).expect("text");
            assert_eq!(
                ring.open_cursor(tenant, namespace, &altered),
                None,
                "byte {at}"
            );
        }
        // another namespace or tenant, a ring without that version, a truncated or overlong text, other text
        assert_eq!(
            ring.open_cursor(tenant, NamespaceId::generate(), &text),
            None
        );
        assert_eq!(
            ring.open_cursor(TenantId::generate(), namespace, &text),
            None
        );
        assert_eq!(
            older.open_cursor(tenant, namespace, &text),
            None,
            "a version this process does not hold"
        );
        assert_eq!(
            ring.open_cursor(tenant, namespace, &text[..text.len() - 2]),
            None
        );
        assert_eq!(
            ring.open_cursor(tenant, namespace, &format!("{text}{}", "00".repeat(400))),
            None
        );
        for other in ["", "c1.", "c1.zz", "k1.n.ORDINARY", &text.to_uppercase()] {
            assert_eq!(ring.open_cursor(tenant, namespace, other), None, "{other}");
        }
        // a ring with no key seals nothing
        assert_eq!(
            KeyRing::default().seal_cursor(1, tenant, namespace, plain),
            None
        );
    }
}
