//! Redis settings round-trip through their canonical wire representation without changing TLS/DB.
use proptest::prelude::*;
use sagat_domain::outbound::{BrokerDestination, redis_endpoint};
proptest! {
    #[test]
    fn redis_ipv6_settings_round_trip(segments in any::<[u16;8]>(), port in 1_u16..=u16::MAX, database in 0_u8..=15, tls in any::<bool>(), stream in "[a-zA-Z0-9:_{}.-]{1,64}") {
        let host = std::net::Ipv6Addr::from(segments);
        let endpoint = format!("{}://[{host}]:{port}", if tls {"rediss"} else {"redis"});
        let destination = BrokerDestination::redis(&endpoint, &stream, database).expect("valid");
        let BrokerDestination::Redis { server, .. } = &destination else { unreachable!() };
        prop_assert_eq!(server.tls(), tls);
        prop_assert_eq!(&BrokerDestination::redis(&redis_endpoint(server), &stream, database).expect("round trip"), &destination);
    }
}
#[test]
fn malformed_redis_authorities_are_refused_instead_of_normalized_into_another_host() {
    for endpoint in [
        "redis://[[::1]]:6379",
        "redis://::1:6379",
        "rediss://[bad::ip]:6379",
        "redis://user:pass@host:6379",
        "redis://host:6379/1",
        "redis://host:0",
    ] {
        assert!(
            BrokerDestination::redis(endpoint, "events", 0).is_err(),
            "{endpoint}"
        );
    }
}
