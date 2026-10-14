//! Bounded measurement of the ADR-0027 policy/bucket/usage locks, not a production capacity promise.
use crate::support::engine::world;
use futures_util::{StreamExt, stream};
use sagat_application::quotas::{NamespaceApiAdmission, RateDecision};
use sagat_domain::quota::QuotaResource;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::Instant;

#[tokio::test]
#[ignore = "explicit benchmark; writes report only when SAGAT_QUOTA_PERF_REPORT is set"]
async fn durable_quota_hot_tenant_and_two_tenant_measurement() {
    let w = world().await;
    let scopes = [
        (w.admin.tenant_id(), w.namespace.id()),
        (w.other_admin.tenant_id(), w.other_namespace.id()),
    ];
    for (principal, namespace) in [
        (&w.admin, w.namespace.id()),
        (&w.other_admin, w.other_namespace.id()),
    ] {
        let tenant = principal.tenant_id();
        w.quotas
            .grant_ceiling(tenant, QuotaResource::ApiRequestsPerSecond, Some(1_000_000))
            .await
            .expect("benchmark ceiling");
        w.quotas
            .set(
                principal,
                Some(namespace),
                QuotaResource::ApiRequestsPerSecond,
                1_000_000,
                &crate::support::engine::ctx(),
            )
            .await
            .expect("benchmark rate");
        assert_eq!(
            w.store
                .admit_namespace_request(tenant, namespace)
                .await
                .expect("prime"),
            RateDecision::Admitted
        );
    }
    sqlx::query("UPDATE quota_accounting_control SET content_enforcement_enabled=true,log_enforcement_enabled=true")
        .execute(&w.db.pool).await.expect("enforce accounting");
    let pg: String = sqlx::query_scalar("SELECT version()")
        .fetch_one(&w.db.pool)
        .await
        .expect("version");
    let mut report = format!(
        "# Durable quota lock measurement\n\n{}\n- Database: {pg}\n- Container: Colima sagat-remaining, 4 vCPU / 6 GiB; shared loaded host\n- 250 operations per scenario, eight concurrent callers, two rounds; 512-byte unique payloads\n- API rate configured to 1,000,000/s for measurement; content/log enforcement enabled\n\n| Scenario | Samples | p50 | p95 | p99 | Max |\n|---|---:|---:|---:|---:|---:|\n",
        super::environment().replace(
            "- PostgreSQL 16 in a Docker container on the same machine\n",
            ""
        )
    );
    for round in 0..2 {
        for content in [false, true] {
            for tenants in [1, 2] {
                let before: i64 = sqlx::query_scalar(
                    "SELECT sum(content_bytes)::bigint FROM tenant_retained_usage",
                )
                .fetch_one(&w.db.pool)
                .await
                .expect("before");
                let started = Instant::now();
                let samples = stream::iter(0..250).map(|i| {
                    let store=w.store.clone(); let pool=w.db.pool.clone(); let (tenant,namespace)=scopes[i%tenants];
                    async move {
                        let start=Instant::now();
                        if content {
                            let mut payload=vec![b'x';512];
                            let key=format!("round:{round}:tenants:{tenants}:item:{i}");
                            payload[..key.len()].copy_from_slice(key.as_bytes());
                            let digest=Sha256::digest(&payload).to_vec();
                            sqlx::query("INSERT INTO payloads(tenant_id,digest,content) VALUES($1,$2,$3)")
                                .bind(tenant.as_uuid()).bind(digest).bind(payload).execute(&pool).await.expect("content commit");
                        } else {
                            assert_eq!(store.admit_namespace_request(tenant,namespace).await.expect("admission"),RateDecision::Admitted);
                        }
                        start.elapsed()
                    }
                }).buffer_unordered(8).collect::<Vec<_>>().await;
                let elapsed = started.elapsed();
                let label = format!(
                    "round {} / {} / {} tenant(s)",
                    round + 1,
                    if content {
                        "content commit"
                    } else {
                        "API admission"
                    },
                    tenants
                );
                let (row, _, _) = super::row(&label, samples);
                report.push_str(&row);
                let after: i64 = sqlx::query_scalar(
                    "SELECT sum(content_bytes)::bigint FROM tenant_retained_usage",
                )
                .fetch_one(&w.db.pool)
                .await
                .expect("after");
                assert_eq!(
                    after - before,
                    if content { 250 * 512 } else { 0 },
                    "exact accounting: {label}"
                );
                println!(
                    "{label}: {:.1} ops/s, exact usage confirmed",
                    250.0 / elapsed.as_secs_f64()
                );
            }
        }
    }
    report.push_str("\nAll 2,000 operations committed; each content batch increased exact retained usage by 128,000 bytes, and API-only batches changed no content bytes. These measurements include database round trips, deferred trigger accounting and pool contention, using direct adapter calls. They do not measure full HTTP latency, cloud networking, multi-node HA or a commercial tier. The shared host was running other validation work; compare repeated controlled runs before sizing a deployment. No unmeasured throughput floor is claimed.\n\nReproduce: `SAGAT_QUOTA_PERF_REPORT=tests/performance/reports/durable-quotas.md cargo test -p sagat-tests --test perf_security_multitenancy durable_quota_hot -- --ignored --nocapture`. Use the documented Docker socket when running in this environment.\n");
    if let Ok(path) = std::env::var("SAGAT_QUOTA_PERF_REPORT") {
        let configured = PathBuf::from(path);
        let path = if configured.is_absolute() {
            configured
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("tests package has a workspace root")
                .join(configured)
        };
        std::fs::write(path, &report).expect("report");
    }
    println!("{report}");
}
