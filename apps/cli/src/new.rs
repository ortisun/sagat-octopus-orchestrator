//! `sagat new <template>`: writes a workflow, the answers to serve it with `worker dev` and a short
//! README, from a template that is part of the binary.
//!
//! The templates live in `templates/` at the root of the repository, are embedded when the tool is
//! built, and are tested: every one validates with the platform's codec, and the ones that can run
//! are run against a platform in the suites of the feature.

use std::path::{Path, PathBuf};

use crate::{EXIT_DONE, Env, Io, resolve};

/// One file of a template.
pub struct File {
    pub name: &'static str,
    pub text: &'static str,
}

/// A template: a name, what it is for and its files.
pub struct Template {
    pub name: &'static str,
    pub about: &'static str,
    pub files: &'static [File],
}

macro_rules! file {
    ($dir:literal, $name:literal) => {
        File {
            name: $name,
            text: include_str!(concat!("../../../templates/", $dir, "/", $name)),
        }
    };
}

/// Every template, in the order `new --list` shows them.
pub const TEMPLATES: &[Template] = &[
    Template {
        name: "hello-world",
        about: "two steps, run by hand: the first workflow",
        files: &[
            file!("hello-world", "workflow.json"),
            file!("hello-world", "mocks.yaml"),
            file!("hello-world", "README.md"),
        ],
    },
    Template {
        name: "event-trigger",
        about: "a run for every event of a type",
        files: &[
            file!("event-trigger", "workflow.json"),
            file!("event-trigger", "mocks.yaml"),
            file!("event-trigger", "README.md"),
        ],
    },
    Template {
        name: "cron",
        about: "a run on a schedule, in a time zone",
        files: &[
            file!("cron", "workflow.json"),
            file!("cron", "mocks.yaml"),
            file!("cron", "README.md"),
        ],
    },
    Template {
        name: "approval",
        about: "wait for a person's decision (an event), with a deadline",
        files: &[
            file!("approval", "workflow.json"),
            file!("approval", "mocks.yaml"),
            file!("approval", "README.md"),
        ],
    },
    Template {
        name: "compensation",
        about: "a saga: each step names its undo, one names how to find out whether it happened",
        files: &[
            file!("compensation", "workflow.json"),
            file!("compensation", "mocks.yaml"),
            file!("compensation", "README.md"),
        ],
    },
    Template {
        name: "inline-script",
        about: "a script that runs in the platform's sandbox",
        files: &[
            file!("inline-script", "workflow.json"),
            file!("inline-script", "enrich.ts"),
            file!("inline-script", "mocks.yaml"),
            file!("inline-script", "README.md"),
        ],
    },
];

/// The template called `name`.
#[must_use]
pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

/// Writes a file that does not exist; with `force`, one that is a plain file (a link is never
/// followed, forced or not).
fn write(path: &Path, text: &str, force: bool) -> Result<(), String> {
    use std::io::Write as _;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
            return Err(format!(
                "{} is not a plain file: not written over",
                path.display()
            ));
        }
        Ok(_) if !force => {
            return Err(format!(
                "{} exists (--force writes over it)",
                path.display()
            ));
        }
        Ok(_) => {
            std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Err(_) => {}
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub(crate) fn run(
    env: &Env,
    template: Option<&str>,
    dir: Option<&Path>,
    force: bool,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let Some(name) = template else {
        io.say("templates:");
        for t in TEMPLATES {
            io.say(format!("  {:<14} {}", t.name, t.about));
        }
        io.say("write one with: sagat new <template> [dir]");
        return Ok(EXIT_DONE);
    };
    let Some(template) = find(name) else {
        let names: Vec<&str> = TEMPLATES.iter().map(|t| t.name).collect();
        return Err(format!(
            "'{name}' is not a template (they are: {})",
            names.join(", ")
        ));
    };
    let target: PathBuf = resolve(env, dir.unwrap_or_else(|| Path::new(template.name)));
    std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    // Nothing is written if something would be overwritten without --force.
    if !force {
        for file in template.files {
            let path = target.join(file.name);
            if std::fs::symlink_metadata(&path).is_ok() {
                return Err(format!(
                    "{} exists (--force writes over it)",
                    path.display()
                ));
            }
        }
    }
    for file in template.files {
        write(&target.join(file.name), file.text, force)?;
    }
    io.say(format!(
        "wrote the {} template to {}:",
        template.name,
        target.display()
    ));
    for file in template.files {
        io.say(format!("  {}", file.name));
    }
    io.say(format!("next: read {}/README.md", target.display()));
    Ok(EXIT_DONE)
}
