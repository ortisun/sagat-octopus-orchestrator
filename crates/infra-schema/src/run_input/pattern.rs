//! `pattern` (D043-054, D043-055, D043-063, D043-064, D043-066, D043-070): the syntax is checked node by node on the
//! parsed `regex-syntax` AST against an allowlist, never on the text, and the compiled program is bounded in size and
//! states. Every runtime engine of the regex is bounded explicitly, so what a compiled pattern can hold is known.
//!
//! The allowlist keeps the meaning of the pattern the same in ECMA-262 (which both drafts name) and in the `regex`
//! engine: literal characters, the escaped metacharacters, `\n`, `\r`, `\t`, `\xHH`, bracket classes of literals and
//! ranges (negated too), `.`, `^`, `$`, alternation, groups and `(?:...)`, and the repetitions with bounds of at most
//! 100. Everything else is refused, among it the Perl classes (Unicode in `regex`, ASCII in ECMA-262), word boundaries,
//! Unicode and POSIX classes, flags, `\A` and `\z`, named groups, class set operations and nested brackets (literals
//! in ECMA-262), a `]` first in a class and `[^]` (an empty class and any character in ECMA-262). Backreferences and
//! lookaround do not parse. `.` is compiled as "any character but `\n`, `\r`, U+2028 and U+2029", its ECMA-262 meaning.

use regex_automata::meta;
use regex_automata::nfa::thompson::{self, WhichCaptures};
use regex_automata::util::syntax;
use regex_syntax::ast::{
    self, AssertionKind, Ast, ClassSet, ClassSetItem, GroupKind, HexLiteralKind, LiteralKind,
    RepetitionKind, RepetitionRange, SpecialLiteralKind,
};

use super::{MAX_PATTERN_PROGRAM_BYTES, MAX_PATTERN_STATES};

/// The largest bound a repetition may have.
const MAX_REPETITION: u32 = 100;
/// How deep a pattern may nest groups and repetitions.
const MAX_PATTERN_NESTING: u32 = 64;
/// The capacity of each lazy DFA cache (forward and reverse) of a compiled pattern. Its weight counts both for every
/// matcher, so it sizes what the heaviest admitted schema weighs: at 64 KiB, 32 patterns weighed more than a tenant's
/// share of the cache and such a schema was compiled again on every start (security review of 2026-11-15, Major 2).
/// A pattern whose lazy DFA does not fit runs on the PikeVM, whose work the charge already prices (D043-063).
pub(super) const HYBRID_CACHE_BYTES: usize = 16 * 1024;
/// How many threads may match one pattern at once besides its owner: the regex keeps a cache for each (D043-064).
const CONCURRENT_MATCHERS: usize = 4;
// A pattern is matched only under a permit of the gate, so the caches its weight counts cover every matcher (D043-107).
const _: () = assert!(super::RUN_INPUT_PERMITS <= 1 + CONCURRENT_MATCHERS);
/// What `.` means in ECMA-262: any character but the four line terminators.
const ECMA_DOT: &str = r"[^\n\r\x{2028}\x{2029}]";

/// One compiled `pattern`.
pub(super) struct Pattern {
    regex: meta::Regex,
    /// The states of its NFA: a match costs `(bytes + 1) × states` units, charged before it runs.
    pub(super) states: u64,
    /// The bytes of its compiled programs.
    pub(super) program_bytes: usize,
    /// The bytes it may keep in memory with its runtime caches, for the weight of a cache entry.
    pub(super) weight: usize,
}

impl Pattern {
    pub(super) fn is_match(&self, text: &str) -> bool {
        self.regex.is_match(text)
    }
}

/// Why a pattern is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Refused {
    /// Outside the syntax allowlist, or not a pattern at all.
    Syntax,
    /// Past its program size or its states.
    Size,
}

/// Compiles `source` if the subset accepts it.
pub(super) fn compile(source: &str) -> Result<Pattern, Refused> {
    let parsed = ast::parse::ParserBuilder::new()
        .nest_limit(MAX_PATTERN_NESTING)
        .octal(false)
        .ignore_whitespace(false)
        .empty_min_range(false)
        .build()
        .parse(source)
        .map_err(|_| Refused::Syntax)?;
    let mut dots = Vec::new();
    allowed(&parsed, &mut dots)?;
    let translated = translate(source, &dots);
    let syntax = syntax::Config::new()
        .unicode(true)
        .utf8(true)
        .case_insensitive(false)
        .multi_line(false)
        .dot_matches_new_line(false)
        .nest_limit(MAX_PATTERN_NESTING);
    let nfa = thompson::Compiler::new()
        .syntax(syntax)
        .configure(
            thompson::Config::new()
                .nfa_size_limit(Some(MAX_PATTERN_PROGRAM_BYTES))
                .which_captures(WhichCaptures::None),
        )
        .build(&translated)
        .map_err(|_| Refused::Size)?;
    let states = nfa.states().len();
    if states > MAX_PATTERN_STATES || nfa.memory_usage() > MAX_PATTERN_PROGRAM_BYTES {
        return Err(Refused::Size);
    }
    let regex = meta::Builder::new()
        .configure(
            meta::Config::new()
                .which_captures(WhichCaptures::None)
                .nfa_size_limit(Some(MAX_PATTERN_PROGRAM_BYTES))
                .hybrid_cache_capacity(HYBRID_CACHE_BYTES)
                .dfa(false)
                .onepass(false)
                .backtrack(false),
        )
        .syntax(syntax)
        .build(&translated)
        .map_err(|_| Refused::Size)?;
    let program_bytes = regex.memory_usage();
    // One warm-up match sizes the PikeVM cache; the lazy DFA caches can grow to their capacity each.
    let mut cache = regex.create_cache();
    let _ = regex.search_with(&mut cache, &regex_automata::Input::new("warm-up"));
    let per_cache = cache.memory_usage().saturating_add(2 * HYBRID_CACHE_BYTES);
    Ok(Pattern {
        regex,
        states: states as u64,
        program_bytes,
        weight: program_bytes.saturating_add((1 + CONCURRENT_MATCHERS) * per_cache),
    })
}

/// Whether every node of `node` is in the allowlist; collects the offsets of every `.` to translate.
fn allowed(node: &Ast, dots: &mut Vec<(usize, usize)>) -> Result<(), Refused> {
    match node {
        Ast::Empty(_) => Ok(()),
        Ast::Dot(span) => {
            dots.push((span.start.offset, span.end.offset));
            Ok(())
        }
        Ast::Literal(literal) => literal_allowed(literal),
        Ast::Assertion(assertion) => match assertion.kind {
            AssertionKind::StartLine | AssertionKind::EndLine => Ok(()),
            _ => Err(Refused::Syntax),
        },
        Ast::ClassBracketed(class) => class_allowed(class),
        Ast::Repetition(repetition) => {
            let within = match &repetition.op.kind {
                RepetitionKind::ZeroOrOne
                | RepetitionKind::ZeroOrMore
                | RepetitionKind::OneOrMore => true,
                RepetitionKind::Range(
                    RepetitionRange::Exactly(n) | RepetitionRange::AtLeast(n),
                ) => *n <= MAX_REPETITION,
                RepetitionKind::Range(RepetitionRange::Bounded(m, n)) => {
                    *m <= MAX_REPETITION && *n <= MAX_REPETITION
                }
            };
            if !within {
                return Err(Refused::Syntax);
            }
            allowed(&repetition.ast, dots)
        }
        Ast::Group(group) => match &group.kind {
            GroupKind::CaptureIndex(_) => allowed(&group.ast, dots),
            GroupKind::NonCapturing(flags) if flags.items.is_empty() => allowed(&group.ast, dots),
            _ => Err(Refused::Syntax),
        },
        Ast::Alternation(alternation) => alternation.asts.iter().try_for_each(|a| allowed(a, dots)),
        Ast::Concat(concat) => concat.asts.iter().try_for_each(|a| allowed(a, dots)),
        Ast::Flags(_) | Ast::ClassUnicode(_) | Ast::ClassPerl(_) => Err(Refused::Syntax),
    }
}

fn literal_allowed(literal: &ast::Literal) -> Result<(), Refused> {
    match literal.kind {
        LiteralKind::Verbatim | LiteralKind::Meta | LiteralKind::Superfluous => Ok(()),
        LiteralKind::HexFixed(HexLiteralKind::X) => Ok(()),
        LiteralKind::Special(
            SpecialLiteralKind::Tab
            | SpecialLiteralKind::LineFeed
            | SpecialLiteralKind::CarriageReturn,
        ) => Ok(()),
        _ => Err(Refused::Syntax),
    }
}

fn class_allowed(class: &ast::ClassBracketed) -> Result<(), Refused> {
    let ClassSet::Item(item) = &class.kind else {
        // `&&`, `--`, `~~`: literals in ECMA-262.
        return Err(Refused::Syntax);
    };
    // A `]` written first: `[]a]` is an empty class then `a]` in ECMA-262, and `[^]` any character.
    let first = match item {
        ClassSetItem::Union(union) => union.items.first(),
        other => Some(other),
    };
    if let Some(ClassSetItem::Literal(literal)) = first
        && literal.c == ']'
        && literal.kind == LiteralKind::Verbatim
    {
        return Err(Refused::Syntax);
    }
    set_item_allowed(item)
}

fn set_item_allowed(item: &ClassSetItem) -> Result<(), Refused> {
    match item {
        ClassSetItem::Empty(_) => Ok(()),
        ClassSetItem::Literal(literal) => literal_allowed(literal),
        ClassSetItem::Range(range) => {
            literal_allowed(&range.start)?;
            literal_allowed(&range.end)
        }
        ClassSetItem::Union(union) => union.items.iter().try_for_each(set_item_allowed),
        // POSIX, Unicode and Perl classes, and a nested bracket (a literal `[` in ECMA-262).
        ClassSetItem::Ascii(_)
        | ClassSetItem::Unicode(_)
        | ClassSetItem::Perl(_)
        | ClassSetItem::Bracketed(_) => Err(Refused::Syntax),
    }
}

/// `source` with every `.` written as the class of its ECMA-262 meaning.
fn translate(source: &str, dots: &[(usize, usize)]) -> String {
    let mut sorted = dots.to_vec();
    sorted.sort_unstable();
    let mut out = String::with_capacity(source.len() + dots.len() * ECMA_DOT.len());
    let mut at = 0;
    for (start, end) in sorted {
        out.push_str(&source[at..start]);
        out.push_str(ECMA_DOT);
        at = end;
    }
    out.push_str(&source[at..]);
    out
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_dot_is_translated_to_its_ecma_meaning() {
        let pattern = compile("^a.b$").expect("compiles");
        assert!(pattern.is_match("axb"));
        for terminator in ["a\nb", "a\rb", "a\u{2028}b", "a\u{2029}b"] {
            assert!(!pattern.is_match(terminator), "{terminator:?}");
        }
        assert!(compile("[.]").expect("compiles").is_match("."));
    }
}
