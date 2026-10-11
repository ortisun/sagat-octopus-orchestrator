//! What the monitoring files promise, held to what the platform does: every metric a dashboard draws
//! or a rule evaluates is one the code emits, every rule and every alert points to a runbook section
//! that exists, and the files the compose overlay mounts are there (FR-008-003, FR-008-005).
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Every `.rs` file under `dir`, skipping tests and build output.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !matches!(
                name.as_str(),
                "target" | "tests" | "node_modules" | "migrations"
            ) {
                sources(&path, out);
            }
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

/// The names of the metrics the code records, with the suffixes a histogram is exposed under.
fn emitted() -> BTreeSet<String> {
    let mut files = Vec::new();
    for dir in ["crates", "apps"] {
        sources(&repo().join(dir), &mut files);
    }
    let call = Regex::new(r#"(counter|gauge|histogram)!\(\s*"([a-z][a-z0-9_]*)""#).expect("regex");
    let mut names = BTreeSet::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("source");
        for found in call.captures_iter(&text) {
            let name = found[2].to_owned();
            if &found[1] == "histogram" {
                for suffix in ["_bucket", "_sum", "_count"] {
                    names.insert(format!("{name}{suffix}"));
                }
            }
            names.insert(name);
        }
    }
    // `up` is the scraper's own.
    names.insert("up".to_owned());
    names
}

/// The metric names a PromQL expression refers to.
fn metrics_in(expr: &str) -> BTreeSet<String> {
    // Label matchers, range selectors and grouping clauses hold label names and durations, not metrics.
    let without_offsets = Regex::new(r"offset\s+\d+[smhdwy]")
        .expect("regex")
        .replace_all(expr, "");
    let without_ranges = Regex::new(r"\[[^\]]*\]")
        .expect("regex")
        .replace_all(&without_offsets, "");
    let without_matchers = Regex::new(r"\{[^}]*\}")
        .expect("regex")
        .replace_all(&without_ranges, "");
    let without_grouping =
        Regex::new(r"\b(by|without|on|ignoring|group_left|group_right)\s*\([^)]*\)")
            .expect("regex")
            .replace_all(&without_matchers, "");
    let word = Regex::new(r"[a-zA-Z_:][a-zA-Z0-9_:]*").expect("regex");
    let keywords: HashSet<&str> = ["and", "or", "unless", "offset", "bool", "le"]
        .into_iter()
        .collect();
    let mut out = BTreeSet::new();
    for found in word.find_iter(&without_grouping) {
        let name = found.as_str();
        let followed_by_call = without_grouping[found.end()..]
            .trim_start()
            .starts_with('(');
        if !followed_by_call
            && !keywords.contains(name)
            && !name.chars().all(|c| c.is_ascii_digit())
        {
            out.insert(name.to_owned());
        }
    }
    out
}

/// The anchors a Markdown file's headings make, the way GitHub makes them.
fn anchors(markdown: &str) -> HashSet<String> {
    markdown
        .lines()
        .filter_map(|line| line.strip_prefix('#'))
        .map(|heading| {
            heading
                .trim_start_matches('#')
                .trim()
                .to_ascii_lowercase()
                .chars()
                .filter_map(|c| match c {
                    c if c.is_ascii_alphanumeric() || c == '-' || c == '_' => Some(c),
                    ' ' => Some('-'),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

/// `docs/runbooks/alerts.md#outbox-stuck` is a section that exists.
fn runbook_exists(reference: &str) -> bool {
    let (path, anchor) = reference
        .split_once('#')
        .map_or((reference, None), |(p, a)| (p, Some(a)));
    let file = repo().join(path);
    let Ok(text) = std::fs::read_to_string(&file) else {
        return false;
    };
    anchor.is_none_or(|a| anchors(&text).contains(a))
}

fn dashboard_exprs() -> Vec<(String, String)> {
    let dir = repo().join("infra/observability/dashboards");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("dashboards").flatten() {
        let dashboard: Value =
            serde_json::from_str(&std::fs::read_to_string(entry.path()).expect("file"))
                .expect("a dashboard is JSON");
        for panel in dashboard["panels"].as_array().expect("panels") {
            for target in panel["targets"].as_array().into_iter().flatten() {
                out.push((
                    panel["title"].as_str().unwrap_or_default().to_owned(),
                    target["expr"].as_str().expect("an expression").to_owned(),
                ));
            }
        }
    }
    out
}

fn rules() -> Vec<Value> {
    let doc: Value =
        serde_yaml_ng::from_str(&read("infra/observability/alerts.yaml")).expect("rules are YAML");
    doc["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .flat_map(|g| g["rules"].as_array().expect("rules").clone())
        .collect()
}

#[test]
fn every_metric_a_dashboard_draws_is_one_the_platform_emits() {
    let emitted = emitted();
    let exprs = dashboard_exprs();
    assert!(
        exprs.len() >= 30,
        "the dashboard draws something: {}",
        exprs.len()
    );
    let mut missing = Vec::new();
    for (title, expr) in &exprs {
        for name in metrics_in(expr) {
            if !emitted.contains(&name) {
                missing.push(format!("{title}: {name}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "metrics the code does not emit:\n{}",
        missing.join("\n")
    );
}

#[test]
fn every_metric_an_alert_rule_evaluates_is_one_the_platform_emits() {
    let emitted = emitted();
    let mut missing = Vec::new();
    for rule in rules() {
        let expr = rule["expr"].as_str().expect("an expression");
        for name in metrics_in(expr) {
            if !emitted.contains(&name) {
                missing.push(format!("{}: {name}", rule["alert"].as_str().unwrap_or("?")));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "metrics the code does not emit:\n{}",
        missing.join("\n")
    );
}

#[test]
fn every_rule_has_a_severity_a_summary_and_a_runbook_section_that_exists() {
    let rules = rules();
    assert!(rules.len() >= 12);
    for rule in rules {
        let name = rule["alert"].as_str().expect("a name");
        assert!(
            matches!(
                rule["labels"]["severity"].as_str(),
                Some("info" | "warning" | "critical")
            ),
            "{name}"
        );
        assert!(
            rule["annotations"]["summary"]
                .as_str()
                .is_some_and(|s| s.len() > 10),
            "{name}"
        );
        let runbook = rule["annotations"]["runbook"]
            .as_str()
            .unwrap_or_else(|| panic!("{name} has no runbook"));
        assert!(
            runbook_exists(runbook),
            "{name}: {runbook} is not a section that exists"
        );
    }
}

#[test]
fn every_alert_the_monitor_can_raise_points_to_a_section_that_exists() {
    for kind in sagat_domain::operations::AlertKind::ALL {
        let reference = format!("docs/runbooks/alerts.md#{}", kind.runbook_anchor());
        assert!(runbook_exists(&reference), "{kind}: {reference}");
    }
}

#[test]
fn the_monitor_raises_an_alert_for_each_condition_the_runbook_describes() {
    let runbook = read("docs/runbooks/alerts.md");
    for kind in sagat_domain::operations::AlertKind::ALL {
        assert!(
            runbook.contains(&format!("`{}`", kind.as_str())),
            "{kind} is not described"
        );
    }
}

#[test]
fn the_files_the_overlay_mounts_and_the_dashboards_it_provisions_exist() {
    let overlay = read("infra/docker/compose.observability.yaml");
    for line in overlay.lines().map(str::trim) {
        if let Some(mounted) = line.strip_prefix("- ../observability/") {
            let source = mounted.split(':').next().expect("a source");
            assert!(
                repo().join("infra/observability").join(source).exists(),
                "{source} is mounted and missing"
            );
        }
    }
    let provisioned: Value = serde_yaml_ng::from_str(&read(
        "infra/observability/grafana/provisioning/dashboards/sagat.yaml",
    ))
    .expect("yaml");
    assert_eq!(
        provisioned["providers"][0]["options"]["path"],
        "/var/lib/grafana/dashboards"
    );
}

#[test]
fn the_dashboard_covers_what_the_specification_names() {
    let titles: String = dashboard_exprs()
        .into_iter()
        .map(|(t, e)| format!("{t} {e}"))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    // FR-008-003: latency, queue lag, leases, DLQ, retry, compensation and quotas.
    for (what, needle) in [
        ("latency", "step_schedule_latency_seconds"),
        ("queue lag", "queue_lag_seconds"),
        ("leases", "leases_expired"),
        ("dead items", "dead_items"),
        ("retry", "retry"),
        ("compensation", "compensation_attempts_total"),
        ("quotas", "quota_exceeded_total"),
    ] {
        assert!(
            titles.contains(needle),
            "the dashboard does not draw {what} ({needle})"
        );
    }
}

#[test]
fn a_dashboard_has_a_unique_uid_and_no_panel_without_an_expression() {
    let dir = repo().join("infra/observability/dashboards");
    let mut uids = HashSet::new();
    for entry in std::fs::read_dir(&dir).expect("dashboards").flatten() {
        let dashboard: Value =
            serde_json::from_str(&std::fs::read_to_string(entry.path()).expect("file"))
                .expect("json");
        assert!(uids.insert(dashboard["uid"].as_str().expect("uid").to_owned()));
        for panel in dashboard["panels"].as_array().expect("panels") {
            if panel["type"] != "row" {
                assert!(
                    panel["targets"].as_array().is_some_and(|t| !t.is_empty()),
                    "{} has no query",
                    panel["title"]
                );
            }
        }
    }
}
