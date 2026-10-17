//! The runtime role must hold a privilege for every statement the stores can issue (G294, T014-007).
//!
//! Every other integration test connects as the database OWNER, which can do anything, so a
//! statement the least-privilege role `sagat_app` may not run passes the whole suite and answers
//! `permission denied` in production. This test closes the class:
//!
//! 1. it reads the SQL literals out of the Rust sources that talk to PostgreSQL (`rust_source`),
//! 2. derives from each statement what it asks of the role (`sql_needs`): SELECT, INSERT columns,
//!    UPDATE columns, DELETE, row locks (`FOR UPDATE|NO KEY UPDATE|SHARE|KEY SHARE`), function calls,
//! 3. asks the catalog of the fully migrated schema whether `sagat_app` holds each privilege, and
//!    cross-checks the answer by probing the server AS the runtime role (a probe that is refused
//!    answers `42501`),
//! 4. follows the triggers those writes fire (a trigger function that is not `SECURITY DEFINER` runs
//!    with the caller's privileges) and the functions CHECK constraints and defaults call.
//!
//! A gap that is not in [`ALLOWED`], with a reason, fails the test; so does an `ALLOWED` entry that no
//! longer excuses anything. What the extractor cannot see is listed in the module documentation of
//! `rust_source` and in `docs/runbooks/control-plane.md`.
#![allow(clippy::panic)]

#[path = "runtime_privileges/rust_source.rs"]
mod rust_source;
#[path = "runtime_privileges/sql_needs.rs"]
mod sql_needs;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sqlx::PgPool;

use crate::support::{RUNTIME_ROLE, TestDb, migrated_db};
use sql_needs::Need;

/// A statement or trigger the runtime role is NOT expected to run, with the reason. Matching is on
/// the exact repository-relative file (or `trigger ...` label) and the exact need text.
struct Allowed {
    file: &'static str,
    need: &'static str,
    reason: &'static str,
}

const IDENTITY: &str = "crates/infra-postgres/src/identity.rs";
const QUOTAS: &str = "crates/infra-postgres/src/quotas.rs";
const CONTENT_USAGE: &str = "crates/infra-postgres/src/content_usage.rs";

const ALLOWED: &[Allowed] = &[
    Allowed {
        file: IDENTITY,
        need: "INSERT tenants(id, slug, created_at)",
        reason: "provision-tenant runs as the owner role, never as the runtime role (control-plane runbook, section 1)",
    },
    Allowed {
        file: IDENTITY,
        need: "INSERT memberships(tenant_id, subject, role, created_at)",
        reason: "the first administrator is written by provision-tenant as the owner role; the runtime role may only read memberships",
    },
    Allowed {
        file: QUOTAS,
        need: "INSERT tenant_scheduling(tenant_id, weight, updated_at)",
        reason: "G260: `control-plane scheduling-weight` is an operator command run with the owner role; the engine only reads the weights",
    },
    Allowed {
        file: QUOTAS,
        need: "UPDATE tenant_scheduling.weight",
        reason: "G260: operator command, owner role (the upsert of the same statement)",
    },
    Allowed {
        file: QUOTAS,
        need: "UPDATE tenant_scheduling.updated_at",
        reason: "G260: operator command, owner role (the upsert of the same statement)",
    },
    Allowed {
        file: QUOTAS,
        need: "DELETE tenant_scheduling",
        reason: "G260: operator command, owner role (withdrawing a weight)",
    },
    Allowed {
        file: CONTENT_USAGE,
        need: "INSERT tenant_retained_usage(tenant_id, content_bytes, content_reserved_bytes, log_bytes)",
        reason: "apply_content_delta is #[allow(dead_code)] and nothing calls it; the migration deliberately gives the runtime role no DML on the counter table (the accounting triggers are SECURITY DEFINER)",
    },
    Allowed {
        file: CONTENT_USAGE,
        need: "ROW LOCK tenant_retained_usage",
        reason: "apply_content_delta, unused (see the INSERT entry)",
    },
    Allowed {
        file: CONTENT_USAGE,
        need: "UPDATE tenant_retained_usage.content_bytes",
        reason: "apply_content_delta, unused (see the INSERT entry)",
    },
    Allowed {
        file: CONTENT_USAGE,
        need: "UPDATE tenant_retained_usage.content_reserved_bytes",
        reason: "apply_content_delta, unused (see the INSERT entry)",
    },
    Allowed {
        file: "crates/infra-postgres/src/telemetry.rs",
        need: "UNRESOLVED placeholder {condition}",
        reason: "a predicate chosen by the caller among literals of the same file; the statement is a plain SELECT of run_events, which is audited as written",
    },
    Allowed {
        file: "crates/infra-postgres/src/operations.rs",
        need: "UNRESOLVED placeholder {groups}",
        reason: "the sub-select text comes from the AlertKind match arms right above, each a literal statement that is audited on its own; the wrapper only reads",
    },
];

#[derive(Debug, Clone)]
struct Site {
    file: String,
    line: usize,
    excerpt: String,
}

impl Site {
    fn label(&self) -> String {
        if self.line == 0 {
            self.file.clone()
        } else {
            format!("{}:{}", self.file, self.line)
        }
    }
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests crate is in the repository")
        .to_path_buf()
}

/// Every non-test `src` directory in the workspace: whatever can talk to PostgreSQL is audited, not
/// only the adapter crate.
fn source_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, in_src: bool, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if matches!(name.as_str(), "target" | "node_modules" | ".git" | "tests") {
                    continue;
                }
                walk(&path, in_src || name == "src", out);
            } else if in_src && path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let root = repository();
    let mut files = Vec::new();
    for top in ["crates", "apps", "runners"] {
        walk(&root.join(top), false, &mut files);
    }
    files.sort();
    files
}

#[derive(Default)]
struct Extracted {
    statements: usize,
    needs: BTreeMap<Need, Vec<Site>>,
    /// A lock clause or DML verb in a literal that is not part of any statement: nothing audits it.
    fragments: Vec<Site>,
    /// Literals carrying SQL that opened no privilege requirement at all (kept for the report).
    statement_files: BTreeSet<String>,
}

fn extract_all() -> Extracted {
    let root = repository();
    let mut all = Extracted::default();
    for path in source_files() {
        let source = std::fs::read_to_string(&path).expect("read source");
        let relative = path
            .strip_prefix(&root)
            .expect("under the repository")
            .to_string_lossy()
            .into_owned();
        let found = rust_source::extract(&source);
        for other in found.unmodelled {
            let site = Site {
                file: relative.clone(),
                line: other.line,
                excerpt: excerpt(&other.text),
            };
            all.needs
                .entry(Need::Unresolved(format!(
                    "statement kind the audit does not model: {}",
                    other.text.split_whitespace().next().unwrap_or_default()
                )))
                .or_default()
                .push(site);
        }
        for fragment in found.fragments {
            all.fragments.push(Site {
                file: relative.clone(),
                line: fragment.line,
                excerpt: excerpt(&fragment.text),
            });
        }
        for statement in found.statements {
            all.statements += 1;
            all.statement_files.insert(relative.clone());
            let site = Site {
                file: relative.clone(),
                line: statement.line,
                excerpt: excerpt(&statement.text),
            };
            for need in sql_needs::needs(&statement.text) {
                all.needs.entry(need).or_default().push(site.clone());
            }
        }
    }
    all
}

fn excerpt(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(90).collect()
}

/// Identifiers come from the repository's own sources; still, nothing but plain names reaches SQL.
fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
}

/// The catalog's answer: `Ok(())` when the role holds the privilege, `Err(why)` otherwise.
async fn catalog(owner: &PgPool, need: &Need) -> Result<(), String> {
    async fn relation_exists(owner: &PgPool, table: &str) -> bool {
        sqlx::query_scalar::<_, bool>("SELECT to_regclass($1) IS NOT NULL")
            .bind(table)
            .fetch_one(owner)
            .await
            .expect("to_regclass")
    }
    async fn table_privilege(owner: &PgPool, table: &str, privilege: &str) -> bool {
        sqlx::query_scalar::<_, bool>("SELECT has_table_privilege($1, to_regclass($2), $3)")
            .bind(RUNTIME_ROLE)
            .bind(table)
            .bind(privilege)
            .fetch_one(owner)
            .await
            .expect("has_table_privilege")
    }
    async fn column_privilege(
        owner: &PgPool,
        table: &str,
        column: &str,
        privilege: &str,
    ) -> Result<bool, String> {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = to_regclass($1) \
             AND attname = $2 AND NOT attisdropped AND attnum > 0)",
        )
        .bind(table)
        .bind(column)
        .fetch_one(owner)
        .await
        .expect("pg_attribute");
        if !exists {
            return Err(format!("column {table}.{column} does not exist"));
        }
        Ok(sqlx::query_scalar::<_, bool>(
            "SELECT has_column_privilege($1, to_regclass($2), $3, $4)",
        )
        .bind(RUNTIME_ROLE)
        .bind(table)
        .bind(column)
        .bind(privilege)
        .fetch_one(owner)
        .await
        .expect("has_column_privilege"))
    }

    let relation = match need {
        Need::Select(t) | Need::Delete(t) | Need::RowLock(t) | Need::Truncate(t) => Some(t),
        Need::Insert(t, _) | Need::Update(t, _) => Some(t),
        Need::Execute(_) | Need::Unresolved(_) => None,
    };
    if let Some(table) = relation
        && !relation_exists(owner, table).await
    {
        return Err(format!(
            "relation {table} does not exist in the migrated schema"
        ));
    }
    let denied = |what: &str| Err(format!("{RUNTIME_ROLE} lacks {what}"));
    match need {
        Need::Select(t) => {
            if table_privilege(owner, t, "SELECT").await {
                Ok(())
            } else {
                denied(&format!("SELECT on {t}"))
            }
        }
        Need::Delete(t) => {
            if table_privilege(owner, t, "DELETE").await {
                Ok(())
            } else {
                denied(&format!("DELETE on {t}"))
            }
        }
        Need::Truncate(t) => {
            if table_privilege(owner, t, "TRUNCATE").await {
                Ok(())
            } else {
                denied(&format!("TRUNCATE on {t}"))
            }
        }
        Need::Insert(t, columns) if columns.is_empty() => {
            if table_privilege(owner, t, "INSERT").await {
                Ok(())
            } else {
                denied(&format!("INSERT on {t}"))
            }
        }
        Need::Insert(t, columns) => {
            for column in columns {
                if !column_privilege(owner, t, column, "INSERT").await? {
                    return denied(&format!("INSERT on {t}.{column}"));
                }
            }
            Ok(())
        }
        Need::Update(t, column) => {
            if column_privilege(owner, t, column, "UPDATE").await? {
                Ok(())
            } else {
                denied(&format!("UPDATE on {t}.{column}"))
            }
        }
        Need::RowLock(t) => {
            // PostgreSQL takes a row lock to be an UPDATE: it needs UPDATE on at least one column.
            let any = sqlx::query_scalar::<_, bool>(
                "SELECT has_any_column_privilege($1, to_regclass($2), 'UPDATE')",
            )
            .bind(RUNTIME_ROLE)
            .bind(t)
            .fetch_one(owner)
            .await
            .expect("has_any_column_privilege");
            if any {
                Ok(())
            } else {
                denied(&format!(
                    "UPDATE (any column), which FOR UPDATE/SHARE needs, on {t}"
                ))
            }
        }
        Need::Execute(name) => {
            let overloads: Vec<(String, bool)> = sqlx::query_as(
                "SELECT p.oid::regprocedure::text, has_function_privilege($1, p.oid, 'EXECUTE') \
                 FROM pg_proc p WHERE p.proname = $2 \
                 AND p.pronamespace IN ('public'::regnamespace, 'pg_catalog'::regnamespace)",
            )
            .bind(RUNTIME_ROLE)
            .bind(name)
            .fetch_all(owner)
            .await
            .expect("pg_proc");
            if overloads.is_empty() {
                return Err(format!("no function named {name} in public or pg_catalog"));
            }
            match overloads.iter().find(|(_, allowed)| !allowed) {
                Some((signature, _)) => denied(&format!("EXECUTE on {signature}")),
                None => Ok(()),
            }
        }
        Need::Unresolved(what) => Err(format!("not audited: {what}")),
    }
}

/// The server's answer: runs the smallest statement that needs just this privilege AS the runtime
/// role inside a transaction that is rolled back. A refusal is SQLSTATE 42501; any other outcome
/// (success, a not-null violation of the probe row, ...) means the privilege check passed.
/// `None` when no probe exists for the need.
async fn probe(runtime: &PgPool, need: &Need) -> Option<bool> {
    let statement = match need {
        Need::Select(t) if identifier(t) => format!("SELECT 1 FROM {t} WHERE false"),
        Need::Delete(t) if identifier(t) => format!("DELETE FROM {t} WHERE false"),
        Need::RowLock(t) if identifier(t) => {
            format!("SELECT 1 FROM {t} WHERE false FOR NO KEY UPDATE")
        }
        Need::Update(t, c) if identifier(t) && identifier(c) => {
            format!("UPDATE {t} SET {c} = DEFAULT WHERE false")
        }
        Need::Insert(t, columns) if identifier(t) && columns.iter().all(|c| identifier(c)) => {
            if columns.is_empty() {
                format!("INSERT INTO {t} DEFAULT VALUES")
            } else {
                let defaults = vec!["DEFAULT"; columns.len()].join(", ");
                format!(
                    "INSERT INTO {t} ({}) VALUES ({defaults})",
                    columns.join(", ")
                )
            }
        }
        _ => return None,
    };
    let mut tx = runtime.begin().await.expect("begin probe");
    let outcome = sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(&mut *tx)
        .await;
    let _ = tx.rollback().await;
    Some(match outcome {
        Err(sqlx::Error::Database(error)) => error.code().as_deref() != Some("42501"),
        _ => true,
    })
}

#[derive(Debug)]
struct Gap {
    site: Site,
    need: String,
    why: String,
}

fn excused(gap: &Gap) -> Option<usize> {
    ALLOWED
        .iter()
        .position(|a| a.file == gap.site.file && a.need == gap.need)
}

/// The tables a trigger can fire on, by statement kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Event {
    Insert,
    Update,
    Delete,
}

impl Event {
    fn bit(self) -> i16 {
        match self {
            Self::Insert => 4,
            Self::Delete => 8,
            Self::Update => 16,
        }
    }
}

fn events_of(needs: impl Iterator<Item = Need>) -> BTreeSet<(String, Event)> {
    needs
        .filter_map(|need| match need {
            Need::Insert(t, _) => Some((t, Event::Insert)),
            Need::Update(t, _) => Some((t, Event::Update)),
            Need::Delete(t) => Some((t, Event::Delete)),
            _ => None,
        })
        .collect()
}

/// The needs of the triggers that fire when the runtime role writes the given tables. A trigger
/// function that is `SECURITY DEFINER` runs as its owner and is not followed; every other one runs
/// as the caller, so what its body does is held to the same rule. Triggers the body fires are
/// followed in turn.
async fn trigger_needs(
    owner: &PgPool,
    written: BTreeSet<(String, Event)>,
) -> (Vec<(Need, Site)>, Vec<String>) {
    let mut pending: Vec<(String, Event)> = written.iter().cloned().collect();
    let mut seen = written;
    let mut found = Vec::new();
    let mut definers = Vec::new();
    while let Some((table, event)) = pending.pop() {
        let triggers: Vec<(String, String, bool, String, i16)> = sqlx::query_as(
            "SELECT t.tgname, p.proname, p.prosecdef, p.prosrc, t.tgtype \
             FROM pg_trigger t JOIN pg_proc p ON p.oid = t.tgfoid \
             WHERE t.tgrelid = to_regclass($1) AND NOT t.tgisinternal AND t.tgenabled <> 'D'",
        )
        .bind(&table)
        .fetch_all(owner)
        .await
        .expect("pg_trigger");
        for (name, function, definer, body, kind) in triggers {
            if kind & event.bit() == 0 {
                continue;
            }
            if definer {
                definers.push(format!("{table}.{name} -> {function}()"));
                continue;
            }
            let site = Site {
                file: format!("trigger {name} -> {function}() on {table}"),
                line: 0,
                excerpt: excerpt(&body),
            };
            for need in sql_needs::needs(&body) {
                for follow in events_of(std::iter::once(need.clone())) {
                    if seen.insert(follow.clone()) {
                        pending.push(follow);
                    }
                }
                found.push((need, site.clone()));
            }
        }
    }
    definers.sort();
    definers.dedup();
    (found, definers)
}

/// The functions CHECK constraints and column defaults of the written tables call: PostgreSQL asks
/// for EXECUTE on them when the row is built, as the caller.
async fn expression_function_gaps(owner: &PgPool, tables: &BTreeSet<String>) -> Vec<Gap> {
    let mut gaps = Vec::new();
    for table in tables {
        let rows: Vec<(String, bool, String)> = sqlx::query_as(
            "SELECT DISTINCT p.oid::regprocedure::text, has_function_privilege($1, p.oid, 'EXECUTE'), \
                    CASE WHEN c.oid IS NOT NULL THEN 'CHECK ' || c.conname ELSE 'DEFAULT of a column' END \
             FROM pg_depend d \
             JOIN pg_proc p ON d.refclassid = 'pg_proc'::regclass AND d.refobjid = p.oid \
             LEFT JOIN pg_constraint c ON d.classid = 'pg_constraint'::regclass AND c.oid = d.objid \
             LEFT JOIN pg_attrdef a ON d.classid = 'pg_attrdef'::regclass AND a.oid = d.objid \
             WHERE c.conrelid = to_regclass($2) OR a.adrelid = to_regclass($2)",
        )
        .bind(RUNTIME_ROLE)
        .bind(table)
        .fetch_all(owner)
        .await
        .expect("expression functions");
        for (signature, allowed, origin) in rows {
            if !allowed {
                gaps.push(Gap {
                    site: Site {
                        file: format!("{origin} on {table}"),
                        line: 0,
                        excerpt: signature.clone(),
                    },
                    need: format!("EXECUTE {signature}"),
                    why: format!("{RUNTIME_ROLE} lacks EXECUTE on {signature}"),
                });
            }
        }
    }
    gaps
}

/// Sequences that serve a column default of a written table (`serial`): an INSERT that omits the
/// column calls `nextval`, which needs USAGE. Identity columns do not.
async fn sequence_gaps(owner: &PgPool, tables: &BTreeSet<String>) -> Vec<Gap> {
    let mut gaps = Vec::new();
    for table in tables {
        let rows: Vec<(String, bool)> = sqlx::query_as(
            "SELECT s.relname::text, has_sequence_privilege($1, s.oid, 'USAGE') \
             FROM pg_class s \
             JOIN pg_depend d ON d.objid = s.oid AND d.classid = 'pg_class'::regclass AND d.deptype = 'a' \
             WHERE s.relkind = 'S' AND d.refobjid = to_regclass($2)",
        )
        .bind(RUNTIME_ROLE)
        .bind(table)
        .fetch_all(owner)
        .await
        .expect("sequences");
        for (sequence, allowed) in rows {
            if !allowed {
                gaps.push(Gap {
                    site: Site {
                        file: format!("default of a column of {table}"),
                        line: 0,
                        excerpt: sequence.clone(),
                    },
                    need: format!("USAGE {sequence}"),
                    why: format!("{RUNTIME_ROLE} lacks USAGE on sequence {sequence}"),
                });
            }
        }
    }
    gaps
}

struct Audit {
    gaps: Vec<Gap>,
    /// Needs where the catalog and the live probe disagreed.
    disagreements: Vec<String>,
    checked: usize,
    probed: usize,
    definer_triggers: Vec<String>,
    trigger_statements: usize,
}

/// Checks every need of `extracted` (and of the triggers and expressions its writes reach) against
/// the migrated database.
async fn audit(db: &TestDb, extracted: &Extracted) -> Audit {
    let runtime = db.runtime_pool().await;
    let mut result = Audit {
        gaps: Vec::new(),
        disagreements: Vec::new(),
        checked: 0,
        probed: 0,
        definer_triggers: Vec::new(),
        trigger_statements: 0,
    };

    let mut needs: Vec<(Need, Site)> = Vec::new();
    for (need, sites) in &extracted.needs {
        for site in sites {
            needs.push((need.clone(), site.clone()));
        }
    }
    let written = events_of(extracted.needs.keys().cloned());
    let (from_triggers, definers) = trigger_needs(&db.pool, written.clone()).await;
    result.definer_triggers = definers;
    result.trigger_statements = from_triggers.len();
    let mut written_tables: BTreeSet<String> = written.into_iter().map(|(t, _)| t).collect();
    for (need, site) in from_triggers {
        if let Need::Insert(t, _) | Need::Update(t, _) = &need {
            written_tables.insert(t.clone());
        }
        needs.push((need, site));
    }

    let sequence_gaps = sequence_gaps(&db.pool, &written_tables).await;
    // An INSERT probe also calls nextval for a `serial` key: a refusal on that account is a sequence
    // gap, reported below, and not a disagreement about the table privilege.
    let sequence_tables: BTreeSet<String> = sequence_gaps
        .iter()
        .filter_map(|gap| gap.site.file.strip_prefix("default of a column of "))
        .map(str::to_owned)
        .collect();

    let mut verdicts: BTreeMap<Need, Result<(), String>> = BTreeMap::new();
    for (need, _) in &needs {
        if verdicts.contains_key(need) {
            continue;
        }
        let verdict = catalog(&db.pool, need).await;
        let explained = matches!(need, Need::Insert(t, _) if sequence_tables.contains(t));
        if let (Some(live), true) = (
            probe(&runtime, need).await,
            !matches!(need, Need::Unresolved(_)),
        ) {
            result.probed += 1;
            if let Err(why) = &verdict {
                if !why.contains("does not exist") && live {
                    result.disagreements.push(format!(
                        "{need}: catalog says denied ({why}), server allowed it"
                    ));
                }
            } else if !live && !explained {
                result
                    .disagreements
                    .push(format!("{need}: catalog says granted, server refused it"));
            }
        }
        result.checked += 1;
        verdicts.insert(need.clone(), verdict);
    }
    for (need, site) in &needs {
        if let Some(Err(why)) = verdicts.get(need) {
            result.gaps.push(Gap {
                site: site.clone(),
                need: need.to_string(),
                why: why.clone(),
            });
        }
    }
    result
        .gaps
        .extend(expression_function_gaps(&db.pool, &written_tables).await);
    result.gaps.extend(sequence_gaps);
    result
}

fn describe(gaps: &[&Gap]) -> String {
    let mut text = String::new();
    for gap in gaps {
        let _ = writeln!(
            text,
            "  {}\n      needs {}: {}\n      in `{}`",
            gap.site.label(),
            gap.need,
            gap.why,
            gap.site.excerpt
        );
    }
    text
}

#[tokio::test]
async fn every_statement_the_stores_issue_is_within_the_runtime_roles_privileges() {
    let db = migrated_db().await;
    let extracted = extract_all();
    let audit = audit(&db, &extracted).await;
    eprintln!(
        "runtime privilege audit: {} statements in {} files, {} distinct needs checked, {} probed as {RUNTIME_ROLE}, \
         {} statements read from non-definer triggers, {} SECURITY DEFINER trigger(s) not followed",
        extracted.statements,
        extracted.statement_files.len(),
        audit.checked,
        audit.probed,
        audit.trigger_statements,
        audit.definer_triggers.len(),
    );

    assert!(
        audit.disagreements.is_empty(),
        "the catalog and the server disagree about what {RUNTIME_ROLE} may do, so the audit's model of \
         PostgreSQL is wrong:\n{}",
        audit.disagreements.join("\n")
    );

    let mut used = vec![false; ALLOWED.len()];
    let mut unexplained = Vec::new();
    for gap in &audit.gaps {
        match excused(gap) {
            Some(index) => used[index] = true,
            None => unexplained.push(gap),
        }
    }
    assert!(
        unexplained.is_empty(),
        "{} statement(s) need a privilege that {RUNTIME_ROLE} does not hold (fix the statement or the \
         grant, or add a reasoned entry to ALLOWED):\n{}",
        unexplained.len(),
        describe(&unexplained)
    );
    let stale: Vec<_> = ALLOWED
        .iter()
        .zip(&used)
        .filter(|(_, used)| !**used)
        .map(|(a, _)| format!("{} / {} ({})", a.file, a.need, a.reason))
        .collect();
    assert!(
        stale.is_empty(),
        "ALLOWED entries that excuse nothing any more: {stale:?}"
    );
    for allowed in ALLOWED {
        assert!(
            !allowed.reason.trim().is_empty(),
            "{} / {} has no reason",
            allowed.file,
            allowed.need
        );
    }
}

#[tokio::test]
async fn no_literal_outside_a_statement_carries_a_lock_clause_or_a_write() {
    // `... WHERE id = $1` plus a separately built ` FOR UPDATE` would escape the extractor; none exist,
    // and one appearing must be read by a person and added here with a reason.
    let extracted = extract_all();
    let stray: Vec<_> = extracted
        .fragments
        .iter()
        .map(|s| format!("{}: {}", s.label(), s.excerpt))
        .collect();
    assert!(
        stray.is_empty(),
        "SQL fragments outside any statement: {stray:#?}"
    );
}

#[tokio::test]
async fn the_extractor_finds_every_kind_of_statement_in_the_real_sources() {
    let extracted = extract_all();
    assert!(
        extracted.statements > 600,
        "only {} statements found",
        extracted.statements
    );
    let kinds: BTreeSet<&str> = extracted
        .needs
        .keys()
        .map(|need| match need {
            Need::Select(_) => "select",
            Need::Insert(..) => "insert",
            Need::Update(..) => "update",
            Need::Delete(_) => "delete",
            Need::RowLock(_) => "row lock",
            Need::Truncate(_) => "truncate",
            Need::Execute(_) => "function",
            Need::Unresolved(_) => "unresolved",
        })
        .collect();
    for kind in [
        "select", "insert", "update", "delete", "row lock", "function",
    ] {
        assert!(
            kinds.contains(kind),
            "no {kind} found: the extractor lost a statement shape"
        );
    }
    assert!(
        extracted
            .statement_files
            .iter()
            .any(|f| f == "crates/infra-postgres/src/primitives.rs"),
        "the primitives store is not scanned"
    );
}

/// The audit must fail on the defect that went to production: a lock on a table the role may only
/// read, and a write to a table it may only append to.
#[tokio::test]
async fn the_audit_flags_the_defects_it_exists_to_catch() {
    let db = migrated_db().await;
    let drifted = [
        (
            "SELECT role FROM memberships WHERE tenant_id = $1 AND subject = $2 FOR SHARE",
            "ROW LOCK memberships",
        ),
        (
            "UPDATE audit_records SET actor = 'x' WHERE id = $1",
            "UPDATE audit_records.actor",
        ),
        (
            "DELETE FROM audit_records WHERE id = $1",
            "DELETE audit_records",
        ),
        (
            "INSERT INTO tenants (id, slug, created_at) VALUES ($1, $2, now())",
            "INSERT tenants(id, slug, created_at)",
        ),
        (
            "SELECT 1 FROM quota_transaction_deltas",
            "SELECT quota_transaction_deltas",
        ),
        (
            "SELECT sagat_quota_add_delta($1, 1, 1, 1)",
            "EXECUTE sagat_quota_add_delta()",
        ),
        ("TRUNCATE audit_records", "TRUNCATE audit_records"),
        (
            "UPDATE approval_requests SET tenant_id = $1 WHERE request_id = $2",
            "UPDATE approval_requests.tenant_id",
        ),
        (
            "INSERT INTO audit_records (id, nonexistent_column) VALUES ($1, $2)",
            "INSERT audit_records(id, nonexistent_column)",
        ),
        (
            "INSERT INTO a_table_that_was_renamed (id) VALUES ($1)",
            "INSERT a_table_that_was_renamed(id)",
        ),
        (
            "INSERT INTO runtime_privileges_serial_probe (value) VALUES ($1)",
            "USAGE runtime_privileges_serial_probe_id_seq",
        ),
    ];
    // A table whose key is a `serial`: the INSERT calls nextval, which needs USAGE on the sequence.
    sqlx::query("CREATE TABLE runtime_privileges_serial_probe (id serial PRIMARY KEY, value int)")
        .execute(&db.pool)
        .await
        .expect("probe table");
    sqlx::query("GRANT SELECT, INSERT ON runtime_privileges_serial_probe TO sagat_app")
        .execute(&db.pool)
        .await
        .expect("probe grant");
    for (sql, expected) in drifted {
        let mut extracted = Extracted::default();
        let site = Site {
            file: "synthetic.rs".to_owned(),
            line: 1,
            excerpt: excerpt(sql),
        };
        for need in sql_needs::needs(sql) {
            extracted.needs.entry(need).or_default().push(site.clone());
        }
        let audit = audit(&db, &extracted).await;
        assert!(
            audit.disagreements.is_empty(),
            "{sql}: {:?}",
            audit.disagreements
        );
        assert!(
            audit.gaps.iter().any(|gap| gap.need == expected),
            "`{sql}` should be reported as `{expected}`; gaps: {:?}",
            audit.gaps.iter().map(|g| &g.need).collect::<Vec<_>>()
        );
    }
}

/// And it must not cry wolf on a statement the role is meant to run.
#[tokio::test]
async fn the_audit_accepts_statements_the_role_is_meant_to_run() {
    let db = migrated_db().await;
    let allowed = [
        "INSERT INTO audit_records (id, tenant_id, actor, action, resource_type, resource_id, created_at) VALUES ($1, $2, $3, $4, $5, $6, now())",
        "SELECT status FROM approval_requests WHERE tenant_id = $1 AND request_id = $2 FOR NO KEY UPDATE",
        "UPDATE approval_requests SET status = 'SETTLED', settled_at = now() WHERE tenant_id = $1 AND request_id = $2",
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
    ];
    for sql in allowed {
        let mut extracted = Extracted::default();
        let site = Site {
            file: "synthetic.rs".to_owned(),
            line: 1,
            excerpt: excerpt(sql),
        };
        for need in sql_needs::needs(sql) {
            extracted.needs.entry(need).or_default().push(site.clone());
        }
        let audit = audit(&db, &extracted).await;
        assert!(
            audit.disagreements.is_empty(),
            "{sql}: {:?}",
            audit.disagreements
        );
        let gaps: Vec<_> = audit.gaps.iter().map(|g| (&g.need, &g.why)).collect();
        assert!(gaps.is_empty(), "`{sql}` is meant to be allowed: {gaps:?}");
    }
}

/// `SAGAT_TEST_AS_RUNTIME_ROLE=1` must really connect the services of the world as `sagat_app`, or the
/// sweep proves nothing; without it, nothing may.
#[tokio::test]
async fn the_sweep_switch_connects_the_services_as_the_runtime_role_and_only_then() {
    use crate::support::engine::{activity, document, world};
    let w = world().await;
    let _ = w.publish(document("sweep", "a", vec![activity("a")])).await;
    let as_runtime: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND usename = $1",
    )
    .bind(RUNTIME_ROLE)
    .fetch_one(&w.db.pool)
    .await
    .expect("pg_stat_activity");
    assert_eq!(
        as_runtime > 0,
        crate::support::as_runtime_role(),
        "{as_runtime} connection(s) as {RUNTIME_ROLE}"
    );
}
