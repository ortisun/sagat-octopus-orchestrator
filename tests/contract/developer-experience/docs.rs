//! The examples in the documentation are run as far as a document can be run, by the tool they
//! show: the commands parse with the tool's own parser, the excerpts are in the files they name, the
//! scripts they run exist and the links lead somewhere.

use std::path::{Path, PathBuf};

use crate::reference::repo;

struct Block {
    file: PathBuf,
    line: usize,
    info: String,
    body: Vec<String>,
}

/// The markdown files whose examples are checked.
fn files() -> Vec<PathBuf> {
    let root = repo();
    let mut found = Vec::new();
    let mut extras = Vec::new();
    let mut visit = |dir: &Path| {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                found.push(path);
            }
        }
    };
    visit(&root.join("docs/quickstarts"));
    visit(&root.join("docs/reference"));
    // The documents that say how to run it and why, with the ports, the variables and the commands.
    visit(&root.join("specs/012-developer-experience"));
    for extra in [
        "docs/runbooks/developer-experience.md",
        "docs/adr/ADR-0019-the-first-day-runs-on-the-real-engine-with-canned-answers-and-a-development-identity-provider.md",
    ] {
        extras.push(root.join(extra));
    }
    for group in ["templates", "examples"] {
        for entry in std::fs::read_dir(root.join(group))
            .into_iter()
            .flatten()
            .flatten()
        {
            if entry.path().is_dir() {
                visit(&entry.path());
                for inner in std::fs::read_dir(entry.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    if inner.path().is_dir() {
                        visit(&inner.path());
                    }
                }
            }
        }
    }
    found.extend(extras);
    found.retain(|p| !p.to_string_lossy().contains("node_modules"));
    found.sort();
    found
}

fn blocks(file: &Path, text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut open: Option<Block> = None;
    for (n, line) in text.lines().enumerate() {
        match (&mut open, line.strip_prefix("```")) {
            (None, Some(info)) => {
                open = Some(Block {
                    file: file.to_path_buf(),
                    line: n + 1,
                    info: info.trim().to_owned(),
                    body: Vec::new(),
                });
            }
            (Some(_), Some("")) => out.extend(open.take()),
            (Some(block), _) => block.body.push(line.to_owned()),
            (None, None) => {}
        }
    }
    out
}

fn all_blocks() -> Vec<Block> {
    files()
        .iter()
        .flat_map(|f| blocks(f, &std::fs::read_to_string(f).expect("read")))
        .collect()
}

/// A block of shell: what the quickstarts and the runbooks call `sh`, `bash` or `shell`.
fn is_shell(block: &Block) -> bool {
    let kind = block.info.split_whitespace().next().unwrap_or_default();
    matches!(kind, "sh" | "bash" | "shell" | "console")
}

fn shown(block: &Block) -> String {
    format!(
        "{}:{}",
        block
            .file
            .strip_prefix(repo())
            .unwrap_or(&block.file)
            .display(),
        block.line
    )
}

#[test]
fn an_excerpt_is_in_the_file_it_names_just_as_shown() {
    let mut checked = 0;
    for block in all_blocks() {
        let Some(path) = block
            .info
            .split_whitespace()
            .find_map(|w| w.strip_prefix("excerpt="))
        else {
            continue;
        };
        assert!(
            block.body.iter().any(|l| !l.trim().is_empty()),
            "{}: an excerpt with nothing in it proves nothing",
            shown(&block)
        );
        let source = std::fs::read_to_string(repo().join(path))
            .unwrap_or_else(|e| panic!("{}: {path}: {e}", shown(&block)));
        let lines: Vec<&str> = source.lines().collect();
        let found = (0..lines.len()).any(|start| {
            let indent = lines[start].len() - lines[start].trim_start().len();
            block.body.iter().enumerate().all(|(k, shown_line)| {
                let Some(line) = lines.get(start + k) else {
                    return false;
                };
                if shown_line.trim().is_empty() {
                    line.trim().is_empty()
                } else {
                    line.trim_end()
                        .get(indent..)
                        .is_some_and(|rest| rest == shown_line.trim_end())
                        && line[..indent].trim().is_empty()
                }
            })
        });
        assert!(
            found,
            "{}: the excerpt is not in {path} as shown (the example changed? regenerate the page)",
            shown(&block)
        );
        checked += 1;
    }
    assert!(
        checked >= 8,
        "only {checked} excerpts are checked: the pages lost them?"
    );
}

/// A shell line as words, the way a shell reads quotes (and nothing else of it).
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, '\\') => {
                if let Some(next) = chars.next() {
                    word.push(next);
                    started = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if started || !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, '#') if !started && word.is_empty() => break,
            (None, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if started || !word.is_empty() {
        out.push(word);
    }
    out
}

/// The logical lines of a shell block: continuations joined, comments left to `words`.
fn commands(block: &Block) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut pending: Option<(usize, String)> = None;
    for (k, raw) in block.body.iter().enumerate() {
        let line = raw.trim_end();
        let (start, mut text) = pending
            .take()
            .unwrap_or((block.line + 1 + k, String::new()));
        text.push_str(line.trim_end_matches('\\'));
        if line.ends_with('\\') {
            text.push(' ');
            pending = Some((start, text));
        } else {
            out.push((start, text));
        }
    }
    out
}

#[test]
fn the_sagat_commands_in_the_docs_are_commands_the_tool_parses() {
    let mut checked = 0;
    for block in all_blocks() {
        if !is_shell(&block) {
            continue;
        }
        for (line, text) in commands(&block) {
            // The line itself, and any command substituted into it (`export SAGAT_TOKEN=$(sagat dev token)`).
            let mut candidates = vec![words(text.trim_start_matches("$ ").trim())];
            let mut rest = text.as_str();
            while let Some(at) = rest.find("$(") {
                let after = &rest[at + 2..];
                let end = after.find(')').unwrap_or(after.len());
                candidates.push(words(&after[..end]));
                rest = &after[end..];
            }
            for words in candidates {
                if words.first().map(String::as_str) != Some("sagat") {
                    continue;
                }
                // A placeholder (<run>) is a thing the reader fills in, not a value to parse.
                if words.iter().any(|w| w.starts_with('<') && w.ends_with('>')) {
                    continue;
                }
                let parsed = sagat_cli::args::parse(&words[1..]);
                assert!(
                    parsed.is_ok(),
                    "{}:{line}: `{}` is not a command: {:?}",
                    block
                        .file
                        .strip_prefix(repo())
                        .unwrap_or(&block.file)
                        .display(),
                    words.join(" "),
                    parsed.err().map(|e| e.0)
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 30,
        "only {checked} commands are checked: the pages lost them?"
    );
}

#[test]
fn the_scripts_a_readme_runs_are_there_and_executable() {
    let mut checked = 0;
    for block in all_blocks() {
        let dir = block.file.parent().expect("dir");
        let readme = block.file.file_name().is_some_and(|n| n == "README.md");
        let inside = block.file.strip_prefix(repo().join("examples")).is_ok();
        if !(readme && inside) || !is_shell(&block) {
            continue;
        }
        for (line, text) in commands(&block) {
            let words = words(&text);
            let Some(first) = words.first().filter(|w| w.starts_with("./")) else {
                continue;
            };
            let script = dir.join(first);
            let meta = std::fs::metadata(&script)
                .unwrap_or_else(|_| panic!("{}:{line}: {first} does not exist", shown(&block)));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert!(
                    meta.permissions().mode() & 0o111 != 0,
                    "{first} is not executable"
                );
            }
            let _ = meta;
            checked += 1;
        }
    }
    assert!(checked >= 10, "only {checked} scripts are checked");
}

#[test]
fn the_links_in_the_docs_lead_to_files_that_exist() {
    let mut checked = 0;
    for file in files() {
        let text = std::fs::read_to_string(&file).expect("read");
        let mut in_fence = false;
        for (n, line) in text.lines().enumerate() {
            if line.starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            let mut rest = line;
            while let Some(at) = rest.find("](") {
                let after = &rest[at + 2..];
                let Some(end) = after.find(')') else { break };
                let target = &after[..end];
                rest = &after[end..];
                if target.starts_with("http")
                    || target.starts_with('#')
                    || target.starts_with("mailto:")
                {
                    continue;
                }
                let path = target.split('#').next().unwrap_or_default();
                let resolved = file.parent().expect("dir").join(path);
                assert!(
                    resolved.exists(),
                    "{}:{}: the link ({target}) leads nowhere",
                    file.strip_prefix(repo()).unwrap_or(&file).display(),
                    n + 1
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 40, "only {checked} links are checked");
}

/// Every file of the examples, the templates and the quickstarts, for what a secret looks like.
fn sources_of_examples() -> Vec<PathBuf> {
    let root = repo();
    let mut found = Vec::new();
    let mut stack = vec![
        root.join("examples"),
        root.join("templates"),
        root.join("docs/quickstarts"),
        root.join("docs/reference"),
    ];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if matches!(
                name.as_str(),
                "node_modules" | "dist" | ".venv" | "bin" | "obj" | "target" | "__pycache__"
            ) {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            // Never follow a link while walking material that can be copied into a checkout.
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                stack.push(path);
            } else if entry.metadata().is_ok_and(|m| m.len() < 512 * 1024) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn no_example_template_or_page_carries_a_secret() {
    // A bearer token of the JWT shape, a private key, a cloud access key, a password in a URL.
    let looks_like = [
        (
            "a JWT",
            regex::Regex::new(r"eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}")
                .expect("regex"),
        ),
        (
            "a private key",
            regex::Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----").expect("regex"),
        ),
        (
            "an access key id",
            regex::Regex::new(r"\b(AKIA|ASIA)[0-9A-Z]{16}\b").expect("regex"),
        ),
        (
            "a password in a URL",
            regex::Regex::new(r"://[^/\s:@]+:[^/\s:@]+@[^/\s]+").expect("regex"),
        ),
        (
            "a literal bearer token",
            regex::Regex::new(r"[Bb]earer [A-Za-z0-9_-]{24,}").expect("regex"),
        ),
    ];
    let mut scanned = 0;
    for path in sources_of_examples() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue; // not text
        };
        scanned += 1;
        for (what, pattern) in &looks_like {
            assert!(
                !pattern.is_match(&text),
                "{} looks like it holds {what}",
                path.strip_prefix(repo()).unwrap_or(&path).display()
            );
        }
    }
    assert!(scanned >= 70, "only {scanned} files were scanned");
}
