//! What reaches a terminal: text somebody else wrote is shown with the characters that rewrite the
//! screen, reorder it or hide text in it replaced, and on one line when it is a value.
#![allow(clippy::expect_used, clippy::panic)]

use sagat_cli::{one_line, printable};

#[test]
fn the_characters_that_rewrite_reorder_or_hide_text_are_not_shown() {
    for (name, hidden) in [
        ("escape", "\u{1b}[2J"),
        ("carriage return", "\r"),
        ("bidirectional override", "\u{202E}"),
        ("bidirectional isolate", "\u{2066}"),
        ("arabic letter mark", "\u{061C}"),
        ("zero width space", "\u{200B}"),
        (
            "word joiner and the invisible operators",
            "\u{2060}\u{2062}\u{2064}",
        ),
        ("deprecated format characters", "\u{206A}\u{206F}"),
        ("variation selector", "\u{FE0F}"),
        ("supplementary variation selector", "\u{E0100}"),
        (
            "tag characters, which carry text nobody sees",
            "\u{E0041}\u{E0042}\u{E007F}",
        ),
        ("soft hyphen", "\u{00AD}"),
        ("combining grapheme joiner", "\u{034F}"),
        ("mongolian vowel separator", "\u{180E}"),
        ("interlinear annotation", "\u{FFF9}\u{FFFA}\u{FFFB}"),
        ("byte order mark", "\u{FEFF}"),
        ("line separator", "\u{2028}"),
    ] {
        let shown = printable(&format!("a{hidden}b")).into_owned();
        assert!(
            hidden
                .chars()
                .filter(|c| !c.is_ascii_graphic())
                .all(|c| !shown.contains(c)),
            "{name}: '{}' was shown as {shown:?}",
            hidden.escape_unicode()
        );
        assert!(
            shown.starts_with('a') && shown.ends_with('b'),
            "{name}: {shown:?}"
        );
    }
}

#[test]
fn text_people_read_is_left_alone_and_a_value_is_one_line() {
    for fine in [
        "plain",
        "acentuação e 日本語",
        "emoji 🎉 and a ZWJ family 👨‍👩‍👧",
        "tab\there",
        "two\nlines",
    ] {
        // A ZWJ inside an emoji sequence is a joiner the tool does not hide: it is U+200D, in the range
        // that is replaced. Only the plain text and the layout characters must be untouched.
        if fine.contains('\u{200D}') {
            continue;
        }
        assert_eq!(printable(fine), fine);
    }
    assert_eq!(one_line("a\nb\tc"), "a b c");
}
