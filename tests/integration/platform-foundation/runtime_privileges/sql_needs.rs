//! What a SQL statement asks of the role that runs it, read from the text alone. The scanner knows
//! the shapes the stores use (INSERT with a column list, UPDATE ... SET, ON CONFLICT DO UPDATE,
//! DELETE, row-locking SELECT, data-modifying CTEs, function calls); it does not parse SQL, and
//! anything it cannot place is reported as `Unresolved` rather than dropped.

use std::collections::BTreeSet;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Need {
    /// SELECT on a relation (table or view).
    Select(String),
    /// INSERT into the listed columns, or every column when the list is empty.
    Insert(String, Vec<String>),
    /// UPDATE of one column.
    Update(String, String),
    Delete(String),
    /// `FOR UPDATE|NO KEY UPDATE|SHARE|KEY SHARE`: PostgreSQL asks for UPDATE on some column.
    RowLock(String),
    Truncate(String),
    /// A function call; the name is resolved against the catalog.
    Execute(String),
    /// A place the scanner could not resolve; the text says where.
    Unresolved(String),
}

impl fmt::Display for Need {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Select(t) => write!(f, "SELECT {t}"),
            Self::Insert(t, columns) if columns.is_empty() => write!(f, "INSERT {t}"),
            Self::Insert(t, columns) => write!(f, "INSERT {t}({})", columns.join(", ")),
            Self::Update(t, column) => write!(f, "UPDATE {t}.{column}"),
            Self::Delete(t) => write!(f, "DELETE {t}"),
            Self::RowLock(t) => write!(f, "ROW LOCK {t}"),
            Self::Truncate(t) => write!(f, "TRUNCATE {t}"),
            Self::Execute(name) => write!(f, "EXECUTE {name}()"),
            Self::Unresolved(what) => write!(f, "UNRESOLVED {what}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Sql {
    Word(String),
    Quoted,
    Param,
    Hole(String),
    Sym(char),
}

fn tokenize(sql: &str) -> Vec<Sql> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '\'' {
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            i += 1;
            out.push(Sql::Quoted);
        } else if c == '"' {
            let start = i + 1;
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
            out.push(Sql::Word(chars[start..i].iter().collect()));
            i += 1;
        } else if c == '$' && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Sql::Param);
        } else if c == '$' && chars.get(i + 1) == Some(&'{') {
            // `${n}`: a `format!` argument that is the number of a bind parameter.
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
            i += 1;
            out.push(Sql::Param);
        } else if c == '{' {
            let start = i + 1;
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
            out.push(Sql::Hole(chars[start..i.min(chars.len())].iter().collect()));
            i += 1;
        } else if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Sql::Word(
                chars[start..i]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase(),
            ));
        } else {
            out.push(Sql::Sym(c));
            i += 1;
        }
    }
    out
}

/// Words that cannot be a table alias.
const NOT_AN_ALIAS: &[&str] = &[
    "where",
    "join",
    "inner",
    "left",
    "right",
    "full",
    "cross",
    "natural",
    "on",
    "using",
    "group",
    "order",
    "limit",
    "offset",
    "for",
    "union",
    "intersect",
    "except",
    "returning",
    "set",
    "having",
    "window",
    "lateral",
    "fetch",
    "values",
    "select",
    "from",
    "and",
    "or",
    "when",
    "then",
    "else",
    "end",
    "do",
    "into",
    "with",
    "as",
    "not",
    "is",
    "in",
    "tablesample",
    "only",
];

struct Scan {
    t: Vec<Sql>,
    /// Paren/bracket depth of each token (an opener and its closer carry the outer depth).
    depth: Vec<usize>,
    /// The index of the opener of the group each token sits in.
    group: Vec<Option<usize>>,
}

impl Scan {
    fn new(sql: &str) -> Self {
        let t = tokenize(sql);
        let (mut depth, mut group, mut stack) = (Vec::new(), Vec::new(), Vec::<usize>::new());
        for (i, token) in t.iter().enumerate() {
            match token {
                Sql::Sym('(' | '[') => {
                    depth.push(stack.len());
                    group.push(stack.last().copied());
                    stack.push(i);
                }
                Sql::Sym(')' | ']') => {
                    stack.pop();
                    depth.push(stack.len());
                    group.push(stack.last().copied());
                }
                _ => {
                    depth.push(stack.len());
                    group.push(stack.last().copied());
                }
            }
        }
        Self { t, depth, group }
    }

    fn word(&self, i: usize) -> Option<&str> {
        match self.t.get(i) {
            Some(Sql::Word(w)) => Some(w),
            _ => None,
        }
    }

    fn is(&self, i: usize, word: &str) -> bool {
        self.word(i) == Some(word)
    }

    fn sym(&self, i: usize, c: char) -> bool {
        self.t.get(i) == Some(&Sql::Sym(c))
    }

    /// A possibly schema-qualified relation name at `i`; `public.` is dropped. Returns the name and
    /// the index after it, or `None` for a placeholder, a subquery or a function call.
    fn relation(&self, mut i: usize) -> Option<(String, usize)> {
        if self.is(i, "only") {
            i += 1;
        }
        let first = self.word(i)?.to_owned();
        if self.sym(i + 1, '.')
            && let Some(second) = self.word(i + 2)
        {
            let name = if first == "public" {
                second.to_owned()
            } else {
                format!("{first}.{second}")
            };
            return Some((name, i + 3));
        }
        Some((first, i + 1))
    }

    /// An optional alias after a relation: `AS x` or a bare word that is not a keyword.
    fn alias(&self, i: usize) -> (Option<String>, usize) {
        if self.is(i, "as")
            && let Some(alias) = self.word(i + 1)
        {
            // `AS x(a, b)` names columns too; skip them.
            let mut next = i + 2;
            if self.sym(next, '(') {
                next = self.close_of(next) + 1;
            }
            return (Some(alias.to_owned()), next);
        }
        match self.word(i) {
            Some(word) if !NOT_AN_ALIAS.contains(&word) => (Some(word.to_owned()), i + 1),
            _ => (None, i),
        }
    }

    fn close_of(&self, open: usize) -> usize {
        let mut level = 0;
        for i in open..self.t.len() {
            match self.t[i] {
                Sql::Sym('(' | '[') => level += 1,
                Sql::Sym(')' | ']') => {
                    level -= 1;
                    if level == 0 {
                        return i;
                    }
                }
                _ => {}
            }
        }
        self.t.len() - 1
    }

    /// The assignments of a SET clause starting at `i`: the target columns.
    fn assignments(&self, mut i: usize) -> Vec<String> {
        let base = self.depth[i.min(self.t.len() - 1)];
        let mut columns = Vec::new();
        loop {
            if self.sym(i, '(') {
                // (a, b) = (...): every name inside is a target.
                let close = self.close_of(i);
                columns.extend((i + 1..close).filter_map(|k| self.word(k).map(str::to_owned)));
                i = close + 1;
            } else if let Some(column) = self.word(i) {
                columns.push(column.to_owned());
                i += 1;
            } else {
                break;
            }
            // Skip `= expression` up to the comma that separates the next assignment.
            while i < self.t.len() {
                if self.depth[i] < base {
                    return columns;
                }
                if self.depth[i] == base
                    && (self.sym(i, ',')
                        || matches!(self.word(i), Some("where" | "from" | "returning")))
                {
                    break;
                }
                i += 1;
            }
            if self.sym(i, ',') {
                i += 1;
            } else {
                break;
            }
        }
        columns
    }

    /// The relations named by FROM and JOIN clauses, at any depth: name, alias, the depth they sit at
    /// and the index of the keyword. CTE names are included; the caller filters them.
    fn relations_read(&self) -> Vec<(String, Option<String>, usize, usize)> {
        let mut found = Vec::new();
        let mut i = 0;
        while i < self.t.len() {
            let keyword = self.word(i);
            let in_from = keyword == Some("from") && !self.is_syntax_from(i);
            if in_from || keyword == Some("join") {
                let mut at = i + 1;
                while let Some((name, next)) = self.relation(at) {
                    if self.sym(next, '(') {
                        // A function in FROM, such as `unnest(...)`: not a relation.
                        break;
                    }
                    let (alias, after) = self.alias(next);
                    found.push((name, alias, self.depth[i], i));
                    at = after;
                    if in_from && self.sym(at, ',') {
                        at += 1;
                    } else {
                        break;
                    }
                }
            }
            i += 1;
        }
        found
    }

    /// `extract(epoch FROM x)`, `substring(x FROM 1)`, `IS DISTINCT FROM`, `DELETE FROM`.
    fn is_syntax_from(&self, i: usize) -> bool {
        if self.is(i.wrapping_sub(1), "distinct") || self.is(i.wrapping_sub(1), "delete") {
            return true;
        }
        self.group[i].is_some_and(|open| {
            open > 0
                && matches!(
                    self.word(open - 1),
                    Some("extract" | "substring" | "trim" | "overlay" | "position")
                )
        })
    }

    fn cte_names(&self) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for i in 0..self.t.len() {
            let Some(name) = self.word(i) else { continue };
            let mut j = i + 1;
            if self.sym(j, '(') {
                j = self.close_of(j) + 1;
            }
            if !self.is(j, "as") {
                continue;
            }
            j += 1;
            if self.is(j, "not") {
                j += 1;
            }
            if self.is(j, "materialized") {
                j += 1;
            }
            let introduced = self.is(i.wrapping_sub(1), "with")
                || self.is(i.wrapping_sub(1), "recursive")
                || self.sym(i.wrapping_sub(1), ',');
            if self.sym(j, '(') && introduced {
                names.insert(name.to_owned());
            }
        }
        names
    }
}

/// Functions the scanner does not look up: statement syntax that looks like a call.
const SYNTAX: &[&str] = &[
    "in",
    "exists",
    "any",
    "all",
    "some",
    "values",
    "array",
    "row",
    "coalesce",
    "greatest",
    "least",
    "nullif",
    "cast",
    "over",
    "filter",
    "within",
    "and",
    "or",
    "not",
    "on",
    "using",
    "as",
    "when",
    "then",
    "else",
    "case",
    "end",
    "select",
    "from",
    "where",
    "set",
    "by",
    "conflict",
    "into",
    "returning",
    "distinct",
    "union",
    "all",
    "except",
    "intersect",
    "limit",
    "offset",
    "having",
    "between",
    "like",
    "ilike",
    "is",
    "lateral",
    "materialized",
    "partition",
    "order",
    "join",
    "key",
    "update",
    "share",
    "of",
    "with",
    "recursive",
    "interval",
    "timestamp",
    "date",
    "time",
    "text",
    "varchar",
    "numeric",
    "bigint",
    "integer",
    "smallint",
    "boolean",
    "uuid",
    "jsonb",
    "json",
    "bytea",
    "float8",
    "float4",
    "int",
    "int4",
    "int8",
    "int2",
    "timestamptz",
    "char",
    // PL/pgSQL control flow in trigger bodies.
    "if",
    "elsif",
    "elseif",
    "while",
    "return",
    "raise",
    "perform",
    "loop",
    "foreach",
    "for",
];

/// The privileges the statement needs, in the order they were found, without repeats.
pub fn needs(sql: &str) -> Vec<Need> {
    let scan = Scan::new(sql);
    let ctes = scan.cte_names();
    let mut out: Vec<Need> = Vec::new();
    let mut push = |need: Need| {
        if !out.contains(&need) {
            out.push(need);
        }
    };
    let mut consumed: BTreeSet<usize> = BTreeSet::new();
    // The DML targets seen so far, with the depth they sit at: RETURNING and ON CONFLICT refer to
    // the nearest one.
    let mut targets: Vec<(String, usize)> = Vec::new();
    let mut last_insert: Option<String> = None;

    let items = scan.relations_read();
    for (name, _, _, at) in &items {
        consumed.insert(*at + 1);
        if !ctes.contains(name) {
            push(Need::Select(name.clone()));
        }
    }

    let mut in_delete = false;
    for i in 0..scan.t.len() {
        let Some(word) = scan.word(i) else { continue };
        let previous = scan.word(i.wrapping_sub(1));
        match word {
            "insert" if scan.is(i + 1, "into") => {
                let Some((table, next)) = scan.relation(i + 2) else {
                    push(Need::Unresolved("INSERT INTO <placeholder>".to_owned()));
                    continue;
                };
                consumed.insert(i + 2);
                let mut after = next;
                if scan.is(after, "as") {
                    after += 2;
                }
                let mut columns = Vec::new();
                if scan.sym(after, '(') {
                    let close = scan.close_of(after);
                    let only_names =
                        (after + 1..close).all(|k| scan.word(k).is_some() || scan.sym(k, ','));
                    let continues = matches!(
                        scan.word(close + 1),
                        Some("values" | "select" | "with" | "overriding" | "default")
                    );
                    if only_names && continues {
                        columns = (after + 1..close)
                            .filter_map(|k| scan.word(k).map(str::to_owned))
                            .collect();
                    }
                }
                push(Need::Insert(table.clone(), columns));
                targets.push((table.clone(), scan.depth[i]));
                last_insert = Some(table);
            }
            "update" if !matches!(previous, Some("for" | "key" | "do")) => {
                let Some((table, next)) = scan.relation(i + 1) else {
                    push(Need::Unresolved("UPDATE <placeholder>".to_owned()));
                    continue;
                };
                let (_, after) = scan.alias(next);
                if !scan.is(after, "set") {
                    continue;
                }
                consumed.insert(i + 1);
                for column in scan.assignments(after + 1) {
                    push(Need::Update(table.clone(), column));
                }
                push(Need::Select(table.clone()));
                targets.push((table, scan.depth[i]));
            }
            "update" if previous == Some("do") && scan.is(i + 1, "set") => {
                let Some(table) = last_insert.clone() else {
                    push(Need::Unresolved(
                        "ON CONFLICT DO UPDATE without INSERT".to_owned(),
                    ));
                    continue;
                };
                for column in scan.assignments(i + 2) {
                    push(Need::Update(table.clone(), column));
                }
            }
            "delete" if scan.is(i + 1, "from") => {
                in_delete = true;
                let Some((table, _)) = scan.relation(i + 2) else {
                    push(Need::Unresolved("DELETE FROM <placeholder>".to_owned()));
                    continue;
                };
                consumed.insert(i + 2);
                push(Need::Delete(table.clone()));
                push(Need::Select(table.clone()));
                targets.push((table, scan.depth[i]));
            }
            "using" if in_delete => {
                if let Some((table, _)) = scan.relation(i + 1) {
                    consumed.insert(i + 1);
                    push(Need::Select(table));
                }
            }
            "truncate" => {
                let at = if scan.is(i + 1, "table") {
                    i + 2
                } else {
                    i + 1
                };
                match scan.relation(at) {
                    Some((table, _)) => {
                        consumed.insert(at);
                        push(Need::Truncate(table));
                    }
                    None => push(Need::Unresolved("TRUNCATE <placeholder>".to_owned())),
                }
            }
            "on" if scan.is(i + 1, "conflict") => {
                if let Some(table) = last_insert.clone() {
                    push(Need::Select(table));
                }
            }
            "returning" => {
                if let Some((table, _)) = targets.iter().rev().find(|(_, d)| *d == scan.depth[i]) {
                    push(Need::Select(table.clone()));
                }
            }
            "for" => {
                let (strength_end, locks) =
                    match (scan.word(i + 1), scan.word(i + 2), scan.word(i + 3)) {
                        (Some("update" | "share"), _, _) => (i + 1, true),
                        (Some("no"), Some("key"), Some("update"))
                        | (Some("key"), Some("share"), _) => {
                            (i + 2 + usize::from(scan.is(i + 1, "no")), true)
                        }
                        _ => (i, false),
                    };
                if !locks {
                    continue;
                }
                let scope: Vec<_> = items
                    .iter()
                    .filter(|(_, _, depth, at)| {
                        *depth == scan.depth[i] && scan.group[*at] == scan.group[i]
                    })
                    .collect();
                let mut named: Vec<String> = Vec::new();
                if scan.is(strength_end + 1, "of") {
                    let mut k = strength_end + 2;
                    while let Some(name) = scan.word(k) {
                        if matches!(name, "nowait" | "skip") {
                            break;
                        }
                        named.push(name.to_owned());
                        k += 1;
                        if scan.sym(k, ',') {
                            k += 1;
                        } else {
                            break;
                        }
                    }
                }
                let tables: Vec<String> = if named.is_empty() {
                    scope.iter().map(|(name, ..)| name.clone()).collect()
                } else {
                    named
                        .iter()
                        .map(|n| {
                            scope
                                .iter()
                                .find(|(name, alias, ..)| alias.as_deref() == Some(n) || name == n)
                                .map_or_else(|| n.clone(), |(name, ..)| name.clone())
                        })
                        .collect()
                };
                if tables.is_empty() {
                    push(Need::Unresolved("row lock without a table".to_owned()));
                }
                for table in tables {
                    if !ctes.contains(&table) {
                        push(Need::RowLock(table));
                    }
                }
            }
            _ => {
                let call = scan.sym(i + 1, '(');
                let table_position = consumed.contains(&i);
                let alias_columns = previous == Some("as");
                if call
                    && !table_position
                    && !alias_columns
                    && !SYNTAX.contains(&word)
                    && !ctes.contains(word)
                {
                    push(Need::Execute(word.to_owned()));
                }
            }
        }
    }
    for t in &scan.t {
        if let Sql::Hole(name) = t {
            // A `format!` argument or a `const` the extractor could not read: its text is not
            // visible here, so whatever it contributes is not audited.
            push(Need::Unresolved(format!("placeholder {{{name}}}")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(sql: &str) -> Vec<String> {
        needs(sql).iter().map(ToString::to_string).collect()
    }

    fn has(sql: &str, expected: &str) {
        let got = set(sql);
        assert!(
            got.iter().any(|n| n == expected),
            "{expected} not in {got:?} for {sql}"
        );
    }

    fn lacks(sql: &str, unexpected: &str) {
        let got = set(sql);
        assert!(
            !got.iter().any(|n| n == unexpected),
            "{unexpected} in {got:?} for {sql}"
        );
    }

    #[test]
    fn a_share_lock_is_a_row_lock_of_the_table_it_names() {
        // The statement of finding G294: the runtime role may read memberships but not lock a row.
        let sql = "SELECT role FROM memberships WHERE tenant_id = $1 AND subject = $2 FOR SHARE";
        has(sql, "ROW LOCK memberships");
        has(sql, "SELECT memberships");
    }

    #[test]
    fn a_lock_restricted_with_of_covers_only_that_alias() {
        let sql = "SELECT r.id FROM runs r JOIN workflows w ON w.id = r.workflow_id FOR NO KEY UPDATE OF r SKIP LOCKED";
        has(sql, "ROW LOCK runs");
        lacks(sql, "ROW LOCK workflows");
        has(sql, "SELECT workflows");
    }

    #[test]
    fn a_lock_in_a_subquery_binds_to_the_subquery_tables_only() {
        let sql = "UPDATE steps SET status = 'X' WHERE id IN (SELECT id FROM runs WHERE a = 1 FOR UPDATE SKIP LOCKED)";
        has(sql, "ROW LOCK runs");
        lacks(sql, "ROW LOCK steps");
        has(sql, "UPDATE steps.status");
    }

    #[test]
    fn an_insert_lists_its_columns_and_an_upsert_adds_the_updated_ones() {
        let sql = "INSERT INTO quota_rate_buckets (tenant_id, window_start, count) VALUES ($1, $2, 1) \
                   ON CONFLICT (tenant_id, window_start) DO UPDATE SET count = quota_rate_buckets.count + 1, \
                   updated_at = now() RETURNING count";
        has(
            sql,
            "INSERT quota_rate_buckets(tenant_id, window_start, count)",
        );
        has(sql, "UPDATE quota_rate_buckets.count");
        has(sql, "UPDATE quota_rate_buckets.updated_at");
        has(sql, "SELECT quota_rate_buckets");
    }

    #[test]
    fn an_insert_from_a_select_without_a_column_list_needs_the_whole_table() {
        has("INSERT INTO t SELECT a FROM u", "INSERT t");
        has("INSERT INTO t SELECT a FROM u", "SELECT u");
    }

    #[test]
    fn commas_inside_an_assignment_do_not_split_it() {
        let sql = "UPDATE t SET a = coalesce(a, 0) + 1, b = CASE WHEN x THEN 1 ELSE 2 END, c = $1 WHERE id = $2";
        for column in ["a", "b", "c"] {
            has(sql, &format!("UPDATE t.{column}"));
        }
        lacks(sql, "UPDATE t.x");
    }

    #[test]
    fn a_data_modifying_cte_is_found_and_its_name_is_not_a_table() {
        let sql = "WITH moved AS (DELETE FROM outbox WHERE id = $1 RETURNING id) \
                   INSERT INTO archive (id) SELECT id FROM moved";
        has(sql, "DELETE outbox");
        has(sql, "INSERT archive(id)");
        lacks(sql, "SELECT moved");
    }

    #[test]
    fn extract_from_and_distinct_from_are_not_relations() {
        let sql = "SELECT extract(epoch FROM now() - created_at) FROM t WHERE a IS DISTINCT FROM b";
        has(sql, "SELECT t");
        lacks(sql, "SELECT created_at");
        lacks(sql, "SELECT b");
    }

    #[test]
    fn a_comma_join_and_a_function_in_from_are_told_apart() {
        let sql = "SELECT 1 FROM a, b x, unnest($1::uuid[]) AS s(id) WHERE true";
        has(sql, "SELECT a");
        has(sql, "SELECT b");
        has(sql, "EXECUTE unnest()");
        lacks(sql, "SELECT s");
        lacks(sql, "SELECT unnest");
    }

    #[test]
    fn function_calls_are_reported_for_the_catalog_to_resolve() {
        has(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            "EXECUTE pg_advisory_xact_lock()",
        );
        has(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            "EXECUTE hashtextextended()",
        );
        lacks("INSERT INTO t (a) VALUES (1)", "EXECUTE t()");
    }

    #[test]
    fn a_placeholder_where_a_table_belongs_is_unresolved_not_dropped() {
        has(
            "INSERT INTO {table} (a) VALUES ($1)",
            "UNRESOLVED INSERT INTO <placeholder>",
        );
        has(
            "INSERT INTO {table} (a) VALUES ($1)",
            "UNRESOLVED placeholder {table}",
        );
        has(
            "UPDATE {table} SET a = 1",
            "UNRESOLVED UPDATE <placeholder>",
        );
    }

    #[test]
    fn braces_inside_a_sql_string_are_json_not_placeholders() {
        let found = set("INSERT INTO steps (a, b) VALUES ($1, '{}'::jsonb)");
        assert!(
            !found.iter().any(|n| n.starts_with("UNRESOLVED")),
            "{found:?}"
        );
    }

    #[test]
    fn update_inside_a_lock_clause_is_not_an_update_statement() {
        let sql = "SELECT id FROM runs WHERE id = $1 FOR NO KEY UPDATE";
        assert!(
            !set(sql).iter().any(|n| n.starts_with("UPDATE ")),
            "{:?}",
            set(sql)
        );
        has(sql, "ROW LOCK runs");
    }

    #[test]
    fn a_placeholder_that_is_a_parameter_number_is_not_unresolved() {
        let found = set("SELECT a FROM t WHERE x = ${n} LIMIT ${m} + 1");
        assert!(
            !found.iter().any(|n| n.starts_with("UNRESOLVED")),
            "{found:?}"
        );
    }

    #[test]
    fn plpgsql_control_flow_is_not_a_function_call() {
        let body = "BEGIN IF (OLD.a, OLD.b) IS DISTINCT FROM (NEW.a, NEW.b) THEN RAISE EXCEPTION 'x'; END IF; RETURN NEW; END";
        assert!(
            !set(body).iter().any(|n| n.starts_with("EXECUTE")),
            "{:?}",
            set(body)
        );
    }

    #[test]
    fn a_schema_qualified_relation_keeps_its_schema_unless_it_is_public() {
        has("SELECT 1 FROM public.runs", "SELECT runs");
        has(
            "SELECT 1 FROM pg_catalog.pg_class",
            "SELECT pg_catalog.pg_class",
        );
    }
}
