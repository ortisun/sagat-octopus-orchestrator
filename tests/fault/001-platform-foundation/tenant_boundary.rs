//! US04 under concurrency and infrastructure failure: whatever else goes wrong, one tenant must
//! never be shown another tenant's data.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use sagat_api_http::RateLimitConfig;

use crate::support::api::{Options, TestApi, json_of, start_with};

struct Tenant {
    slug: &'static str,
    admin: &'static str,
    workspace: (String, &'static str),
    namespace: (String, &'static str),
}

async fn three_tenants(api: &TestApi) -> Vec<Tenant> {
    let mut tenants = Vec::new();
    for (slug, admin, ws_slug, ns_slug) in [
        ("acme", "alice", "acme-platform", "acme-dev"),
        ("globex", "bob", "globex-platform", "globex-dev"),
        ("initech", "carol", "initech-platform", "initech-dev"),
    ] {
        api.tenant(slug, admin).await;
        let ws = api.create_workspace(admin, slug, ws_slug).await;
        let ns = api.create_namespace(admin, slug, &ws, ns_slug, "dev").await;
        tenants.push(Tenant {
            slug,
            admin,
            workspace: (ws, ws_slug),
            namespace: (ns, ns_slug),
        });
    }
    tenants
}

/// Tiny deterministic generator so a failing run can be reproduced.
struct Xorshift(u64);
impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).expect("fits")
    }
}

async fn flood(
    api: Arc<TestApi>,
    tenants: Arc<Vec<Tenant>>,
    requests: usize,
    seed: u64,
) -> HashMap<String, usize> {
    let mut rng = Xorshift(seed);
    let mut tasks = Vec::new();
    for _ in 0..requests {
        let (caller, target, namespace) =
            (rng.below(3), rng.below(3), rng.next().is_multiple_of(2));
        let (api, tenants) = (Arc::clone(&api), Arc::clone(&tenants));
        tasks.push(tokio::spawn(async move {
            let (who, owner) = (&tenants[caller], &tenants[target]);
            let (id, expected_slug, kind) = if namespace {
                (&owner.namespace.0, owner.namespace.1, "namespaces")
            } else {
                (&owner.workspace.0, owner.workspace.1, "workspaces")
            };
            let response = api
                .get_as(&format!("/{kind}/{id}"), who.admin, who.slug)
                .await;
            let status = response.status();
            let outcome = match (caller == target, status) {
                (true, StatusCode::OK) => {
                    let body = json_of(response).await;
                    assert_eq!(
                        body["slug"], expected_slug,
                        "own resource must be the right one"
                    );
                    "own-200"
                }
                (true, s) if s == StatusCode::SERVICE_UNAVAILABLE || s == StatusCode::NOT_FOUND => {
                    "own-degraded"
                }
                (false, StatusCode::NOT_FOUND) => "foreign-404",
                (false, StatusCode::SERVICE_UNAVAILABLE) => "foreign-503",
                (own, s) => panic!(
                    "TENANT BOUNDARY VIOLATION: caller {} asked {} (own: {own}) and got {s}",
                    who.slug, owner.slug
                ),
            };
            outcome.to_owned()
        }));
    }
    let mut counts = HashMap::new();
    for task in tasks {
        *counts.entry(task.await.expect("task")).or_insert(0) += 1;
    }
    counts
}

#[tokio::test]
async fn no_tenant_ever_sees_anothers_data_under_concurrency_and_a_database_failure() {
    let api = Arc::new(
        start_with(Options {
            rate_limit: RateLimitConfig {
                per_second: 100_000.0,
                burst: 100_000.0,
            },
            ..Options::default()
        })
        .await,
    );
    let tenants = Arc::new(three_tenants(&api).await);

    // Phase 1: 600 mixed own/foreign lookups while every database connection is repeatedly killed.
    let killer = {
        let pool = api.db.pool.clone();
        tokio::spawn(async move {
            for _ in 0..4 {
                tokio::time::sleep(Duration::from_millis(120)).await;
                let _ = sqlx::query(
                    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                     WHERE datname = current_database() AND pid <> pg_backend_pid()",
                )
                .execute(&pool)
                .await;
            }
        })
    };
    let chaos = flood(Arc::clone(&api), Arc::clone(&tenants), 600, 0x5eed_0001).await;
    killer.await.expect("killer");
    assert!(
        chaos.values().sum::<usize>() == 600,
        "every request was answered: {chaos:?}"
    );
    assert!(
        chaos.get("foreign-404").copied().unwrap_or(0)
            + chaos.get("foreign-503").copied().unwrap_or(0)
            > 0,
        "the flood really contained foreign lookups: {chaos:?}"
    );

    // Phase 2: calm again. Own resources are served, foreign ones are 404, deterministically.
    let calm = flood(Arc::clone(&api), Arc::clone(&tenants), 300, 0x5eed_0002).await;
    assert_eq!(calm.get("foreign-503"), None, "{calm:?}");
    assert_eq!(calm.get("own-degraded"), None, "{calm:?}");
    assert!(
        calm.get("own-200").copied().unwrap_or(0) > 0
            && calm.get("foreign-404").copied().unwrap_or(0) > 0
    );

    // The denial trail is bounded by the number of distinct (caller, foreign resource) pairs:
    // 3 callers x 2 foreign tenants x 2 resource kinds = 12, however many requests were made.
    crate::support::eventually("every distinct denial to be recorded", || async {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_records WHERE action = 'access.denied.cross_tenant'",
        )
        .fetch_one(&api.db.pool)
        .await
        .expect("count");
        (n >= 12).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action = 'access.denied.cross_tenant'",
    )
    .fetch_one(&api.db.pool)
    .await
    .expect("count");
    assert_eq!(n, 12, "900 requests, 12 distinct denials");
}
