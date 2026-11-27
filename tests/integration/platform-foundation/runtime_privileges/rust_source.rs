//! Just enough of a Rust lexer to find the SQL a store can issue: string literals (with `\`
//! continuations and raw strings), `concat!`, `macro_rules!` templates that wrap a `concat!`, and
//! `const` fragments that a `format!` template names, plus explicitly scoped parent predicates.
//! Unresolved SQL calls retain structured provenance. It never executes Rust source,
//! which is the limit of the audit: SQL assembled at run time from values it cannot see is reported
//! as unresolved, not guessed.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Str(String, usize),
    Expanded(Expansion, usize, bool),
    Ident(String, usize),
    Punct(char),
}

/// SQL text and unresolved calls travel together: const substitution cannot erase a call.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Expansion {
    text: String,
    unresolved: Vec<String>,
    /// A macro of literal parameters whose one rule is a `concat!` of literals and those parameters
    /// (`($g:literal, $now:literal) => { concat!($g, ".leased_at < ", $now) }`): its parts, a parameter by its
    /// index. Expanded only where it is called with one string literal per parameter.
    template: Option<Vec<Part>>,
}

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Text(Expansion),
    Param(usize),
}

impl Expansion {
    fn append(&mut self, other: &Self) {
        self.text.push_str(&other.text);
        self.unresolved.extend(other.unresolved.iter().cloned());
    }

    /// The text of a template called with `args`; `None` when the call does not give one literal per parameter.
    fn instantiate(&self, args: &[String]) -> Option<Self> {
        let parts = self.template.as_ref()?;
        let wanted = parts
            .iter()
            .filter_map(|part| match part {
                Part::Param(index) => Some(index + 1),
                Part::Text(_) => None,
            })
            .max()
            .unwrap_or(0);
        if args.len() < wanted {
            return None;
        }
        let mut out = Self::default();
        for part in parts {
            match part {
                Part::Text(text) => out.append(text),
                Part::Param(index) => out.text.push_str(&args[*index]),
            }
        }
        Some(out)
    }

    fn unknown(name: &str) -> Self {
        Self {
            text: format!("{{macro {name}!()}}"),
            unresolved: vec![name.to_owned()],
            template: None,
        }
    }
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
    pub unresolved: Vec<Literal>,
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
            out.push(Tok::Ident(chars[start..i].iter().collect(), line));
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
fn take_macros(
    tokens: Vec<Tok>,
    mut macros: BTreeMap<String, Expansion>,
    module: Option<&str>,
) -> (BTreeMap<String, Expansion>, Vec<Tok>) {
    let mut kept = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if let (
            Tok::Ident(a, _),
            Some(Tok::Punct('!')),
            Some(Tok::Ident(name, _)),
            Some(Tok::Punct('{')),
        ) = (
            &tokens[i],
            tokens.get(i + 1),
            tokens.get(i + 2),
            tokens.get(i + 3),
        ) && a == "macro_rules"
        {
            let end = matching(&tokens, i + 3, '{', '}');
            let body = &tokens[i + 3..end];
            if let Some(template) = literal_template(body, &macros) {
                if let Some(module) = module {
                    macros.insert(format!("crate::{module}::{name}"), template.clone());
                    macros.insert(format!("$crate::{module}::{name}"), template.clone());
                }
                macros.insert(name.clone(), template);
                i = end + 1;
                continue;
            }
            let mut prefix = Expansion::default();
            let from = body
                .iter()
                .position(|t| matches!(t, Tok::Ident(word, _) if word == "concat"))
                // A macro without `concat!` is a fixed fragment: `() => { "text" }`.
                .unwrap_or(0);
            let mut at = from;
            while at < body.len() {
                if let Some((name, next, _)) = fixed_call(body, at) {
                    prefix.append(
                        &fixed_text(&macros, &name)
                            .cloned()
                            .unwrap_or_else(|| Expansion::unknown(&name)),
                    );
                    at = next;
                    continue;
                }
                match &body[at] {
                    Tok::Str(text, _) => prefix.text.push_str(text),
                    Tok::Punct('$') => break,
                    _ => {}
                }
                at += 1;
            }
            if let Some(module) = module {
                macros.insert(format!("crate::{module}::{name}"), prefix.clone());
                macros.insert(format!("$crate::{module}::{name}"), prefix.clone());
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

/// The template of a macro body `{ ($a:literal, ...) => { concat!(...) }; }` whose `concat!` holds only literals, its
/// parameters and fixed calls; `None` for any other shape (it is then read as a prefix, as before).
fn literal_template(body: &[Tok], macros: &BTreeMap<String, Expansion>) -> Option<Expansion> {
    // body[0] is `{`; the rule's matcher follows
    if body.get(1) != Some(&Tok::Punct('(')) {
        return None;
    }
    let close = matching(body, 1, '(', ')');
    let mut params = Vec::new();
    for item in body[2..close].split(|t| *t == Tok::Punct(',')) {
        match item {
            [] => {}
            [
                Tok::Punct('$'),
                Tok::Ident(name, _),
                Tok::Punct(':'),
                Tok::Ident(kind, _),
            ] if kind == "literal" => params.push(name.clone()),
            _ => return None,
        }
    }
    if params.is_empty() {
        return None;
    }
    let concat = body[close..]
        .iter()
        .position(|t| matches!(t, Tok::Ident(word, _) if word == "concat"))?
        + close;
    if body.get(concat + 1..concat + 3) != Some(&[Tok::Punct('!'), Tok::Punct('(')]) {
        return None;
    }
    let inner_close = matching(body, concat + 2, '(', ')');
    let inner = &body[concat + 3..inner_close];
    let mut parts = Vec::new();
    let mut at = 0;
    while at < inner.len() {
        // a template called with literals and this template's parameters (`live_gate_run!($r)` inside
        // `eligible_gate_waiter!`): its parts, each parameter of the call replaced by what the caller gave
        if let Some((nested, next)) = nested_template(inner, at, macros, &params) {
            parts.extend(nested);
            at = next;
            continue;
        }
        if let Some((name, next, _)) = fixed_call(inner, at) {
            parts.push(Part::Text(
                fixed_text(macros, &name)
                    .cloned()
                    .unwrap_or_else(|| Expansion::unknown(&name)),
            ));
            at = next;
            continue;
        }
        match (&inner[at], inner.get(at + 1)) {
            (Tok::Str(text, _), _) => parts.push(Part::Text(Expansion {
                text: text.clone(),
                ..Expansion::default()
            })),
            (Tok::Punct('$'), Some(Tok::Ident(name, _))) => {
                parts.push(Part::Param(params.iter().position(|p| p == name)?));
                at += 1;
            }
            (Tok::Punct(','), _) => {}
            _ => return None,
        }
        at += 1;
    }
    Some(Expansion {
        template: Some(parts),
        ..Expansion::default()
    })
}

/// A call at `at`, inside a template's `concat!`, of another template whose arguments are literals or parameters of the
/// enclosing template (`params`): the called template's parts with each of its parameters replaced by its argument, and
/// the index after the call's `)`. `None` when `at` is no such call.
fn nested_template(
    tokens: &[Tok],
    at: usize,
    macros: &BTreeMap<String, Expansion>,
    params: &[String],
) -> Option<(Vec<Part>, usize)> {
    let (Tok::Ident(name, _), Some(Tok::Punct('!')), Some(Tok::Punct('('))) =
        (tokens.get(at)?, tokens.get(at + 1), tokens.get(at + 2))
    else {
        return None;
    };
    let called = macros.get(name)?.template.as_ref()?;
    let close = matching(tokens, at + 2, '(', ')');
    let mut args = Vec::new();
    for item in tokens[at + 3..close].split(|t| *t == Tok::Punct(',')) {
        match item {
            [] => {}
            [Tok::Str(text, _)] => args.push(Part::Text(Expansion {
                text: text.clone(),
                ..Expansion::default()
            })),
            [Tok::Punct('$'), Tok::Ident(param, _)] => {
                args.push(Part::Param(params.iter().position(|p| p == param)?));
            }
            _ => return None,
        }
    }
    let mut parts = Vec::with_capacity(called.len());
    for part in called {
        parts.push(match part {
            Part::Text(text) => Part::Text(text.clone()),
            Part::Param(index) => args.get(*index)?.clone(),
        });
    }
    Some((parts, close + 1))
}

/// A call `name!("a", "b")` of a template at `at`: its expansion and the index after the call's `)`. A call that does
/// not give one literal per parameter is unresolved (its text is not guessed).
fn template_call(
    tokens: &[Tok],
    at: usize,
    macros: &BTreeMap<String, Expansion>,
) -> Option<(Expansion, usize)> {
    let (Tok::Ident(name, _), Some(Tok::Punct('!')), Some(Tok::Punct('('))) =
        (tokens.get(at)?, tokens.get(at + 1), tokens.get(at + 2))
    else {
        return None;
    };
    let template = macros.get(name).filter(|m| m.template.is_some())?;
    let close = matching(tokens, at + 2, '(', ')');
    let mut args = Vec::new();
    for item in tokens[at + 3..close].split(|t| *t == Tok::Punct(',')) {
        match item {
            [] => {}
            [Tok::Str(text, _)] => args.push(text.clone()),
            _ => return Some((Expansion::unknown(name), close + 1)),
        }
    }
    Some((
        template
            .instantiate(&args)
            .unwrap_or_else(|| Expansion::unknown(name)),
        close + 1,
    ))
}

/// Only the fixed no-argument calls this repository uses, preserving any unknown path in full.
fn fixed_call(tokens: &[Tok], at: usize) -> Option<(String, usize, usize)> {
    let mut next = at;
    let mut name = String::new();
    if tokens.get(next) == Some(&Tok::Punct('$')) {
        name.push('$');
        next += 1;
    }
    let line = match tokens.get(next)? {
        Tok::Ident(_, line) => *line,
        _ => return None,
    };
    loop {
        let Tok::Ident(part, _) = tokens.get(next)? else {
            return None;
        };
        name.push_str(part);
        next += 1;
        if tokens.get(next..next + 2) == Some(&[Tok::Punct(':'), Tok::Punct(':')]) {
            name.push_str("::");
            next += 2;
        } else {
            break;
        }
    }
    (tokens.get(next..next + 3) == Some(&[Tok::Punct('!'), Tok::Punct('('), Tok::Punct(')')]))
        .then_some((name, next + 3, line))
}

fn fixed_text<'a>(macros: &'a BTreeMap<String, Expansion>, name: &str) -> Option<&'a Expansion> {
    macros.get(name)
}

/// `const NAME: &str = "..." ;` (also `concat!` and adjacent literals) -> NAME => text.
fn take_consts(tokens: &[Tok]) -> BTreeMap<String, String> {
    let mut consts = BTreeMap::new();
    for (i, token) in tokens.iter().enumerate() {
        let (Tok::Ident(keyword, _), Some(Tok::Ident(name, _))) = (token, tokens.get(i + 1)) else {
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
/// SQLx query argument boundaries, including the generic variants used by these stores.
fn sql_arguments(tokens: &[Tok], at: usize) -> Option<(usize, usize)> {
    let [
        Tok::Ident(module, _),
        Tok::Punct(':'),
        Tok::Punct(':'),
        Tok::Ident(function, _),
    ] = tokens.get(at..at + 4)?
    else {
        return None;
    };
    if module != "sqlx"
        || !matches!(
            function.as_str(),
            "query" | "query_as" | "query_scalar" | "raw_sql"
        )
    {
        return None;
    }
    let mut open = at + 4;
    if tokens.get(open..open + 3) == Some(&[Tok::Punct(':'), Tok::Punct(':'), Tok::Punct('<')]) {
        open = matching(tokens, open + 2, '<', '>') + 1;
    }
    (tokens.get(open) == Some(&Tok::Punct('('))).then(|| (open, matching(tokens, open, '(', ')')))
}

fn expand(
    tokens: &[Tok],
    macros: &BTreeMap<String, Expansion>,
    consts: &BTreeMap<String, String>,
    sql_context: bool,
) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if let Some((open, close)) = sql_arguments(tokens, i) {
            out.extend_from_slice(&tokens[i..=open]);
            out.extend(expand(&tokens[open + 1..close], macros, consts, true));
            out.push(tokens[close].clone());
            i = close + 1;
            continue;
        }
        if let Some((name, next, line)) = fixed_call(tokens, i) {
            if let Some(text) = fixed_text(macros, &name) {
                out.push(Tok::Expanded(text.clone(), line, sql_context));
            } else if sql_context {
                out.push(Tok::Expanded(Expansion::unknown(&name), line, true));
            } else {
                // Keep an unrelated Rust macro opaque; never retry its qualified basename.
                out.extend_from_slice(&tokens[i..next]);
            }
            i = next;
            continue;
        }
        if let Some((expansion, next)) = template_call(tokens, i, macros) {
            let line = tokens[i..next]
                .iter()
                .find_map(|t| match t {
                    Tok::Str(_, line) | Tok::Ident(_, line) => Some(*line),
                    _ => None,
                })
                .unwrap_or(0);
            out.push(Tok::Expanded(expansion, line, sql_context));
            i = next;
            continue;
        }
        if let (Tok::Ident(name, _), Some(Tok::Punct('!')), Some(Tok::Punct('('))) =
            (&tokens[i], tokens.get(i + 1), tokens.get(i + 2))
        {
            let close = matching(tokens, i + 2, '(', ')');
            let inner = &tokens[i + 3..close];
            if let (Some(prefix), [Tok::Str(tail, line)]) = (macros.get(name), inner) {
                let mut text = prefix.clone();
                text.text.push_str(tail);
                out.push(Tok::Expanded(text, *line, sql_context));
                i = close + 1;
                continue;
            }
            if name == "concat" {
                let mut text = Expansion::default();
                let mut line = 0;
                let mut at = 0;
                while at < inner.len() {
                    if let Some((expansion, next)) = template_call(inner, at, macros) {
                        text.append(&expansion);
                        at = next;
                        continue;
                    }
                    if let Some((name, next, _)) = fixed_call(inner, at) {
                        text.append(
                            &fixed_text(macros, &name)
                                .cloned()
                                .unwrap_or_else(|| Expansion::unknown(&name)),
                        );
                        at = next;
                        continue;
                    }
                    match &inner[at] {
                        Tok::Str(part, source_line) => {
                            line = *source_line;
                            text.text.push_str(part);
                        }
                        Tok::Ident(constant, _) => text.text.push_str(
                            &consts
                                .get(constant)
                                .cloned()
                                .unwrap_or_else(|| format!("{{{constant}}}")),
                        ),
                        _ => {}
                    }
                    at += 1;
                }
                out.push(Tok::Expanded(text, line, sql_context));
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
    extract_with_macros(source, &BTreeMap::new(), false)
}

/// Only qualified declarations escape the parent module; its SQL statements never do.
pub fn macro_prelude(source: &str) -> BTreeMap<String, Expansion> {
    macro_prelude_of(source, "execution", &BTreeMap::new())
}

/// The macros a module of the store declares, under the paths other modules call them by
/// (`crate::<module>::name`): a statement that splices a fragment of another module is audited
/// with its text, never reported as an unresolved placeholder. `known` are the declarations read
/// before, which a nested call may name.
pub fn macro_prelude_of(
    source: &str,
    module: &str,
    known: &BTreeMap<String, Expansion>,
) -> BTreeMap<String, Expansion> {
    take_macros(lex(source), known.clone(), Some(module))
        .0
        .into_iter()
        .filter(|(name, _)| {
            name.starts_with(&format!("crate::{module}::"))
                || name.starts_with(&format!("$crate::{module}::"))
        })
        .collect()
}

/// The macros the crate root declares, by name: textually before its modules, so every module calls them unqualified.
pub fn root_macros(source: &str) -> BTreeMap<String, Expansion> {
    take_macros(lex(source), BTreeMap::new(), None).0
}

fn macro_scope(
    tokens: &[Tok],
    parent: &BTreeMap<String, Expansion>,
    lexical: bool,
) -> BTreeMap<String, Expansion> {
    let mut scope = parent.clone();
    if lexical {
        for (name, value) in parent {
            if let Some(name) = name.strip_prefix("crate::execution::") {
                scope.insert(name.into(), value.clone());
            }
        }
        return scope;
    }
    let modules: BTreeSet<&str> = parent
        .keys()
        .filter_map(|name| name.strip_prefix("crate::"))
        .filter_map(|name| name.rsplit_once("::").map(|(module, _)| module))
        .collect();
    for module in modules {
        import_from(tokens, parent, module, &mut scope);
    }
    scope
}

/// `use crate::<module>::{a, b as c};` brings the module's declared macros into scope by name.
fn import_from(
    tokens: &[Tok],
    parent: &BTreeMap<String, Expansion>,
    module: &str,
    scope: &mut BTreeMap<String, Expansion>,
) {
    let prefix = lex(&format!("use crate::{module}::"));
    let mut depth = 0_i32;
    for at in 0..tokens.len() {
        match tokens[at] {
            Tok::Punct('{') => depth += 1,
            Tok::Punct('}') => depth -= 1,
            _ => {}
        }
        if depth != 0
            || !tokens.get(at..at + prefix.len()).is_some_and(|actual| {
                actual.iter().zip(&prefix).all(|(a, b)| match (a, b) {
                    (Tok::Ident(a, _), Tok::Ident(b, _)) => a == b,
                    _ => a == b,
                })
            })
        {
            continue;
        }
        let start = at + prefix.len();
        let imported = if tokens.get(start) == Some(&Tok::Punct('{')) {
            &tokens[start + 1..matching(tokens, start, '{', '}')]
        } else {
            let end = tokens[start..]
                .iter()
                .position(|t| *t == Tok::Punct(';'))
                .map_or(tokens.len(), |n| start + n);
            &tokens[start..end]
        };
        for item in imported.split(|t| *t == Tok::Punct(',')) {
            let (name, alias) = match item {
                [Tok::Ident(name, _)] => (name, name),
                [
                    Tok::Ident(name, _),
                    Tok::Ident(keyword, _),
                    Tok::Ident(alias, _),
                ] if keyword == "as" => (name, alias),
                _ => continue,
            };
            if let Some(value) = parent.get(&format!("crate::{module}::{name}")) {
                scope.insert(alias.clone(), value.clone());
            }
        }
    }
}

pub fn extract_with_macros(
    source: &str,
    inherited: &BTreeMap<String, Expansion>,
    lexical: bool,
) -> Extraction {
    let tokens = lex(source);
    let scope = macro_scope(&tokens, inherited, lexical);
    let (macros, tokens) = take_macros(tokens, scope, None);
    let consts = take_consts(&tokens);
    let tokens = expand(&tokens, &macros, &consts, false);
    let mut extraction = Extraction::default();
    for token in tokens {
        let (text, line) = match token {
            Tok::Str(text, line) => (text, line),
            Tok::Expanded(expansion, line, sql_context) => {
                let mut shape = expansion.text.clone();
                for name in &expansion.unresolved {
                    shape = shape.replace(&format!("{{macro {name}!()}}"), "");
                }
                let report = sql_context
                    || is_statement(&shape)
                    || is_unmodelled(&shape)
                    || is_fragment(&shape);
                for name in expansion.unresolved.into_iter().filter(|_| report) {
                    extraction.unresolved.push(Literal {
                        line,
                        text: format!("unresolved SQL macro {name}!()"),
                    });
                }
                (expansion.text, line)
            }
            _ => continue,
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
    fn a_template_calling_a_template_with_its_parameters_is_expanded() {
        // the shape of `eligible_gate_waiter!` (crates/infra-postgres/src/lib.rs, task T022-112), which calls
        // `live_gate_run!` with its own parameter
        let source = "macro_rules! live { ($r:literal) => { concat!($r, \".status IN ('RUNNING') AND NOT \", $r, \".paused\") }; }\n\
             macro_rules! eligible { ($w:literal, $r:literal, $now:literal) => { concat!($w, \".deadline_at > \", $now, \" AND \", live!($r)) }; }\n\
             fn f() { q(concat!(\"SELECT 1 FROM waiters w JOIN runs r ON true WHERE \", eligible!(\"w\", \"r\", \"now()\"))); }";
        let found = extract(source);
        assert_eq!(found.statements.len(), 1, "{found:?}");
        assert_eq!(
            found.statements[0].text,
            "SELECT 1 FROM waiters w JOIN runs r ON true WHERE w.deadline_at > now() AND r.status IN ('RUNNING') AND NOT r.paused"
        );
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
    fn a_macro_of_literal_parameters_is_expanded_with_its_arguments() {
        // the form of `purgeable_grant!` (feature 022): parameters substituted, never glued into the text
        let found = extract(
            r#"macro_rules! aged { ($g:literal, $now:literal) => { concat!($g, ".leased_at < ", $now) }; }
               fn f() { q(concat!("DELETE FROM grants g WHERE ", aged!("g", "clock_timestamp()"), " AND true")); }"#,
        );
        assert_eq!(found.statements.len(), 1, "{found:?}");
        assert_eq!(
            found.statements[0].text,
            "DELETE FROM grants g WHERE g.leased_at < clock_timestamp() AND true"
        );
        assert!(found.unresolved.is_empty(), "{found:?}");
    }

    #[test]
    fn a_template_called_without_literals_is_unresolved_instead_of_guessed() {
        let found = extract(
            r#"macro_rules! aged { ($g:literal) => { concat!($g, ".leased_at < now()") }; }
               fn f(alias: &str) { q(concat!("DELETE FROM grants g WHERE ", aged!(alias))); }"#,
        );
        assert_eq!(found.unresolved.len(), 1, "{found:?}");
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
    fn qualified_fixed_predicates_preserve_nested_privilege_requirements() {
        let parent = macro_prelude(
            r#"
            macro_rules! policy { () => { "AND EXISTS (SELECT 1 FROM secret_pools FOR SHARE) " }; }
            macro_rules! complete { () => { concat!($crate::execution::policy!(), "AND true ") }; }
        "#,
        );
        let found = extract_with_macros(
            r#"fn f() { q(concat!("SELECT id FROM steps WHERE true ", crate::execution::complete!(), "ORDER BY id")); }"#,
            &parent,
            false,
        );
        assert_eq!(found.statements.len(), 1);
        assert_eq!(
            found.statements[0].text,
            "SELECT id FROM steps WHERE true AND EXISTS (SELECT 1 FROM secret_pools FOR SHARE) AND true ORDER BY id"
        );
        assert!(found.fragments.is_empty() && found.unresolved.is_empty());
    }

    /// B1 of the keys review (2026-11-20): a fragment another module of the store declares is
    /// audited where it is spliced, called by its path or imported by name.
    #[test]
    fn a_fragment_of_another_module_is_expanded_by_path_and_by_import() {
        let execution = macro_prelude(r#"macro_rules! policy { () => { "AND true " }; }"#);
        let flow_control = macro_prelude_of(
            r#"macro_rules! lateral { () => { " LEFT JOIN LATERAL (SELECT 1 FROM concurrency_holders h) p ON true " }; }"#,
            "flow_control",
            &execution,
        );
        let mut inherited = execution;
        inherited.extend(flow_control);
        for source in [
            r#"fn f() { q(concat!("SELECT id FROM runs", crate::flow_control::lateral!(), "WHERE true")); }"#,
            r#"use crate::flow_control::lateral;
               fn f() { q(concat!("SELECT id FROM runs", lateral!(), "WHERE true")); }"#,
        ] {
            let found = extract_with_macros(source, &inherited, false);
            assert!(found.unresolved.is_empty(), "{source}");
            assert_eq!(
                found.statements[0].text,
                "SELECT id FROM runs LEFT JOIN LATERAL (SELECT 1 FROM concurrency_holders h) p ON true WHERE true"
            );
        }
        // a module that declares nothing by that name lends nothing
        let found = extract_with_macros(
            r#"fn f() { q(concat!("SELECT id FROM runs", crate::other::lateral!())); }"#,
            &inherited,
            false,
        );
        assert!(!found.unresolved.is_empty());
    }

    #[test]
    fn an_unknown_nested_fixed_macro_is_unresolved_instead_of_omitted() {
        let found = extract(
            r#"
            macro_rules! complete { () => { concat!(missing_policy!(), "AND true ") }; }
            fn f() { q(concat!("SELECT id FROM steps WHERE true ", complete!(), "ORDER BY id")); }
        "#,
        );
        assert_eq!(found.statements.len(), 1);
        assert!(
            found.statements[0]
                .text
                .contains("{macro missing_policy!()}"),
            "an unknown predicate cannot silently disappear: {:?}",
            found.statements
        );
    }

    #[test]
    fn inherited_predicates_keep_caller_lines_and_exclude_parent_statements() {
        let inherited = macro_prelude(
            r#"
            macro_rules! policy { () => { "AND EXISTS (SELECT 1 FROM secret_pools FOR SHARE) " }; }
            fn parent() { q("DELETE FROM parent_only"); }
        "#,
        );
        let found = extract_with_macros(
            r#"

fn child() { q(concat!("SELECT id FROM steps WHERE true ", policy!(), "ORDER BY id")); }"#,
            &inherited,
            true,
        );
        assert_eq!(found.statements.len(), 1);
        assert_eq!(found.statements[0].line, 3);
        assert_eq!(
            found.statements[0].text,
            "SELECT id FROM steps WHERE true AND EXISTS (SELECT 1 FROM secret_pools FOR SHARE) ORDER BY id"
        );
        let shadow = extract_with_macros(
            r#"
            macro_rules! policy { () => { "AND EXISTS (SELECT 1 FROM local_policy FOR UPDATE) " }; }
            fn child() { q(concat!("SELECT id FROM steps WHERE true ", policy!())); }
        "#,
            &inherited,
            true,
        );
        assert!(
            shadow.statements[0]
                .text
                .contains("local_policy FOR UPDATE")
        );
        assert!(!shadow.statements[0].text.contains("secret_pools"));
    }

    #[test]
    fn an_unknown_module_cannot_borrow_an_inherited_predicate_basename() {
        let inherited = macro_prelude(r#"macro_rules! policy { () => { "AND harmless" }; }"#);
        let found = extract_with_macros(
            r#"fn child() { q(concat!("SELECT id FROM steps WHERE true ", other_module::policy!())); }"#,
            &inherited,
            true,
        );
        assert_eq!(
            found.statements[0].text,
            "SELECT id FROM steps WHERE true {macro other_module::policy!()}"
        );
    }

    #[test]
    fn unresolved_macros_cannot_be_erased_by_consts_or_hidden_before_a_sql_verb() {
        for body in [
            r#"const POLICY: &str = ""; macro_rules! query { () => { concat!("SELECT id FROM steps ", POLICY!()) }; } fn f() { sqlx::query(query!()); }"#,
            r#"macro_rules! query { () => { concat!(POLICY!(), "SELECT id FROM steps") }; } fn f() { sqlx::query(query!()); }"#,
            r#"macro_rules! query { () => { concat!(POLICY!()) }; } fn f() { sqlx::query(query!()); }"#,
            r#"fn f() { sqlx::query(POLICY!()); }"#,
            r#"fn f() { sqlx::query_as::<_, (String, i32)>(POLICY!()); }"#,
        ] {
            let found = extract(body);
            assert!(
                !found.unresolved.is_empty(),
                "an unknown macro must remain an explicit audit failure: {found:?}"
            );
        }
    }

    #[test]
    fn local_shadowing_does_not_replace_a_qualified_execution_predicate() {
        let parent = macro_prelude(
            r#"macro_rules! policy { () => { "AND EXISTS (SELECT 1 FROM parent_policy FOR SHARE) " }; }"#,
        );
        let found = extract_with_macros(
            r#"
            macro_rules! policy { () => { "AND EXISTS (SELECT 1 FROM local_policy FOR UPDATE) " }; }
            fn f() { sqlx::query(concat!("SELECT id FROM steps WHERE true ", crate::execution::policy!())); }
        "#,
            &parent,
            false,
        );
        assert!(found.statements[0].text.contains("parent_policy FOR SHARE"));
        assert!(!found.statements[0].text.contains("local_policy"));
    }

    #[test]
    fn unqualified_parent_predicates_require_lexical_scope_or_an_explicit_import() {
        let parent = macro_prelude(r#"macro_rules! policy { () => { "AND true " }; }"#);
        let missing = extract_with_macros(
            r#"fn f() { sqlx::query(concat!("SELECT id FROM steps WHERE true ", policy!())); }"#,
            &parent,
            false,
        );
        assert_eq!(missing.unresolved.len(), 1);
        for source in [
            r#"use crate::execution::{policy as imported}; fn f() { sqlx::query(concat!("SELECT id FROM steps WHERE true ", imported!())); }"#,
            r#"use crate::execution::policy as imported; fn f() { sqlx::query(concat!("SELECT id FROM steps WHERE true ", imported!())); }"#,
        ] {
            let found = extract_with_macros(source, &parent, false);
            assert!(found.unresolved.is_empty());
            assert_eq!(
                found.statements[0].text,
                "SELECT id FROM steps WHERE true AND true "
            );
        }
        let ordinary = extract(r#"fn f() { unreachable!(); }"#);
        assert!(
            ordinary.unresolved.is_empty(),
            "an unrelated Rust macro is not SQL"
        );
    }

    #[test]
    fn ordinary_non_sql_wrappers_and_concat_are_not_sql_failures() {
        for source in [
            r#"macro_rules! ordinary { () => { unreachable!() }; } fn f() { ordinary!(); }"#,
            r#"fn f() { concat!(module_path!(), "label"); }"#,
        ] {
            let found = extract(source);
            assert!(
                found.unresolved.is_empty(),
                "ordinary Rust expression is not SQL: {found:?}"
            );
        }
    }

    #[test]
    fn a_direct_unknown_sql_macro_retains_its_actual_caller_line() {
        let found = extract("\n\nfn f() {\n    sqlx::query(POLICY!());\n}");
        assert_eq!(found.unresolved.len(), 1);
        assert_eq!(found.unresolved[0].line, 4);
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
