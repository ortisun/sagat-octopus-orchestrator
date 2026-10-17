//! Just enough of a Rust lexer to find the SQL a store can issue: string literals (with `\`
//! continuations and raw strings), `concat!`, `macro_rules!` templates that wrap a `concat!`, and
//! `const` fragments that a `format!` template names. It never executes or expands anything else,
//! which is the limit of the audit: SQL assembled at run time from values it cannot see is reported
//! as unresolved, not guessed.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Str(String, usize),
    Ident(String),
    Punct(char),
}

/// One string literal that begins a SQL statement, after macro, `concat!` and `const` resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct Literal {
    pub line: usize,
    pub text: String,
}

/// What a source file contributes to the audit.
#[derive(Debug, Default)]
pub struct Extraction {
    pub statements: Vec<Literal>,
    /// Literals that carry a DML verb or a row-lock clause without being a statement themselves.
    pub fragments: Vec<Literal>,
    /// Statements of a kind the audit does not model (`LOCK TABLE`, `COPY`, DDL, ...). Written in
    /// capitals, as SQL is in this code base, so prose that starts with "Create" is not caught.
    pub unmodelled: Vec<Literal>,
}

pub fn lex(source: &str) -> Vec<Tok> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line) = (0usize, 1usize);
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            let mut depth = 1;
            i += 2;
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if let Some((text, next, lines)) = raw_string(&chars, i) {
            out.push(Tok::Str(text, line));
            line += lines;
            i = next;
        } else if c == '"' || (c == 'b' && chars.get(i + 1) == Some(&'"')) {
            let start_line = line;
            i += if c == 'b' { 2 } else { 1 };
            let mut text = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    let escaped = chars.get(i + 1).copied().unwrap_or('\\');
                    if escaped == '\n' {
                        // A trailing backslash drops the newline and the next line's indentation.
                        i += 2;
                        line += 1;
                        while i < chars.len() && chars[i].is_whitespace() {
                            if chars[i] == '\n' {
                                line += 1;
                            }
                            i += 1;
                        }
                        continue;
                    }
                    text.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => '\0',
                        other => other,
                    });
                    i += 2;
                    continue;
                }
                if chars[i] == '\n' {
                    line += 1;
                }
                text.push(chars[i]);
                i += 1;
            }
            i += 1;
            out.push(Tok::Str(text, start_line));
        } else if c == '\'' {
            // A character literal or a lifetime; neither carries SQL.
            if chars.get(i + 1) == Some(&'\\') {
                i += 2;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if chars.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                i += 1;
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else {
            out.push(Tok::Punct(c));
            i += 1;
        }
    }
    out
}

/// `r"..."`, `r#"..."#`, `br"..."`: returns the text, the index after it and the lines it spans.
fn raw_string(chars: &[char], at: usize) -> Option<(String, usize, usize)> {
    let mut i = at;
    if chars.get(i) == Some(&'b') {
        i += 1;
    }
    if chars.get(i) != Some(&'r') {
        return None;
    }
    i += 1;
    let mut hashes = 0;
    while chars.get(i) == Some(&'#') {
        hashes += 1;
        i += 1;
    }
    if chars.get(i) != Some(&'"') {
        return None;
    }
    // `r` and `b` also start ordinary identifiers (`result`, `bar`); a quote must follow directly.
    i += 1;
    let start = i;
    loop {
        if i >= chars.len() {
            return None;
        }
        if chars[i] == '"' && (0..hashes).all(|h| chars.get(i + 1 + h) == Some(&'#')) {
            let text: String = chars[start..i].iter().collect();
            let lines = text.matches('\n').count();
            return Some((text, i + 1 + hashes, lines));
        }
        i += 1;
    }
}

fn matching(tokens: &[Tok], open_at: usize, open: char, close: char) -> usize {
    let mut depth = 0;
    for (offset, token) in tokens[open_at..].iter().enumerate() {
        match token {
            Tok::Punct(c) if *c == open => depth += 1,
            Tok::Punct(c) if *c == close => {
                depth -= 1;
                if depth == 0 {
                    return open_at + offset;
                }
            }
            _ => {}
        }
    }
    tokens.len().saturating_sub(1)
}

/// `macro_rules! name { ($tail:literal) => { concat!("prefix", ..., $tail) }; }` -> name => prefix.
/// The definitions are removed from the stream: their text is expanded where they are invoked.
fn take_macros(tokens: Vec<Tok>) -> (BTreeMap<String, String>, Vec<Tok>) {
    let mut macros: BTreeMap<String, String> = BTreeMap::new();
    let mut kept = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if let (Tok::Ident(a), Some(Tok::Punct('!')), Some(Tok::Ident(name)), Some(Tok::Punct('{'))) = (
            &tokens[i],
            tokens.get(i + 1),
            tokens.get(i + 2),
            tokens.get(i + 3),
        ) && a == "macro_rules"
        {
            let end = matching(&tokens, i + 3, '{', '}');
            let mut prefix = String::new();
            let body = &tokens[i + 3..end];
            let from = body
                .iter()
                .position(|t| matches!(t, Tok::Ident(word) if word == "concat"))
                // A macro without `concat!` is a fixed fragment: `() => { "text" }`.
                .unwrap_or(0);
            let mut at = from;
            while at < body.len() {
                match &body[at] {
                    Tok::Str(text, _) => prefix.push_str(text),
                    Tok::Punct('$') => break,
                    Tok::Ident(fixed)
                        if macros.contains_key(fixed)
                            && matches!(
                                body.get(at + 1..at + 4),
                                Some([Tok::Punct('!'), Tok::Punct('('), Tok::Punct(')')])
                            ) =>
                    {
                        prefix.push_str(&macros[fixed]);
                        at += 3;
                    }
                    _ => {}
                }
                at += 1;
            }
            macros.insert(name.clone(), prefix);
            i = end + 1;
        } else {
            kept.push(tokens[i].clone());
            i += 1;
        }
    }
    (macros, kept)
}

/// `const NAME: &str = "..." ;` (also `concat!` and adjacent literals) -> NAME => text.
fn take_consts(tokens: &[Tok]) -> BTreeMap<String, String> {
    let mut consts = BTreeMap::new();
    for (i, token) in tokens.iter().enumerate() {
        let (Tok::Ident(keyword), Some(Tok::Ident(name))) = (token, tokens.get(i + 1)) else {
            continue;
        };
        if keyword != "const" && keyword != "static" {
            continue;
        }
        let Some(equals) = tokens[i..].iter().position(|t| *t == Tok::Punct('=')) else {
            continue;
        };
        let end = tokens[i..]
            .iter()
            .position(|t| *t == Tok::Punct(';'))
            .map_or(tokens.len(), |p| i + p);
        if i + equals > end {
            continue;
        }
        let strings: Vec<&str> = tokens[i + equals + 1..end]
            .iter()
            .filter_map(|t| match t {
                Tok::Str(text, _) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        if !strings.is_empty() {
            consts.insert(name.clone(), strings.concat());
        }
    }
    consts
}

/// Replaces macro invocations carrying a literal and `concat!` lists of literals with one literal.
fn expand(
    tokens: &[Tok],
    macros: &BTreeMap<String, String>,
    consts: &BTreeMap<String, String>,
) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if let (Tok::Ident(name), Some(Tok::Punct('!')), Some(Tok::Punct('('))) =
            (&tokens[i], tokens.get(i + 1), tokens.get(i + 2))
        {
            let close = matching(tokens, i + 2, '(', ')');
            let inner = &tokens[i + 3..close];
            if let (Some(prefix), [Tok::Str(tail, line)]) = (macros.get(name), inner) {
                out.push(Tok::Str(format!("{prefix}{tail}"), *line));
                i = close + 1;
                continue;
            }
            if let (Some(text), true) = (macros.get(name), inner.is_empty()) {
                // A macro taking no argument expands to its fixed text.
                out.push(Tok::Str(text.clone(), 0));
                i = close + 1;
                continue;
            }
            if name == "concat" {
                let mut text = String::new();
                let mut line = 0;
                let mut at_token = 0;
                while at_token < inner.len() {
                    match &inner[at_token] {
                        Tok::Str(part, at) => {
                            line = *at;
                            text.push_str(part);
                        }
                        // `name!()` of a macro that takes no argument: its fixed text.
                        Tok::Ident(fixed)
                            if macros.contains_key(fixed)
                                && matches!(
                                    inner.get(at_token + 1..at_token + 4),
                                    Some([Tok::Punct('!'), Tok::Punct('('), Tok::Punct(')')])
                                ) =>
                        {
                            text.push_str(&macros[fixed]);
                            at_token += 3;
                        }
                        Tok::Ident(constant) => match consts.get(constant) {
                            Some(value) => text.push_str(value),
                            None => text.push_str(&format!("{{{constant}}}")),
                        },
                        _ => {}
                    }
                    at_token += 1;
                }
                out.push(Tok::Str(text, line));
                i = close + 1;
                continue;
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    out
}

/// `{NAME}` placeholders that name a known `const` are replaced by its text.
fn resolve(text: &str, consts: &BTreeMap<String, String>) -> String {
    let mut resolved = text.to_owned();
    for _ in 0..3 {
        let mut changed = false;
        for (name, value) in consts {
            let placeholder = format!("{{{name}}}");
            if resolved.contains(&placeholder) {
                resolved = resolved.replace(&placeholder, value);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    resolved
}

fn first_word(text: &str) -> (&str, &str) {
    let trimmed = text.trim_start();
    let end = trimmed
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(trimmed.len());
    (&trimmed[..end], &trimmed[end..])
}

/// A literal is a statement when it opens with a statement keyword followed by white space.
pub fn is_statement(text: &str) -> bool {
    let (word, rest) = first_word(text);
    let word = word.to_ascii_uppercase();
    let verb = matches!(
        word.as_str(),
        "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "WITH" | "TRUNCATE"
    );
    if verb && rest.starts_with(char::is_whitespace) {
        return true;
    }
    // Session control, which needs no privilege; matched on the whole opening so prose such as
    // "set SAGAT_STUDIO_DIR ..." is not mistaken for a statement.
    let opening = text.trim_start().to_ascii_uppercase();
    [
        "SAVEPOINT ",
        "RELEASE SAVEPOINT ",
        "ROLLBACK TO SAVEPOINT ",
        "SET TRANSACTION ",
    ]
    .iter()
    .any(|prefix| opening.starts_with(prefix))
}

const UNMODELLED_VERBS: &[&str] = &[
    "LOCK",
    "CALL",
    "DO",
    "COPY",
    "EXPLAIN",
    "CREATE",
    "ALTER",
    "DROP",
    "GRANT",
    "REVOKE",
    "VACUUM",
    "ANALYZE",
    "REFRESH",
    "NOTIFY",
    "LISTEN",
    "UNLISTEN",
    "COMMENT",
    "REINDEX",
    "CLUSTER",
    "DISCARD",
    "RESET",
    "SHOW",
    "DECLARE",
    "PREPARE",
    "EXECUTE",
    "DEALLOCATE",
    "BEGIN",
    "COMMIT",
    "ABORT",
    "START",
    "VALUES",
    "SECURITY",
];

/// A statement kind the audit does not model: an all-capitals opening verb followed by white space.
fn is_unmodelled(text: &str) -> bool {
    let (word, rest) = first_word(text);
    UNMODELLED_VERBS.contains(&word) && (rest.is_empty() || rest.starts_with(char::is_whitespace))
}

/// A literal that is not a statement but would be dangerous if it were glued to one.
fn is_fragment(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    [
        "FOR UPDATE",
        "FOR NO KEY UPDATE",
        "FOR SHARE",
        "FOR KEY SHARE",
        "INSERT INTO ",
        "DELETE FROM ",
    ]
    .iter()
    .any(|needle| upper.contains(needle))
}

pub fn extract(source: &str) -> Extraction {
    let (macros, tokens) = take_macros(lex(source));
    let consts = take_consts(&tokens);
    let tokens = expand(&tokens, &macros, &consts);
    let mut extraction = Extraction::default();
    for token in tokens {
        let Tok::Str(text, line) = token else {
            continue;
        };
        let text = resolve(&text, &consts);
        if is_statement(&text) {
            extraction.statements.push(Literal { line, text });
        } else if is_unmodelled(&text) {
            extraction.unmodelled.push(Literal { line, text });
        } else if is_fragment(&text) {
            extraction.fragments.push(Literal { line, text });
        }
    }
    extraction
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_continued_literal_is_one_statement_without_the_indentation() {
        let found =
            extract("fn f() { q(\"SELECT a \\\n            FROM t \\\n   WHERE x = 1\"); }");
        assert_eq!(found.statements.len(), 1);
        assert_eq!(found.statements[0].text, "SELECT a FROM t WHERE x = 1");
        assert_eq!(found.statements[0].line, 1);
    }

    #[test]
    fn a_raw_literal_keeps_quotes_and_backslashes() {
        let found = extract("const Q: &str = r#\"SELECT '\\d' FROM t WHERE a = \"b\"\"#;");
        assert_eq!(
            found.statements[0].text,
            "SELECT '\\d' FROM t WHERE a = \"b\""
        );
    }

    #[test]
    fn comments_and_lifetimes_and_chars_hide_nothing_and_invent_nothing() {
        let found = extract(
            "// SELECT * FROM hidden\n/* UPDATE t SET a = 1 */\nfn f<'a>(x: &'a str) { let c = '\"'; q(\"DELETE FROM visible\"); }",
        );
        assert_eq!(found.statements.len(), 1);
        assert_eq!(found.statements[0].text, "DELETE FROM visible");
    }

    #[test]
    fn a_macro_template_is_expanded_where_it_is_invoked() {
        let found = extract(
            "macro_rules! sel { ($tail:literal) => { concat!(\"SELECT id \", \"FROM memberships \", $tail) }; }\n\
             fn f() { q(sel!(\"WHERE id = $1 FOR SHARE\")); }",
        );
        assert_eq!(
            found
                .statements
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["SELECT id FROM memberships WHERE id = $1 FOR SHARE"]
        );
        assert!(
            found.fragments.is_empty(),
            "the tail was consumed by the expansion"
        );
    }

    #[test]
    fn a_fixed_security_predicate_inside_a_query_template_is_not_omitted() {
        let found = extract(
            "macro_rules! policy { () => { \"AND EXISTS (SELECT 1 FROM secret_pools) \" }; }\n\
             macro_rules! sel { ($tail:literal) => { concat!(\"SELECT id FROM steps WHERE true \", policy!(), $tail) }; }\n\
             const QUERY: &str = sel!(\"ORDER BY id LIMIT $1\"); fn f() { q(QUERY); }",
        );
        assert_eq!(found.statements.len(), 1);
        assert_eq!(
            found.statements[0].text,
            "SELECT id FROM steps WHERE true AND EXISTS (SELECT 1 FROM secret_pools) ORDER BY id LIMIT $1"
        );
        assert!(found.fragments.is_empty());
    }

    #[test]
    fn a_macro_without_an_argument_is_expanded_to_its_text() {
        let found = extract(
            "macro_rules! fixed { () => { \"AND NOT archived\" }; }\n\
             fn f() { q(concat!(\"SELECT id FROM steps WHERE \", fixed!(), \" ORDER BY id\")); }",
        );
        assert_eq!(
            found
                .statements
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["SELECT id FROM steps WHERE AND NOT archived ORDER BY id"]
        );
    }

    #[test]
    fn a_const_named_in_a_template_is_substituted() {
        let found =
            extract("const COLS: &str = \"a, b\";\nfn f() { format!(\"SELECT {COLS} FROM t\"); }");
        assert!(
            found
                .statements
                .iter()
                .any(|s| s.text == "SELECT a, b FROM t")
        );
    }

    #[test]
    fn a_lock_clause_outside_a_statement_is_reported_as_a_fragment() {
        let found = extract("fn f() { let tail = \" WHERE id = $1 FOR UPDATE\"; }");
        assert!(found.statements.is_empty());
        assert_eq!(found.fragments.len(), 1);
    }

    #[test]
    fn statements_the_audit_does_not_model_are_reported_not_ignored() {
        let found = extract(
            "fn f() { q(\"LOCK TABLE runs IN SHARE MODE\"); q(\"Create the thing\"); q(\"begin\"); q(\"COMMIT\"); }",
        );
        let kinds: Vec<_> = found.unmodelled.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(kinds, ["LOCK TABLE runs IN SHARE MODE", "COMMIT"]);
    }

    #[test]
    fn prose_that_starts_with_a_verb_is_not_a_statement() {
        assert!(!is_statement("update_failed"));
        assert!(!is_statement("settled"));
        assert!(!is_statement(
            "set SAGAT_STUDIO_DIR on the control plane to serve it"
        ));
        assert!(is_statement("  update t set a = 1"));
        assert!(is_statement(
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY"
        ));
        assert!(is_statement("RELEASE SAVEPOINT event_admission_budget"));
    }
}
