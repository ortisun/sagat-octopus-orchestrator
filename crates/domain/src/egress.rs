//! Egress policy: which destinations a tenant's code and integrations may reach (`FR-009-008`).
//!
//! The policy is default-deny and has two stages because names lie. [`EgressPolicy::check_destination`]
//! looks at what the caller asked for (scheme, host, port) before any name is resolved;
//! [`EgressPolicy::check_address`] looks at every address the name resolved to, so a hostile DNS
//! answer (a private address, the cloud metadata service, an IPv4 address smuggled inside an IPv6
//! one) cannot be reached through an innocent-looking, allow-listed name. The adapter that actually
//! connects must connect to an address that passed the second stage, never resolve again.
//!
//! Some addresses can never be allowed, by any rule: loopback, link-local (which includes the cloud
//! metadata service), unspecified, multicast and reserved space. Private and shared ranges are
//! denied unless an operator allows them by an explicit network rule.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

/// Most rules one policy may hold.
pub const MAX_RULES: usize = 256;
/// Longest host name (RFC 1035).
const MAX_HOST: usize = 253;

/// Why a policy cannot be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EgressError {
    #[error("a policy holds at most {MAX_RULES} rules")]
    TooManyRules,
    #[error(
        "'{0}' is not a valid host rule (an exact host name, '*.' and a domain, or a CIDR network)"
    )]
    InvalidRule(String),
    #[error("port 0 is not a port")]
    InvalidPort,
}

/// An IP network in CIDR notation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    network: IpAddr,
    prefix: u8,
}

impl Cidr {
    /// # Errors
    /// [`EgressError::InvalidRule`] for a malformed address or prefix.
    pub fn parse(text: &str) -> Result<Self, EgressError> {
        let bad = || EgressError::InvalidRule(clip(text));
        let (address, prefix) = text
            .split_once('/')
            .map_or((text, None), |(a, p)| (a, Some(p)));
        let address = IpAddr::from_str(address).map_err(|_| bad())?;
        let max = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => max,
            Some(p) => p.parse::<u8>().ok().filter(|p| *p <= max).ok_or_else(bad)?,
        };
        Ok(Self {
            network: mask(address, prefix),
            prefix,
        })
    }

    #[must_use]
    pub fn contains(&self, address: IpAddr) -> bool {
        match (self.network, address) {
            (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_)) => {
                mask(address, self.prefix) == self.network
            }
            _ => false,
        }
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix)
    }
}

fn mask(address: IpAddr, prefix: u8) -> IpAddr {
    match address {
        IpAddr::V4(v4) => {
            let bits = u32::from(v4);
            let keep = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - u32::from(prefix))
            };
            IpAddr::V4(Ipv4Addr::from(bits & keep))
        }
        IpAddr::V6(v6) => {
            let bits = u128::from(v6);
            let keep = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - u32::from(prefix))
            };
            IpAddr::V6(Ipv6Addr::from(bits & keep))
        }
    }
}

fn clip(text: &str) -> String {
    text.chars().take(64).collect()
}

/// One entry of an allow list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostRule {
    /// Exactly this host name (lower case).
    Exact(String),
    /// Any host below this domain: `*.example.com` allows `a.example.com` and `a.b.example.com`, not
    /// `example.com` and not `evilexample.com`.
    Below(String),
    /// Any address in this network, which also allows addressing it by IP literal.
    Network(Cidr),
}

impl fmt::Display for HostRule {
    /// The rule as it is written: what [`HostRule::parse`] accepts gives the same rule back.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(host) => f.write_str(host),
            Self::Below(domain) => write!(f, "*.{domain}"),
            Self::Network(network) => write!(f, "{network}"),
        }
    }
}

impl HostRule {
    /// # Errors
    /// [`EgressError::InvalidRule`].
    pub fn parse(text: &str) -> Result<Self, EgressError> {
        let bad = || EgressError::InvalidRule(clip(text));
        let text = text.trim();
        if text.is_empty() || text.len() > MAX_HOST + 2 {
            return Err(bad());
        }
        if let Some(domain) = text.strip_prefix("*.") {
            return valid_name(domain).map(Self::Below).ok_or_else(bad);
        }
        if text.contains('/') || IpAddr::from_str(text).is_ok() {
            return Cidr::parse(text).map(Self::Network);
        }
        valid_name(text).map(Self::Exact).ok_or_else(bad)
    }
}

/// A lower-case DNS name made of `[a-z0-9-]` labels: no empty label, no leading or trailing hyphen,
/// at most 63 characters per label, and not a number in disguise.
fn valid_name(text: &str) -> Option<String> {
    let name = text.to_ascii_lowercase();
    if name.is_empty() || name.len() > MAX_HOST || name.ends_with('.') || looks_numeric(&name) {
        return None;
    }
    name.split('.')
        .all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        .then_some(name)
}

/// Whether the last label is a number, which resolvers read as an IPv4 address in forms (`127.1`,
/// `0x7f000001`, `2130706433`, `0177.0.0.1`) that a naive parser does not recognise.
fn looks_numeric(name: &str) -> bool {
    let last = name.rsplit('.').next().unwrap_or(name);
    !last.is_empty()
        && (last.bytes().all(|b| b.is_ascii_digit())
            || last
                .strip_prefix("0x")
                .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit())))
}

/// What the caller asked to reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Destination<'a> {
    pub scheme: &'a str,
    pub host: &'a str,
    pub port: u16,
}

/// The kind of address that was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressClass {
    Loopback,
    LinkLocal,
    Unspecified,
    Multicast,
    Reserved,
    /// RFC 1918 and unique local addresses: reachable only through an explicit network rule.
    Private,
    /// 100.64.0.0/10 and other shared or special-purpose space: likewise.
    Shared,
    Documentation,
    /// A known cloud metadata endpoint.
    Metadata,
}

impl AddressClass {
    /// Whether no rule may ever allow it.
    #[must_use]
    pub const fn never_allowed(self) -> bool {
        matches!(
            self,
            Self::Loopback
                | Self::LinkLocal
                | Self::Unspecified
                | Self::Multicast
                | Self::Reserved
                | Self::Metadata
        )
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loopback => "loopback",
            Self::LinkLocal => "link_local",
            Self::Unspecified => "unspecified",
            Self::Multicast => "multicast",
            Self::Reserved => "reserved",
            Self::Private => "private",
            Self::Shared => "shared",
            Self::Documentation => "documentation",
            Self::Metadata => "metadata",
        }
    }
}

/// Why a destination was refused. Safe to log and to store: it names a category, never a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denial {
    /// Only `https` (and `http` when the policy allows it) are supported.
    UnsupportedScheme,
    /// The host is empty, malformed or a number in disguise.
    InvalidHost,
    PortNotAllowed,
    /// No rule allows the host (default deny).
    NotAllowlisted,
    /// The host, or an address it resolved to, is in a category that is refused.
    ForbiddenAddress(AddressClass),
    /// The name resolved to nothing usable.
    NoAddress,
}

impl Denial {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnsupportedScheme => "UNSUPPORTED_SCHEME",
            Self::InvalidHost => "INVALID_HOST",
            Self::PortNotAllowed => "PORT_NOT_ALLOWED",
            Self::NotAllowlisted => "NOT_ALLOWLISTED",
            Self::ForbiddenAddress(_) => "FORBIDDEN_ADDRESS",
            Self::NoAddress => "NO_ADDRESS",
        }
    }
}

/// A namespace's egress policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressPolicy {
    rules: Vec<HostRule>,
    ports: Vec<u16>,
    allow_http: bool,
}

impl EgressPolicy {
    /// The policy that allows nothing.
    #[must_use]
    pub const fn deny_all() -> Self {
        Self {
            rules: Vec::new(),
            ports: Vec::new(),
            allow_http: false,
        }
    }

    /// `ports` empty means the default ports of the allowed schemes (443, and 80 for `http`).
    ///
    /// # Errors
    /// [`EgressError`] for too many rules or port 0.
    pub fn new(
        rules: Vec<HostRule>,
        ports: Vec<u16>,
        allow_http: bool,
    ) -> Result<Self, EgressError> {
        if rules.len() > MAX_RULES {
            return Err(EgressError::TooManyRules);
        }
        if ports.contains(&0) {
            return Err(EgressError::InvalidPort);
        }
        Ok(Self {
            rules,
            ports,
            allow_http,
        })
    }

    #[must_use]
    pub fn rules(&self) -> &[HostRule] {
        &self.rules
    }

    #[must_use]
    pub fn ports(&self) -> &[u16] {
        &self.ports
    }

    #[must_use]
    pub const fn allows_http(&self) -> bool {
        self.allow_http
    }

    /// First stage, before any name is resolved.
    ///
    /// # Errors
    /// The [`Denial`] that applies.
    pub fn check_destination(&self, destination: &Destination<'_>) -> Result<(), Denial> {
        match destination.scheme.to_ascii_lowercase().as_str() {
            "https" => {}
            "http" if self.allow_http => {}
            _ => return Err(Denial::UnsupportedScheme),
        }
        let port_ok = if self.ports.is_empty() {
            destination.port == 443 || (self.allow_http && destination.port == 80)
        } else {
            self.ports.contains(&destination.port)
        };
        if !port_ok {
            return Err(Denial::PortNotAllowed);
        }
        let host = destination
            .host
            .trim_start_matches('[')
            .trim_end_matches(']');
        if let Ok(address) = IpAddr::from_str(host) {
            // An address literal is never matched by a name rule: only a network rule can allow it
            // (public space included), and the address checks of the second stage apply as well.
            self.check_address(address)?;
            return if self.covered(address) {
                Ok(())
            } else {
                Err(Denial::NotAllowlisted)
            };
        }
        let Some(name) = valid_name(host) else {
            return Err(Denial::InvalidHost);
        };
        let allowed = self.rules.iter().any(|rule| match rule {
            HostRule::Exact(exact) => *exact == name,
            HostRule::Below(domain) => name
                .strip_suffix(domain.as_str())
                .is_some_and(|rest| rest.ends_with('.') && rest.len() > 1),
            HostRule::Network(_) => false,
        });
        if allowed {
            Ok(())
        } else {
            Err(Denial::NotAllowlisted)
        }
    }

    /// Second stage, for every address an allow-listed name resolved to (and, as part of the first
    /// stage, for address literals).
    ///
    /// # Errors
    /// [`Denial::ForbiddenAddress`] for a refused category, or for a private or shared address that
    /// no network rule covers.
    pub fn check_address(&self, address: IpAddr) -> Result<(), Denial> {
        let address = unwrap_embedded(address);
        match classify(address) {
            Some(class) if class.never_allowed() => Err(Denial::ForbiddenAddress(class)),
            Some(class) if !self.covered(address) => Err(Denial::ForbiddenAddress(class)),
            _ => Ok(()),
        }
    }

    /// Whether an explicit network rule covers `address`.
    fn covered(&self, address: IpAddr) -> bool {
        let address = unwrap_embedded(address);
        self.rules
            .iter()
            .any(|rule| matches!(rule, HostRule::Network(network) if network.contains(address)))
    }
}

/// An IPv4 address carried inside an IPv6 one (mapped, NAT64, 6to4) is judged as that IPv4 address.
fn unwrap_embedded(address: IpAddr) -> IpAddr {
    let IpAddr::V6(v6) = address else {
        return address;
    };
    if let Some(v4) = v6.to_ipv4_mapped() {
        return IpAddr::V4(v4);
    }
    let segments = v6.segments();
    // 64:ff9b::/96 (NAT64): the IPv4 address is the last 32 bits.
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return IpAddr::V4(Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        ));
    }
    // 2002::/16 (6to4): the IPv4 address follows the prefix.
    if segments[0] == 0x2002 {
        return IpAddr::V4(Ipv4Addr::new(
            (segments[1] >> 8) as u8,
            segments[1] as u8,
            (segments[2] >> 8) as u8,
            segments[2] as u8,
        ));
    }
    address
}

/// The category of a special-purpose address, or `None` for ordinary public space.
#[must_use]
pub fn classify(address: IpAddr) -> Option<AddressClass> {
    match address {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

fn classify_v4(a: Ipv4Addr) -> Option<AddressClass> {
    let [o0, o1, o2, _] = a.octets();
    Some(match (o0, o1, o2) {
        (100, 100, 100) if a.octets()[3] == 200 => AddressClass::Metadata,
        (0, ..) => AddressClass::Unspecified,
        (127, ..) => AddressClass::Loopback,
        (169, 254, _) => AddressClass::LinkLocal,
        (224..=239, ..) => AddressClass::Multicast,
        (240..=255, ..) => AddressClass::Reserved,
        (10, ..) | (192, 168, _) => AddressClass::Private,
        (172, 16..=31, _) => AddressClass::Private,
        (100, 64..=127, _) => AddressClass::Shared,
        (192, 0, 0) | (198, 18..=19, _) => AddressClass::Shared,
        (192, 0, 2) | (198, 51, 100) | (203, 0, 113) => AddressClass::Documentation,
        _ => return None,
    })
}

fn classify_v6(a: Ipv6Addr) -> Option<AddressClass> {
    let s = a.segments();
    Some(match () {
        () if a.is_unspecified() => AddressClass::Unspecified,
        () if a.is_loopback() => AddressClass::Loopback,
        () if a.is_multicast() => AddressClass::Multicast,
        () if s[0] & 0xffc0 == 0xfe80 => AddressClass::LinkLocal,
        // AWS' IPv6 instance metadata endpoint.
        () if s == [0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254] => AddressClass::Metadata,
        () if s[0] & 0xfe00 == 0xfc00 => AddressClass::Private,
        () if s[0] == 0x2001 && s[1] == 0x0db8 => AddressClass::Documentation,
        // Deprecated site-local, IPv4-compatible and the discard prefix are not for the internet.
        () if s[0] & 0xffc0 == 0xfec0
            || s[..6] == [0; 6]
            || s[0] == 0x0100 && s[1..4] == [0; 3] =>
        {
            AddressClass::Reserved
        }
        () => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_prints_the_way_it_is_written_and_parses_back_to_itself() {
        for text in [
            "api.example.com",
            "*.example.org",
            "203.0.113.0/24",
            "10.0.0.0/8",
            "2001:db8::/32",
            "203.0.113.7/32",
        ] {
            let rule = HostRule::parse(text).expect("rule");
            assert_eq!(rule.to_string(), text);
            assert_eq!(HostRule::parse(&rule.to_string()), Ok(rule));
        }
        // Normalisation shows: a host is lower-cased and a network is masked.
        assert_eq!(
            HostRule::parse("API.Example.COM")
                .expect("rule")
                .to_string(),
            "api.example.com"
        );
        assert_eq!(
            HostRule::parse("203.0.113.77/24")
                .expect("rule")
                .to_string(),
            "203.0.113.0/24"
        );
    }

    fn policy(rules: &[&str]) -> EgressPolicy {
        EgressPolicy::new(
            rules
                .iter()
                .map(|r| HostRule::parse(r).expect("rule"))
                .collect(),
            vec![],
            false,
        )
        .expect("policy")
    }

    fn https(host: &str) -> Destination<'_> {
        Destination {
            scheme: "https",
            host,
            port: 443,
        }
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("ip")
    }

    #[test]
    fn nothing_is_allowed_by_default() {
        let p = EgressPolicy::deny_all();
        assert_eq!(
            p.check_destination(&https("api.example.com")),
            Err(Denial::NotAllowlisted)
        );
        assert_eq!(
            p.check_destination(&https("8.8.8.8")),
            Err(Denial::NotAllowlisted),
            "not even a public address literal"
        );
    }

    #[test]
    fn exact_and_wildcard_rules_match_whole_labels_only() {
        let p = policy(&["api.example.com", "*.hooks.example.org"]);
        assert_eq!(p.check_destination(&https("api.example.com")), Ok(()));
        assert_eq!(
            p.check_destination(&https("API.Example.COM")),
            Ok(()),
            "case-insensitive"
        );
        assert_eq!(p.check_destination(&https("a.hooks.example.org")), Ok(()));
        assert_eq!(p.check_destination(&https("a.b.hooks.example.org")), Ok(()));
        for no in [
            "hooks.example.org",
            "evilhooks.example.org",
            "api.example.com.evil.net",
            "xapi.example.com",
            "example.com",
        ] {
            assert_eq!(
                p.check_destination(&https(no)),
                Err(Denial::NotAllowlisted),
                "{no}"
            );
        }
    }

    #[test]
    fn numbers_in_disguise_are_not_host_names() {
        let p = policy(&["api.example.com"]);
        for sneaky in [
            "127.1",
            "2130706433",
            "0x7f000001",
            "0177.0.0.1",
            "1.2.3.4.5",
            "api.example.com.",
            "",
            "-a.example.com",
            "a..example.com",
            "ex ample.com",
            "ex_ample.com",
            "ü.example.com",
        ] {
            let result = p.check_destination(&https(sneaky));
            assert!(
                matches!(
                    result,
                    Err(Denial::InvalidHost | Denial::ForbiddenAddress(_))
                ),
                "{sneaky:?}: {result:?}"
            );
        }
    }

    #[test]
    fn scheme_and_port_are_part_of_the_policy() {
        let p = policy(&["api.example.com"]);
        let at = |scheme, port| Destination {
            scheme,
            host: "api.example.com",
            port,
        };
        assert_eq!(
            p.check_destination(&at("http", 443)),
            Err(Denial::UnsupportedScheme)
        );
        assert_eq!(
            p.check_destination(&at("ftp", 21)),
            Err(Denial::UnsupportedScheme)
        );
        assert_eq!(
            p.check_destination(&at("file", 0)),
            Err(Denial::UnsupportedScheme)
        );
        assert_eq!(
            p.check_destination(&at("https", 8443)),
            Err(Denial::PortNotAllowed)
        );
        let open = EgressPolicy::new(
            vec![HostRule::parse("api.example.com").expect("r")],
            vec![8443],
            true,
        )
        .expect("policy");
        assert_eq!(open.check_destination(&at("http", 8443)), Ok(()));
        assert_eq!(
            open.check_destination(&at("https", 443)),
            Err(Denial::PortNotAllowed)
        );
        assert_eq!(
            EgressPolicy::new(vec![], vec![0], false),
            Err(EgressError::InvalidPort)
        );
    }

    #[test]
    fn special_addresses_are_classified() {
        let cases = [
            ("127.0.0.1", Some(AddressClass::Loopback)),
            ("127.255.255.254", Some(AddressClass::Loopback)),
            ("0.0.0.0", Some(AddressClass::Unspecified)),
            ("169.254.169.254", Some(AddressClass::LinkLocal)),
            ("224.0.0.1", Some(AddressClass::Multicast)),
            ("255.255.255.255", Some(AddressClass::Reserved)),
            ("240.0.0.1", Some(AddressClass::Reserved)),
            ("10.1.2.3", Some(AddressClass::Private)),
            ("172.16.0.1", Some(AddressClass::Private)),
            ("172.31.255.255", Some(AddressClass::Private)),
            ("172.32.0.1", None),
            ("192.168.1.1", Some(AddressClass::Private)),
            ("100.64.0.1", Some(AddressClass::Shared)),
            ("100.100.100.200", Some(AddressClass::Metadata)),
            ("198.18.0.1", Some(AddressClass::Shared)),
            ("203.0.113.9", Some(AddressClass::Documentation)),
            ("8.8.8.8", None),
            ("1.1.1.1", None),
            ("::", Some(AddressClass::Unspecified)),
            ("::1", Some(AddressClass::Loopback)),
            ("fe80::1", Some(AddressClass::LinkLocal)),
            ("ff02::1", Some(AddressClass::Multicast)),
            ("fc00::1", Some(AddressClass::Private)),
            ("fd12:3456::1", Some(AddressClass::Private)),
            ("fd00:ec2::254", Some(AddressClass::Metadata)),
            ("2001:db8::1", Some(AddressClass::Documentation)),
            ("2606:4700:4700::1111", None),
        ];
        for (text, expected) in cases {
            assert_eq!(classify(ip(text)), expected, "{text}");
        }
    }

    #[test]
    fn an_ipv4_address_hidden_inside_ipv6_is_judged_as_the_ipv4_address() {
        let p = policy(&["api.example.com"]);
        for hidden in [
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "2002:7f00:1::",
            "2002:a9fe:a9fe::1",
        ] {
            assert!(
                matches!(
                    p.check_address(ip(hidden)),
                    Err(Denial::ForbiddenAddress(_))
                ),
                "{hidden}"
            );
        }
        assert_eq!(
            p.check_address(ip("::ffff:8.8.8.8")),
            Ok(()),
            "a public one stays public"
        );
    }

    #[test]
    fn private_space_needs_an_explicit_network_rule_but_the_forbidden_space_never_does() {
        let open = policy(&[
            "api.example.com",
            "10.20.0.0/16",
            "100.64.0.0/10",
            "127.0.0.0/8",
            "169.254.0.0/16",
        ]);
        // Allowed because an operator said so.
        assert_eq!(open.check_address(ip("10.20.5.5")), Ok(()));
        assert_eq!(open.check_address(ip("100.65.0.1")), Ok(()));
        // Not covered by any rule.
        assert_eq!(
            open.check_address(ip("10.21.0.1")),
            Err(Denial::ForbiddenAddress(AddressClass::Private))
        );
        assert_eq!(
            open.check_address(ip("192.168.0.1")),
            Err(Denial::ForbiddenAddress(AddressClass::Private))
        );
        // Covered by a rule, and still refused: no rule can allow these.
        assert_eq!(
            open.check_address(ip("127.0.0.1")),
            Err(Denial::ForbiddenAddress(AddressClass::Loopback))
        );
        assert_eq!(
            open.check_address(ip("169.254.169.254")),
            Err(Denial::ForbiddenAddress(AddressClass::LinkLocal))
        );
    }

    #[test]
    fn a_literal_address_needs_a_network_rule_to_be_reached_by_literal() {
        let by_name = policy(&["api.example.com"]);
        assert_eq!(
            by_name.check_destination(&https("8.8.8.8")),
            Err(Denial::NotAllowlisted),
            "a name rule does not allow a literal, public or not"
        );
        assert_eq!(
            by_name.check_destination(&https("10.0.0.5")),
            Err(Denial::ForbiddenAddress(AddressClass::Private))
        );
        assert_eq!(
            by_name.check_destination(&https("[::1]")),
            Err(Denial::ForbiddenAddress(AddressClass::Loopback))
        );
        let by_net = policy(&["10.0.0.0/24", "8.8.8.0/24"]);
        assert_eq!(by_net.check_destination(&https("10.0.0.5")), Ok(()));
        assert_eq!(by_net.check_destination(&https("8.8.8.8")), Ok(()));
        assert_eq!(
            by_net.check_destination(&https("1.1.1.1")),
            Err(Denial::NotAllowlisted)
        );
        assert_eq!(
            by_net.check_destination(&https("10.0.1.5")),
            Err(Denial::ForbiddenAddress(AddressClass::Private))
        );
        assert_eq!(
            by_net.check_destination(&https("api.example.com")),
            Err(Denial::NotAllowlisted),
            "a network rule does not allow names"
        );
    }

    #[test]
    fn rules_are_validated_and_bounded() {
        for bad in [
            "",
            " ",
            "*.",
            "*",
            "a/b",
            "10.0.0.0/33",
            "::/129",
            "exa mple.com",
            "*.127.1",
            "http://x.com",
        ] {
            assert!(HostRule::parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            HostRule::parse("*.Example.COM"),
            Ok(HostRule::Below("example.com".into()))
        );
        assert_eq!(
            HostRule::parse("10.1.2.3"),
            Ok(HostRule::Network(Cidr::parse("10.1.2.3/32").expect("cidr")))
        );
        assert_eq!(
            Cidr::parse("10.1.2.3/8").expect("cidr").to_string(),
            "10.0.0.0/8",
            "host bits are cleared"
        );
        let many = vec![HostRule::parse("a.example.com").expect("rule"); MAX_RULES + 1];
        assert_eq!(
            EgressPolicy::new(many, vec![], false),
            Err(EgressError::TooManyRules)
        );
    }

    #[test]
    fn denials_have_stable_codes_and_never_carry_the_address() {
        assert_eq!(
            Denial::ForbiddenAddress(AddressClass::Private).code(),
            "FORBIDDEN_ADDRESS"
        );
        assert_eq!(Denial::NotAllowlisted.code(), "NOT_ALLOWLISTED");
        assert!(AddressClass::Metadata.never_allowed() && !AddressClass::Private.never_allowed());
    }
}
