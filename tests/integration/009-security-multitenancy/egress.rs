//! The egress policy (FR-009-008, US03): default-deny, no policy can open the addresses that are
//! never reachable, a refusal is audited, and the policy that is stored is the one that is enforced.

use sagat_application::error::AppError;
use sagat_domain::egress::{AddressClass, Denial, Destination};
use sagat_domain::identity::{NamespaceId, Principal, Role};
use sagat_infra_egress::{EgressClient, EgressError, EgressLimits, EgressRequest};

use crate::support::engine::{World, ctx, subject_of, world};

fn ns(w: &World) -> NamespaceId {
    w.namespace.id()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

async fn set(w: &World, rules: &[&str], ports: &[u16], http: bool) {
    w.egress
        .set(&w.admin, ns(w), &strings(rules), ports, http, &ctx())
        .await
        .expect("set the policy");
}

async fn verdict(w: &World, scheme: &str, host: &str, port: u16) -> Result<(), Denial> {
    w.egress
        .check(
            w.admin.tenant_id(),
            ns(w),
            &Destination { scheme, host, port },
        )
        .await
        .expect("the lookup works")
}

// ------------------------------------------------------------- the policy

#[tokio::test]
async fn a_namespace_without_a_policy_may_reach_nothing() {
    let w = world().await;
    let shown = w.egress.get(&w.admin, ns(&w), &ctx()).await.expect("get");
    assert!(shown.rules.is_empty() && shown.ports.is_empty() && !shown.allow_http);
    for host in ["api.example.com", "example.org", "93.184.216.34"] {
        assert_eq!(
            verdict(&w, "https", host, 443).await,
            Err(Denial::NotAllowlisted),
            "{host}"
        );
    }
}

#[tokio::test]
async fn the_policy_is_stored_in_canonical_text_and_replaced_as_a_whole() {
    let w = world().await;
    set(
        &w,
        &[
            "API.Example.COM",
            "*.partner.example",
            "203.0.113.77/24",
            "api.example.com",
        ],
        &[8443, 443, 443],
        false,
    )
    .await;
    let shown = w.egress.get(&w.admin, ns(&w), &ctx()).await.expect("get");
    assert_eq!(
        shown.rules,
        ["*.partner.example", "203.0.113.0/24", "api.example.com"],
        "lower-cased, masked, sorted, without repeats"
    );
    assert_eq!(shown.ports, [443, 8443]);
    assert_eq!(shown.updated_by, "alice");

    set(&w, &["only.example.net"], &[], true).await;
    let shown = w.egress.get(&w.admin, ns(&w), &ctx()).await.expect("get");
    assert_eq!(shown.rules, ["only.example.net"], "replaced, not merged");
    assert!(shown.allow_http);

    w.egress
        .reset(&w.admin, ns(&w), &ctx())
        .await
        .expect("reset");
    assert!(
        w.egress
            .get(&w.admin, ns(&w), &ctx())
            .await
            .expect("get")
            .rules
            .is_empty()
    );
}

#[tokio::test]
async fn rules_ports_and_counts_are_validated_and_nothing_is_stored_on_refusal() {
    let w = world().await;
    let many: Vec<String> = (0..257).map(|i| format!("h{i}.example.com")).collect();
    for (what, rules, ports) in [
        (
            "a rule that is not a host",
            strings(&["not a host!"]),
            vec![],
        ),
        ("a number in disguise", strings(&["127.1"]), vec![]),
        (
            "a hexadecimal number in disguise",
            strings(&["0x7f000001"]),
            vec![],
        ),
        (
            "a hyphen at the edge",
            strings(&["-bad.example.com"]),
            vec![],
        ),
        ("an empty rule", strings(&[""]), vec![]),
        ("a malformed network", strings(&["10.0.0.0/33"]), vec![]),
        ("too many rules", many, vec![]),
        ("port zero", strings(&["a.example.com"]), vec![0]),
        (
            "too many ports",
            strings(&["a.example.com"]),
            (1..=33).collect(),
        ),
    ] {
        let result = w
            .egress
            .set(&w.admin, ns(&w), &rules, &ports, false, &ctx())
            .await;
        assert!(
            matches!(result, Err(AppError::NotAllowed(_))),
            "{what}: {result:?}"
        );
    }
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM egress_policies")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(rows, 0);
}

// ------------------------------------------------------------- what is allowed and what never is

#[tokio::test]
async fn the_policy_allows_what_it_names_and_default_denies_the_rest() {
    let w = world().await;
    set(
        &w,
        &[
            "api.example.com",
            "*.partner.example",
            "203.0.113.0/24",
            "10.1.0.0/16",
        ],
        &[443, 8443],
        false,
    )
    .await;
    for (scheme, host, port) in [
        ("https", "api.example.com", 443),
        ("https", "API.EXAMPLE.COM", 443),
        ("https", "x.partner.example", 8443),
        ("https", "a.b.partner.example", 443),
        ("https", "203.0.113.9", 443),
        ("https", "10.1.2.3", 443),
    ] {
        assert_eq!(
            verdict(&w, scheme, host, port).await,
            Ok(()),
            "{scheme}://{host}:{port}"
        );
    }
    for (scheme, host, port, why) in [
        ("http", "api.example.com", 443, "plain http is not allowed"),
        ("https", "api.example.com", 80, "a port that is not listed"),
        (
            "https",
            "other.example.com",
            443,
            "a host that is not listed",
        ),
        (
            "https",
            "partner.example",
            443,
            "the domain itself is not below it",
        ),
        ("https", "evilpartner.example", 443, "a look-alike suffix"),
        (
            "https",
            "10.2.0.1",
            443,
            "private space outside every network rule",
        ),
        (
            "https",
            "203.0.114.1",
            443,
            "public space outside every network rule",
        ),
        ("ftp", "api.example.com", 21, "an unsupported scheme"),
    ] {
        assert!(verdict(&w, scheme, host, port).await.is_err(), "{why}");
    }
}

#[tokio::test]
async fn no_policy_can_open_the_addresses_that_are_never_reachable() {
    // The worst policy an administrator could write: every dangerous network, named outright.
    let w = world().await;
    set(
        &w,
        &[
            "127.0.0.0/8",
            "169.254.0.0/16",
            "0.0.0.0/0",
            "::/0",
            "::1/128",
            "fe80::/10",
            "224.0.0.0/4",
            "localhost",
            "*.localhost",
            "metadata.google.internal",
            "100.100.100.200/32",
            "fd00:ec2::254/128",
        ],
        &[],
        true,
    )
    .await;
    for (host, class) in [
        ("127.0.0.1", AddressClass::Loopback),
        ("127.255.255.254", AddressClass::Loopback),
        ("[::1]", AddressClass::Loopback),
        ("169.254.169.254", AddressClass::LinkLocal),
        ("100.100.100.200", AddressClass::Metadata),
        ("[fd00:ec2::254]", AddressClass::Metadata),
        ("169.254.1.1", AddressClass::LinkLocal),
        ("[fe80::1]", AddressClass::LinkLocal),
        ("224.0.0.1", AddressClass::Multicast),
        ("0.0.0.0", AddressClass::Unspecified),
        // An IPv4 address carried inside an IPv6 one is judged as the IPv4 address.
        ("[::ffff:127.0.0.1]", AddressClass::Loopback),
        ("[::ffff:169.254.169.254]", AddressClass::LinkLocal),
        ("[64:ff9b::7f00:1]", AddressClass::Loopback),
        ("[2002:7f00:1::1]", AddressClass::Loopback),
    ] {
        assert_eq!(
            verdict(&w, "https", host, 443).await,
            Err(Denial::ForbiddenAddress(class)),
            "{host}"
        );
    }
    // Numbers in disguise are not host names, and so cannot be matched by a name rule either.
    for host in [
        "127.1",
        "0x7f000001",
        "2130706433",
        "0177.0.0.1",
        "0x7f.1",
        "1.1.1.256",
    ] {
        assert!(
            matches!(
                verdict(&w, "https", host, 443).await,
                Err(Denial::InvalidHost | Denial::ForbiddenAddress(_) | Denial::NotAllowlisted)
            ),
            "{host}"
        );
    }
}

#[tokio::test]
async fn the_policy_that_is_stored_is_the_policy_the_client_enforces() {
    // A loopback server is listening, and the policy names it as plainly as a policy can. The
    // guarded client built from the *stored* policy still refuses to connect.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let w = world().await;
    set(&w, &["127.0.0.1/32", "localhost"], &[port], true).await;
    let policy = w
        .egress
        .policy_for(w.admin.tenant_id(), ns(&w))
        .await
        .expect("policy");
    let client = EgressClient::new(policy, EgressLimits::default()).expect("client");
    let refused = client
        .send(EgressRequest {
            method: reqwest::Method::GET,
            url: format!("http://127.0.0.1:{port}/"),
            headers: Vec::new(),
            body: Vec::new(),
        })
        .await;
    assert!(
        matches!(
            refused,
            Err(EgressError::Denied(Denial::ForbiddenAddress(
                AddressClass::Loopback
            )))
        ),
        "{refused:?}"
    );
    // Nothing was accepted on the server: the connection was never attempted.
    let accepted =
        tokio::time::timeout(std::time::Duration::from_millis(200), listener.accept()).await;
    assert!(accepted.is_err(), "the guarded client connected");
}

// ------------------------------------------------------------- the audit trail

#[tokio::test]
async fn a_refused_request_is_audited_with_its_category_and_no_path_or_query() {
    let w = world().await;
    let metrics = sagat_telemetry::Metrics::global();
    let denied_before = denials_counted(&metrics.render());
    set(&w, &["api.example.com"], &[], false).await;
    assert!(verdict(&w, "https", "api.example.com", 443).await.is_ok());
    assert!(verdict(&w, "https", "169.254.169.254", 443).await.is_err());
    assert!(verdict(&w, "https", "evil.example.net", 443).await.is_err());
    // Asking again for the same destination does not grow the trail.
    assert!(verdict(&w, "https", "evil.example.net", 443).await.is_err());

    let tenant = w.admin.tenant_id();
    let mut rows = crate::support::eventually("two refusals are audited", || async {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT resource_id, reason FROM audit_records \
             WHERE tenant_id = $1 AND action = 'egress.denied' ORDER BY resource_id",
        )
        .bind(tenant.as_uuid())
        .fetch_all(&w.db.pool)
        .await
        .expect("audit");
        (rows.len() >= 2).then_some(rows)
    })
    .await;
    rows.sort();
    let namespace = ns(&w);
    assert_eq!(
        rows,
        vec![
            (
                format!("{namespace}/169.254.169.254:443"),
                "FORBIDDEN_ADDRESS".to_owned()
            ),
            (
                format!("{namespace}/evil.example.net:443"),
                "NOT_ALLOWLISTED".to_owned()
            ),
        ]
    );
    // An allowed request leaves nothing behind.
    assert!(rows.iter().all(|(_, reason)| reason != "OK"));
    // The counter sees every refusal, the repeated one included, where the trail keeps one row.
    // (Other tests of this binary count too, so "at least".)
    assert!(
        denials_counted(&metrics.render()) >= denied_before + 3.0,
        "{}",
        metrics.render()
    );
}

/// What `security_denials_total{action="egress.denied"}` has counted so far in this process.
fn denials_counted(page: &str) -> f64 {
    page.lines()
        .filter(|line| line.starts_with("security_denials_total{action=\"egress.denied\"}"))
        .filter_map(|line| line.rsplit(' ').next()?.parse::<f64>().ok())
        .sum()
}

#[tokio::test]
async fn changing_the_policy_is_audited_without_revealing_where_traffic_may_now_go() {
    let w = world().await;
    set(
        &w,
        &["10.20.30.0/24", "internal-api.corp.example"],
        &[8443],
        true,
    )
    .await;
    w.egress
        .reset(&w.admin, ns(&w), &ctx())
        .await
        .expect("reset");
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT reason FROM audit_records WHERE action = 'egress.policy_changed' ORDER BY created_at",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("audit");
    assert_eq!(
        rows,
        [
            "2 rules, 1 ports, http allowed",
            "policy removed: everything is denied"
        ]
    );
    for reason in rows {
        assert!(
            !reason.contains("10.20.30") && !reason.contains("corp.example"),
            "{reason}"
        );
    }
}

// ------------------------------------------------------------- roles and tenants

#[tokio::test]
async fn only_an_administrator_reads_or_changes_the_policy() {
    let w = world().await;
    for role in [Role::Developer, Role::Operator, Role::Viewer, Role::Worker] {
        let who = Principal::new(w.admin.tenant_id(), subject_of("someone"), role);
        let rules = strings(&["a.example.com"]);
        assert!(
            matches!(
                w.egress.get(&who, ns(&w), &ctx()).await,
                Err(AppError::Forbidden { .. })
            ),
            "{role}"
        );
        assert!(
            matches!(
                w.egress.set(&who, ns(&w), &rules, &[], false, &ctx()).await,
                Err(AppError::Forbidden { .. })
            ),
            "{role}"
        );
        assert!(
            matches!(
                w.egress.reset(&who, ns(&w), &ctx()).await,
                Err(AppError::Forbidden { .. })
            ),
            "{role}"
        );
    }
}

#[tokio::test]
async fn each_operation_audits_a_probe_of_a_foreign_namespace_on_its_own() {
    for operation in ["get", "set", "reset"] {
        let w = world().await;
        let here = ns(&w);
        let result = match operation {
            "get" => w.egress.get(&w.other_admin, here, &ctx()).await.map(|_| ()),
            "set" => w
                .egress
                .set(
                    &w.other_admin,
                    here,
                    &strings(&["a.example.com"]),
                    &[],
                    false,
                    &ctx(),
                )
                .await
                .map(|_| ()),
            _ => w.egress.reset(&w.other_admin, here, &ctx()).await,
        };
        assert_eq!(result, Err(AppError::NotFound), "{operation}");
        let tenant = w.other_admin.tenant_id();
        let audited = crate::support::eventually(operation, || async {
            let rows: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM audit_records WHERE tenant_id = $1 \
                 AND action = 'access.denied.cross_tenant' AND resource_type = 'namespace'",
            )
            .bind(tenant.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
            (rows > 0).then_some(rows)
        })
        .await;
        assert_eq!(audited, 1, "{operation}");
    }
}

#[tokio::test]
async fn one_tenants_policy_never_applies_to_another() {
    let w = world().await;
    set(&w, &["api.example.com"], &[], false).await;
    // The other tenant, in its own namespace of the same name, still has the default.
    let theirs = w
        .egress
        .check(
            w.other_admin.tenant_id(),
            w.other_namespace.id(),
            &Destination {
                scheme: "https",
                host: "api.example.com",
                port: 443,
            },
        )
        .await
        .expect("lookup");
    assert_eq!(theirs, Err(Denial::NotAllowlisted));
    // And naming this tenant's namespace as the other tenant finds no policy either.
    let crossed = w
        .egress
        .policy_for(w.other_admin.tenant_id(), ns(&w))
        .await
        .expect("policy");
    assert!(crossed.rules().is_empty());
}
